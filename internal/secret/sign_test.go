package secret

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"strings"
	"testing"
)

var testSecret = []byte("test-secret-at-least-16-bytes")

const testID = "0b3c9a1e-4f2d-4a7b-9c8e-1d2f3a4b5c6d"

const otherSecret = "another-secret-16-plus"

func TestSetupTokenMatchesOnlyTheSameToken(t *testing.T) {
	if !VerifySetupToken(testSecret, "ABCDE-12345", "ABCDE-12345") {
		t.Fatal("same token must match")
	}
	for _, bad := range []string{"ABCDE-12346", "ABCDE-123456", ""} {
		if VerifySetupToken(testSecret, "ABCDE-12345", bad) {
			t.Fatalf("%q must not match", bad)
		}
	}
}

func TestSetupTokenIgnoresDashCaseAndWhitespace(t *testing.T) {
	for _, in := range []string{" abcde12345\n", "abcde - 12345"} {
		if !VerifySetupToken(testSecret, "ABCDE-12345", in) {
			t.Fatalf("%q must match", in)
		}
	}
}

func TestGeneratedSetupTokenShape(t *testing.T) {
	token := NewSetupToken()
	a, b, ok := strings.Cut(token, "-")
	if !ok {
		t.Fatalf("no dash in %q", token)
	}
	for _, group := range []string{a, b} {
		if len(group) != 5 {
			t.Fatalf("bad group in %q", token)
		}
		for _, c := range group {
			if !strings.ContainsRune(setupTokenAlphabet, c) {
				t.Fatalf("bad char %q in %q", c, token)
			}
		}
	}
	if token == NewSetupToken() {
		t.Fatal("tokens must differ")
	}
}

func TestSignedCookieRoundTrips(t *testing.T) {
	cookie := SignSession(testSecret, testID)
	if !strings.HasPrefix(cookie, testID) {
		t.Fatal("cookie must start with the id")
	}
	if id, ok := VerifySession(testSecret, cookie); !ok || id != testID {
		t.Fatalf("got %q %v", id, ok)
	}
}

func TestSessionRejectsBadInput(t *testing.T) {
	cookie := SignSession(testSecret, testID)
	sig := cookie[strings.LastIndex(cookie, ".")+1:]
	last := "0"
	if strings.HasSuffix(cookie, "0") {
		last = "1"
	}
	cases := map[string]string{
		"unsigned":         testID,
		"tampered sig":     cookie[:len(cookie)-1] + last,
		"tampered id":      "other-id." + sig,
		"empty":            "",
		"dot only":         ".",
		"empty sig":        "id.",
		"non-hex sig":      "id.zz",
		"odd-length sig":   "id.abc",
		"flash cross-over": SignFlash(testSecret, testID),
	}
	for name, bad := range cases {
		if _, ok := VerifySession(testSecret, bad); ok {
			t.Errorf("%s: must be rejected", name)
		}
	}
	if _, ok := VerifySession([]byte(otherSecret), cookie); ok {
		t.Error("a different secret must invalidate the cookie")
	}
}

func TestSessionAcceptsUppercaseHexSignature(t *testing.T) {
	cookie := SignSession(testSecret, testID)
	i := strings.LastIndex(cookie, ".")
	upper := cookie[:i+1] + strings.ToUpper(cookie[i+1:])
	if _, ok := VerifySession(testSecret, upper); !ok {
		t.Fatal("hex decoding accepts either case")
	}
}

func TestCSRFTokenVerifiesAndIsSessionScoped(t *testing.T) {
	token := DeriveCSRF(testSecret, testID)
	if !VerifyCSRF(testSecret, testID, token) {
		t.Fatal("own token must verify")
	}
	if VerifyCSRF(testSecret, "some-other-session", token) {
		t.Fatal("token must be session scoped")
	}
	if VerifyCSRF([]byte(otherSecret), testID, token) {
		t.Fatal("token must be secret scoped")
	}
	if VerifyCSRF(testSecret, testID, "not-hex") {
		t.Fatal("non-hex must fail")
	}
}

func TestCSRFTokenDiffersFromSessionSignature(t *testing.T) {
	cookie := SignSession(testSecret, testID)
	sig := cookie[strings.LastIndex(cookie, ".")+1:]
	if sig == DeriveCSRF(testSecret, testID) {
		t.Fatal("domain separation broken")
	}
}

func TestFlashRoundTripsAndRejectsForgeries(t *testing.T) {
	signed := SignFlash(testSecret, "settings")
	if !strings.HasPrefix(signed, "settings.") {
		t.Fatal("flash must start with the value")
	}
	if v, ok := VerifyFlash(testSecret, signed); !ok || v != "settings" {
		t.Fatalf("got %q %v", v, ok)
	}
	for _, bad := range []string{"settings", "password_reset_keys:1:9"} {
		if _, ok := VerifyFlash(testSecret, bad); ok {
			t.Fatalf("%q must be rejected", bad)
		}
	}
	forged := SignFlash(testSecret, "password_reset_keys:1:2")
	sig := forged[strings.LastIndex(forged, ".")+1:]
	if _, ok := VerifyFlash(testSecret, "password_reset_keys:1:9."+sig); ok {
		t.Fatal("tampered flash must be rejected")
	}
	if _, ok := VerifyFlash([]byte(otherSecret), forged); ok {
		t.Fatal("different secret must be rejected")
	}
}

func TestFlashAndSessionSignaturesDoNotCrossVerify(t *testing.T) {
	if _, ok := VerifyFlash(testSecret, SignSession(testSecret, testID)); ok {
		t.Fatal("session cookie accepted as flash")
	}
}

func TestVerifiesRustCapturedVectors(t *testing.T) {
	raw, err := os.ReadFile("testdata/vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var v struct {
		SecretUTF8 string `json:"secret_utf8"`
		Sessions   []struct {
			SessionCookie string `json:"session_cookie"`
			CSRF          string `json:"csrf"`
		} `json:"sessions"`
	}
	if err := json.Unmarshal(raw, &v); err != nil {
		t.Fatal(err)
	}
	if len(v.Sessions) == 0 {
		t.Fatal("no vectors")
	}
	secret := []byte(v.SecretUTF8)
	for _, s := range v.Sessions {
		id, ok := VerifySession(secret, s.SessionCookie)
		if !ok {
			t.Fatalf("Rust cookie %q rejected", s.SessionCookie)
		}
		if got := DeriveCSRF(secret, id); got != s.CSRF {
			t.Fatalf("csrf for %s: got %s want %s", id, got, s.CSRF)
		}
		if SignSession(secret, id) != s.SessionCookie {
			t.Fatalf("re-signing %s differs from Rust", id)
		}
	}
}

func TestHexRoundTripAllBytes(t *testing.T) {
	all := make([]byte, 256)
	for i := range all {
		all[i] = byte(i)
	}
	enc := hex.EncodeToString(all)
	if len(enc) != 512 || !strings.HasPrefix(enc, "000102") || !strings.HasSuffix(enc, "fdfeff") {
		t.Fatal("unexpected hex")
	}
}
