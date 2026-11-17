// Package secret holds the process secret behind session cookies and CSRF
// tokens.
package secret

import "crypto/rand"

// MinLen is the shortest PINGWARD_SECRET accepted, in bytes.
const MinLen = 16

const generatedLen = 32

// Source says where the process secret came from, for the one-time startup
// warning.
type Source int

const (
	// SourceEnv is a PINGWARD_SECRET long enough to use.
	SourceEnv Source = iota
	// SourceGenerated means the variable was unset or blank: a random secret
	// serves this process only.
	SourceGenerated
	// SourceRejected means the variable was shorter than MinLen: ignored, and a
	// random secret generated.
	SourceRejected
)

// Resolve returns the process secret. The raw value's bytes are used as-is
// (no decoding); a blank raw means the variable was unset.
func Resolve(raw string) ([]byte, Source) {
	if raw == "" {
		return generate(), SourceGenerated
	}
	if len(raw) < MinLen {
		return generate(), SourceRejected
	}
	return []byte(raw), SourceEnv
}

func generate() []byte {
	buf := make([]byte, generatedLen)
	if _, err := rand.Read(buf); err != nil {
		panic("secret: no randomness available: " + err.Error())
	}
	return buf
}
