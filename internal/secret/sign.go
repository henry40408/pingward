package secret

import (
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"strings"
	"unicode"
)

// Each HMAC is domain-separated by a prefix. Without it the CSRF token would
// equal the session cookie's signature and every form would print it.
const (
	sessionDomain = "session:"
	csrfDomain    = "csrf:"
	flashDomain   = "flash:"
	setupDomain   = "setup:"
)

const sigSeparator = "."

func mac(secret []byte, domain, message string) []byte {
	h := hmac.New(sha256.New, secret)
	h.Write([]byte(domain))
	h.Write([]byte(message))
	return h.Sum(nil)
}

func sign(secret []byte, domain, value string) string {
	return value + sigSeparator + hex.EncodeToString(mac(secret, domain, value))
}

// verify splits on the last separator and returns the signed value.
func verify(secret []byte, domain, signed string) (string, bool) {
	i := strings.LastIndex(signed, sigSeparator)
	if i < 0 {
		return "", false
	}
	value, sigHex := signed[:i], signed[i+1:]
	sig, err := hex.DecodeString(sigHex)
	if err != nil {
		return "", false
	}
	if !hmac.Equal(sig, mac(secret, domain, value)) {
		return "", false
	}
	return value, true
}

// SignSession returns the session cookie value, "<id>.<hmac>".
func SignSession(secret []byte, sessionID string) string {
	return sign(secret, sessionDomain, sessionID)
}

// VerifySession recovers the session id from a cookie value.
func VerifySession(secret []byte, cookieValue string) (string, bool) {
	return verify(secret, sessionDomain, cookieValue)
}

// SignFlash signs the one-shot flash cookie so a sibling subdomain cannot
// plant a message the server never sent.
func SignFlash(secret []byte, value string) string {
	return sign(secret, flashDomain, value)
}

func VerifyFlash(secret []byte, cookieValue string) (string, bool) {
	return verify(secret, flashDomain, cookieValue)
}

// DeriveCSRF returns the session's CSRF token (hex HMAC of the session id).
func DeriveCSRF(secret []byte, sessionID string) string {
	return hex.EncodeToString(mac(secret, csrfDomain, sessionID))
}

// VerifyCSRF checks a submitted CSRF token in constant time.
func VerifyCSRF(secret []byte, sessionID, submitted string) bool {
	got, err := hex.DecodeString(submitted)
	if err != nil {
		return false
	}
	return hmac.Equal(got, mac(secret, csrfDomain, sessionID))
}

// Crockford base32 without I, L, O, U: hard to mistype when read off a log.
const setupTokenAlphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"

// NewSetupToken returns a first-run setup token, "XXXXX-XXXXX" (50 bits).
func NewSetupToken() string {
	var raw [10]byte
	if _, err := rand.Read(raw[:]); err != nil {
		panic("secret: OS random source failed: " + err.Error())
	}
	chars := make([]byte, len(raw))
	for i, b := range raw {
		// 256 is a multiple of 32, so masking keeps every character equally likely.
		chars[i] = setupTokenAlphabet[b&31]
	}
	return string(chars[:5]) + "-" + string(chars[5:])
}

// normalizeSetupToken drops dashes and whitespace and uppercases; the dash is
// only for readability.
func normalizeSetupToken(token string) string {
	var b strings.Builder
	for _, r := range token {
		if r == '-' || unicode.IsSpace(r) {
			continue
		}
		b.WriteString(strings.ToUpper(string(r)))
	}
	return b.String()
}

// VerifySetupToken compares both sides as MACs, so the comparison runs over
// equal-length digests and leaks neither length nor a matching prefix.
func VerifySetupToken(secret []byte, expected, submitted string) bool {
	want := mac(secret, setupDomain, normalizeSetupToken(expected))
	got := mac(secret, setupDomain, normalizeSetupToken(submitted))
	return hmac.Equal(got, want)
}
