package auth

import (
	"net"
	"net/http"
	"net/netip"
	"strconv"
	"strings"

	"github.com/henry40408/pingward/internal/config"
)

// PeerAddr extracts the socket peer from http.Request.RemoteAddr. An
// unparseable address yields the zero Addr ("no peer"), which fails every
// trust check.
//
// The IPv6 zone ("fe80::1%eth0") is dropped here: Rust's SocketAddr carries no
// zone in the IP, and a zoned netip.Addr never equals or falls inside an
// unzoned pattern, so keeping it would silently stop trusting a link-local proxy.
func PeerAddr(remoteAddr string) netip.Addr {
	host, _, err := net.SplitHostPort(remoteAddr)
	if err != nil {
		host = remoteAddr
	}
	addr, err := netip.ParseAddr(host)
	if err != nil {
		return netip.Addr{}
	}
	return canonical(addr)
}

// canonical maps an IPv4-mapped IPv6 address to IPv4 and drops any zone.
func canonical(addr netip.Addr) netip.Addr {
	return addr.WithZone("").Unmap()
}

// parseHeaderIP parses an address taken from a header or pattern. Zones are
// refused, as Rust's IpAddr parser does: a zone is never legitimate there.
func parseHeaderIP(raw string) (netip.Addr, bool) {
	addr, err := netip.ParseAddr(raw)
	if err != nil || addr.Zone() != "" {
		return netip.Addr{}, false
	}
	return addr, true
}

// IsTrustedProxy reports whether ip matches a trusted-proxy pattern: a bare
// address or a CIDR block. It compares canonically, so an IPv4-mapped peer
// matches a v4 pattern. Unparseable patterns match nothing; DNS is never
// consulted, or a resolver would decide trust.
func IsTrustedProxy(patterns []string, ip netip.Addr) bool {
	if !ip.IsValid() {
		return false
	}
	ip = canonical(ip)
	for _, pattern := range patterns {
		if proxyPatternMatches(pattern, ip) {
			return true
		}
	}
	return false
}

func proxyPatternMatches(pattern string, ip netip.Addr) bool {
	pattern = strings.TrimFunc(pattern, isRustWhitespace)
	network, prefixText, hasPrefix := strings.Cut(pattern, "/")
	if !hasPrefix {
		addr, ok := parseHeaderIP(pattern)
		return ok && canonical(addr) == ip
	}
	addr, ok := parseHeaderIP(strings.TrimFunc(network, isRustWhitespace))
	if !ok {
		return false
	}
	bits, ok := parsePrefixLength(strings.TrimFunc(prefixText, isRustWhitespace))
	if !ok || addr.Is4() != ip.Is4() {
		return false
	}
	// Rust compares the raw octets of the pattern; an IPv4-mapped pattern
	// address is an IPv6 block there, which never matches a canonical v4 peer.
	prefix, err := addr.Prefix(bits)
	return err == nil && prefix.Contains(ip)
}

// parsePrefixLength mirrors Rust's u8 parse: digits with an optional leading
// "+", at most 255. Whether it fits the address family is checked by Prefix.
func parsePrefixLength(text string) (int, bool) {
	text = strings.TrimPrefix(text, "+")
	if text == "" || text[0] == '+' || text[0] == '-' {
		return 0, false
	}
	bits, err := strconv.ParseUint(text, 10, 8)
	return int(bits), err == nil
}

// isVisibleASCII matches what Rust's HeaderValue::to_str accepts.
func isVisibleASCII(value string) bool {
	for i := 0; i < len(value); i++ {
		c := value[i]
		if (c < 32 && c != '\t') || c > 126 {
			return false
		}
	}
	return true
}

// ForwardAuthUsername returns the forward-auth username iff forward-auth is
// configured, the header is present, visible ASCII and not blank, and peer is
// a configured trusted proxy. Only blankness is judged here: ValidateUsername
// gates new accounts, so an existing user is never locked out by a stricter rule.
func ForwardAuthUsername(headers http.Header, peer netip.Addr, cfg *config.Config) (string, bool) {
	if cfg.ForwardAuthHeader == nil || !peer.IsValid() {
		return "", false
	}
	if !IsTrustedProxy(cfg.TrustedProxies, peer) {
		return "", false
	}
	value := headers.Get(*cfg.ForwardAuthHeader)
	if !isVisibleASCII(value) {
		return "", false
	}
	username := strings.TrimFunc(value, isRustWhitespace)
	return username, username != ""
}

// ClientIP is the address to record against a session or ping: the leftmost
// entry of the first X-Forwarded-For header when the peer is a trusted proxy
// (anyone else could spoof it), otherwise, or if that entry is unparseable,
// the peer. ok is false without a peer.
//
// Do NOT merge with RateLimitKey: the leftmost entry is client-supplied, fine
// for attribution, unusable as a limiter key.
func ClientIP(headers http.Header, peer netip.Addr, trustedProxies []string) (ip string, ok bool) {
	if !peer.IsValid() {
		return "", false
	}
	peer = canonical(peer)
	if !IsTrustedProxy(trustedProxies, peer) {
		return peer.String(), true
	}
	first := headers.Get("X-Forwarded-For")
	if !isVisibleASCII(first) {
		return peer.String(), true
	}
	leftmost, _, _ := strings.Cut(first, ",")
	forwarded, parsed := parseHeaderIP(strings.TrimFunc(leftmost, isRustWhitespace))
	if !parsed {
		return peer.String(), true
	}
	return canonical(forwarded).String(), true
}

// RateLimitKey is the client address used as a limiter key. It takes the
// rightmost hop of the last X-Forwarded-For header line (a proxy may append a
// new line), trusted only when the peer is a trusted proxy; it assumes exactly
// one proxy. Without a peer it falls back to a shared loopback bucket.
func RateLimitKey(headers http.Header, peer netip.Addr, trustedProxies []string) netip.Addr {
	if !peer.IsValid() {
		return netip.AddrFrom4([4]byte{127, 0, 0, 1})
	}
	peer = canonical(peer)
	if !IsTrustedProxy(trustedProxies, peer) {
		return peer
	}
	lines := headers.Values("X-Forwarded-For")
	if len(lines) == 0 {
		return peer
	}
	last := lines[len(lines)-1]
	if !isVisibleASCII(last) {
		return peer
	}
	hop := last[strings.LastIndex(last, ",")+1:]
	forwarded, parsed := parseHeaderIP(strings.TrimFunc(hop, isRustWhitespace))
	if !parsed {
		return peer
	}
	return canonical(forwarded)
}
