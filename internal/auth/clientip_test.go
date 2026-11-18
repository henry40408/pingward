package auth

import (
	"net/http"
	"net/netip"
	"testing"

	"github.com/henry40408/pingward/internal/config"
)

func addr(t *testing.T, text string) netip.Addr {
	t.Helper()
	return netip.MustParseAddr(text)
}

func forwardAuthConfig(header string, trusted ...string) *config.Config {
	cfg := &config.Config{TrustedProxies: trusted}
	if header != "" {
		cfg.ForwardAuthHeader = &header
	}
	return cfg
}

func headersWith(pairs ...string) http.Header {
	h := http.Header{}
	for i := 0; i < len(pairs); i += 2 {
		h.Add(pairs[i], pairs[i+1])
	}
	return h
}

func TestForwardAuthHonouredOnlyFromTrustedProxy(t *testing.T) {
	cfg := forwardAuthConfig("X-Forwarded-User", "10.0.0.1")
	h := headersWith("X-Forwarded-User", "alice")
	if got, ok := ForwardAuthUsername(h, addr(t, "10.0.0.1"), cfg); !ok || got != "alice" {
		t.Fatalf("trusted peer: %q %v", got, ok)
	}
	if _, ok := ForwardAuthUsername(h, addr(t, "8.8.8.8"), cfg); ok {
		t.Fatal("untrusted peer accepted")
	}
	if _, ok := ForwardAuthUsername(h, netip.Addr{}, cfg); ok {
		t.Fatal("no peer accepted")
	}
}

func TestForwardAuthDisabledWhenUnconfigured(t *testing.T) {
	h := headersWith("X-Forwarded-User", "alice")
	if _, ok := ForwardAuthUsername(h, addr(t, "10.0.0.1"), forwardAuthConfig("", "10.0.0.1")); ok {
		t.Fatal("unconfigured forward auth accepted")
	}
}

func TestForwardAuthRefusesBlankAndNonVisibleASCII(t *testing.T) {
	cfg := forwardAuthConfig("X-User", "10.0.0.1")
	peer := addr(t, "10.0.0.1")
	for _, value := range []string{"", "   ", "\t", "al\x00ice", "alicé"} {
		if got, ok := ForwardAuthUsername(headersWith("X-User", value), peer, cfg); ok {
			t.Errorf("%q accepted as %q", value, got)
		}
	}
	if got, ok := ForwardAuthUsername(headersWith("X-User", "  bob  "), peer, cfg); !ok || got != "bob" {
		t.Fatalf("trim: %q %v", got, ok)
	}
	if _, ok := ForwardAuthUsername(http.Header{}, peer, cfg); ok {
		t.Fatal("absent header accepted")
	}
}

func TestClientIPPrefersForwardedForOnlyFromATrustedProxy(t *testing.T) {
	trusted := []string{"10.0.0.1"}
	h := headersWith("X-Forwarded-For", "203.0.113.7")
	if got, ok := ClientIP(h, addr(t, "10.0.0.1"), trusted); !ok || got != "203.0.113.7" {
		t.Fatalf("trusted: %q %v", got, ok)
	}
	if got, _ := ClientIP(h, addr(t, "8.8.8.8"), trusted); got != "8.8.8.8" {
		t.Fatalf("untrusted peer must not spoof its address: %q", got)
	}
	if _, ok := ClientIP(h, netip.Addr{}, trusted); ok {
		t.Fatal("no peer must give no address")
	}
}

func TestClientIPTakesTheFirstEntryAndIgnoresJunk(t *testing.T) {
	trusted := []string{"10.0.0.1"}
	peer := addr(t, "10.0.0.1")
	cases := map[string]string{
		"203.0.113.7, 10.0.0.1": "203.0.113.7",
		"  203.0.113.7  ":       "203.0.113.7",
		"not-an-ip":             "10.0.0.1",
		"":                      "10.0.0.1",
		"fe80::1%eth0":          "10.0.0.1",
		"203.0.113.7, \xff":     "10.0.0.1",
		"::ffff:203.0.113.7":    "203.0.113.7",
	}
	for value, want := range cases {
		if got, _ := ClientIP(headersWith("X-Forwarded-For", value), peer, trusted); got != want {
			t.Errorf("%q: got %q, want %q", value, got, want)
		}
	}
	if got, _ := ClientIP(http.Header{}, peer, trusted); got != "10.0.0.1" {
		t.Fatalf("no header: %q", got)
	}
}

func TestClientIPUsesOnlyTheFirstHeaderLine(t *testing.T) {
	h := headersWith("X-Forwarded-For", "203.0.113.7", "X-Forwarded-For", "198.51.100.1")
	if got, _ := ClientIP(h, addr(t, "10.0.0.1"), []string{"10.0.0.1"}); got != "203.0.113.7" {
		t.Fatalf("got %q", got)
	}
}

