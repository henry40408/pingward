package auth

import (
	"fmt"
	"strings"
	"unicode"
	"unicode/utf8"
)

// MaxUsernameChars is the longest username in characters (not bytes).
const MaxUsernameChars = 64

// logUsernameMaxChars is the longest username kept in a log line.
const logUsernameMaxChars = 64

// ValidateUsername is the single username rule for every surface that creates
// an account: /setup, the admin "new user" form and forward-auth provisioning.
// It returns the trimmed name to store, or the user-facing refusal (empty when valid). Case and
// script are free: the UNIQUE constraint compares exact bytes.
func ValidateUsername(raw string) (username, refusal string) {
	username = strings.TrimFunc(raw, isRustWhitespace)
	if username == "" {
		return "", "username and password are required"
	}
	if utf8.RuneCountInString(username) > MaxUsernameChars {
		return "", fmt.Sprintf("Username must be at most %d characters.", MaxUsernameChars)
	}
	if strings.ContainsFunc(username, isControl) {
		return "", "Username must not contain control characters."
	}
	return username, ""
}

// isControl matches Rust char::is_control (Unicode category Cc).
func isControl(r rune) bool { return unicode.Is(unicode.Cc, r) }

// isRustWhitespace matches Rust char::is_whitespace (Unicode White_Space).
func isRustWhitespace(r rune) bool { return unicode.Is(unicode.White_Space, r) }

// LogUsername truncates an attacker-chosen username for logging. Callers must
// render it quoted (%q or slog) so an embedded newline cannot forge a log line.
func LogUsername(raw string) string {
	if utf8.RuneCountInString(raw) <= logUsernameMaxChars {
		return raw
	}
	runes := []rune(raw)
	return string(runes[:logUsernameMaxChars]) + "…"
}