func TestClientIPWithoutTrustedProxiesAlwaysUsesThePeer(t *testing.T) {
	h := headersWith("X-Forwarded-For", "203.0.113.7")
	if got, _ := ClientIP(h, addr(t, "10.0.0.1"), nil); got != "10.0.0.1" {
		t.Fatalf("got %q", got)
	}
}

func TestClientIPMatchesAV4MappedPeerAgainstAV4Pattern(t *testing.T) {
	trusted := []string{"172.18.0.0/16"}
	h := headersWith("X-Forwarded-For", "203.0.113.7")
	if got, _ := ClientIP(h, addr(t, "::ffff:172.18.0.5"), trusted); got != "203.0.113.7" {
		t.Fatalf("mapped trusted peer: %q", got)
	}
	if got, _ := ClientIP(h, addr(t, "::ffff:8.8.8.8"), trusted); got != "8.8.8.8" {
		t.Fatalf("mapped stranger must be stored in v4 form: %q", got)
	}
}

func TestTrustedProxyAcceptsACIDRBlock(t *testing.T) {
	nets := []string{"172.16.0.0/12"}
	for ip, want := range map[string]bool{
		"172.18.0.5": true, "172.31.255.255": true, "172.15.255.255": false, "172.32.0.1": false,
	} {
		if got := IsTrustedProxy(nets, addr(t, ip)); got != want {
			t.Errorf("%s: got %v", ip, got)
		}
	}
}

func TestTrustedProxyHandlesPrefixEdgesAndV6(t *testing.T) {
	check := func(patterns []string, ip string, want bool) {
		t.Helper()
		if got := IsTrustedProxy(patterns, addr(t, ip)); got != want {
			t.Errorf("%v / %s: got %v", patterns, ip, got)
		}
	}
	check([]string{"0.0.0.0/0"}, "8.8.8.8", true)
	check([]string{"10.0.0.1/32"}, "10.0.0.1", true)
	check([]string{"10.0.0.1/32"}, "10.0.0.2", false)
	v6 := []string{"fd00::/8", "::1/128"}
	check(v6, "fd00:1234::9", true)
	check(v6, "::1", true)
	check(v6, "2001:db8::1", false)
	check(v6, "10.0.0.1", false) // families never cross
}

func TestTrustedProxyRejectsUnparseablePatterns(t *testing.T) {
	for _, junk := range []string{"caddy", "10.0.0.1/", "10.0.0.1/33", "10.0.0.0/x", "", "10.0.0.1/-1", "10.0.0.1/++8", "fe80::1%eth0", "fe80::/10%eth0"} {
		if IsTrustedProxy([]string{junk}, addr(t, "10.0.0.1")) {
			t.Errorf("%q must not be trusted", junk)
		}
	}
	if IsTrustedProxy([]string{"fe80::1%eth0"}, addr(t, "fe80::1")) {
		t.Error("a zoned pattern must not be trusted")
	}
	if IsTrustedProxy([]string{"0.0.0.0/0"}, netip.Addr{}) {
		t.Error("the zero address must never be trusted")
	}
}

func TestTrustedProxyTrimsPatternsAndAcceptsALeadingPlusPrefix(t *testing.T) {
	if !IsTrustedProxy([]string{" 10.0.0.0 / 8 "}, addr(t, "10.1.2.3")) {
		t.Error("spaces around the pattern parts are trimmed")
	}
	if !IsTrustedProxy([]string{"10.0.0.0/+8"}, addr(t, "10.1.2.3")) {
		t.Error("Rust's u8 parse accepts a leading plus")
	}
}

func TestTrustedProxyMappedPatternNeverMatchesAV4Peer(t *testing.T) {
	if IsTrustedProxy([]string{"::ffff:10.0.0.0/104"}, addr(t, "10.1.2.3")) {
		t.Error("a mapped CIDR pattern is a v6 block in Rust")
	}
	if !IsTrustedProxy([]string{"::ffff:10.0.0.1"}, addr(t, "10.0.0.1")) {
		t.Error("a bare mapped pattern is canonicalised")
	}
}

func TestPeerAddrKeepsALinkLocalProxyTrustedDespiteItsZone(t *testing.T) {
	trusted := []string{"fe80::/10", "fe80::1"}
	for _, remote := range []string{"[fe80::1%eth0]:4000", "[fe80::1]:4000", "fe80::1%en0"} {
		peer := PeerAddr(remote)
		if !peer.IsValid() || peer.Zone() != "" {
			t.Fatalf("%q: zone survived: %v", remote, peer)
		}
		if !IsTrustedProxy(trusted, peer) {
			t.Errorf("%q: link-local proxy no longer trusted", remote)
		}
	}
	// Even a zoned Addr handed straight to the trust check keeps its verdict.
	if !IsTrustedProxy([]string{"fe80::1"}, netip.MustParseAddr("fe80::1%eth0")) {
		t.Error("zoned address must be judged without its zone")
	}
	// A zone must not leak into the recorded address either.
	if got, _ := ClientIP(http.Header{}, netip.MustParseAddr("fe80::1%eth0"), nil); got != "fe80::1" {
		t.Errorf("recorded %q", got)
	}
}

func TestPeerAddrParsesIPv4IPv6AndGarbage(t *testing.T) {
	if got := PeerAddr("203.0.113.9:1234"); got.String() != "203.0.113.9" {
		t.Fatalf("v4: %v", got)
	}
	if got := PeerAddr("[::ffff:10.0.0.1]:80"); got.String() != "10.0.0.1" {
		t.Fatalf("mapped: %v", got)
	}
	for _, junk := range []string{"", "@", "unix", "[bad]:1"} {
		if PeerAddr(junk).IsValid() {
			t.Errorf("%q gave a peer", junk)
		}
	}
}

func TestRateLimitKeyPrefersXFFOnlyFromATrustedProxy(t *testing.T) {
	trusted := []string{"10.0.0.1"}
	h := headersWith("X-Forwarded-For", "203.0.113.9, 10.0.0.1")
	if got := RateLimitKey(h, addr(t, "10.0.0.1"), trusted); got != addr(t, "10.0.0.1") {
		t.Fatalf("appended hop: %v", got)
	}
	stranger := addr(t, "8.8.8.8")
	if got := RateLimitKey(h, stranger, trusted); got != stranger {
		t.Fatalf("stranger: %v", got)
	}
}

func TestSpoofedLeadingXFFHopsDoNotChangeTheKey(t *testing.T) {
	trusted := []string{"10.0.0.1"}
	for _, spoof := range []string{"1.2.3.4", "5.6.7.8", "9.9.9.9, 8.8.8.8"} {
		h := headersWith("X-Forwarded-For", spoof+", 198.51.100.7")
		if got := RateLimitKey(h, addr(t, "10.0.0.1"), trusted); got != addr(t, "198.51.100.7") {
			t.Errorf("%q: %v", spoof, got)
		}
	}
}

func TestRateLimitKeyUsesTheLastXFFHeaderLine(t *testing.T) {
	h := headersWith("X-Forwarded-For", "9.9.9.9, 8.8.8.8", "X-Forwarded-For", "198.51.100.7")
	if got := RateLimitKey(h, addr(t, "10.0.0.1"), []string{"10.0.0.1"}); got != addr(t, "198.51.100.7") {
		t.Fatalf("got %v", got)
	}
}

func TestRateLimitKeyIgnoresGarbageXFFFromATrustedProxy(t *testing.T) {
	peer := addr(t, "10.0.0.1")
	for _, value := range []string{"not-an-ip", "", "1.2.3.4, ", "fe80::1%eth0", "1.2.3.4, \xff"} {
		h := headersWith("X-Forwarded-For", value)
		if got := RateLimitKey(h, peer, []string{"10.0.0.1"}); got != peer {
			t.Errorf("%q: %v", value, got)
		}
	}
	if got := RateLimitKey(http.Header{}, peer, []string{"10.0.0.1"}); got != peer {
		t.Fatalf("no header: %v", got)
	}
}

func TestRateLimitKeyFallsBackToLoopbackWithNoPeer(t *testing.T) {
	h := headersWith("X-Forwarded-For", "203.0.113.9")
	want := addr(t, "127.0.0.1")
	if RateLimitKey(h, netip.Addr{}, []string{"10.0.0.1"}) != want || RateLimitKey(http.Header{}, netip.Addr{}, nil) != want {
		t.Fatal("no peer must share the loopback bucket")
	}
}

func TestRateLimitKeyMatchesAV4MappedPeerAgainstAV4Pattern(t *testing.T) {
	h := headersWith("X-Forwarded-For", "203.0.113.9")
	if got := RateLimitKey(h, addr(t, "::ffff:10.0.0.1"), []string{"10.0.0.1"}); got != addr(t, "203.0.113.9") {
		t.Fatalf("got %v", got)
	}
}

func TestRateLimitKeyIgnoresHeadersFromAnUntrustedV6Peer(t *testing.T) {
	peer := addr(t, "::1")
	h := headersWith("X-Forwarded-For", "not-an-ip")
	if got := RateLimitKey(h, peer, []string{"10.0.0.1"}); got != peer {
		t.Fatalf("got %v", got)
	}
}
