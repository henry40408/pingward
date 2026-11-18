// Package apikey generates and hashes REST API keys.
//
// A key is "pw_" + 64 hex characters (two v4 UUIDs), shown once. Only its
// SHA-256 is stored, unsalted so it can be an indexed lookup: safe for a
// high-entropy secret, unlike a password.
package apikey

import (
	"crypto/sha256"
	"encoding/hex"

	"github.com/henry40408/pingward/internal/ids"
)

const Prefix = "pw_"

// displayBodyChars is how many body characters the non-secret, displayable
// prefix keeps.
const displayBodyChars = 8

// Key is a freshly generated API key. Full is never stored.
type Key struct {
	Full   string
	Prefix string
	Hash   string
}

func Generate() Key {
	body := ids.NewV4Simple() + ids.NewV4Simple()
	full := Prefix + body
	return Key{
		Full:   full,
		Prefix: Prefix + body[:displayBodyChars],
		Hash:   Hash(full),
	}
}

// Hash returns SHA-256 as lowercase hex. It also derives session handles.
func Hash(token string) string {
	sum := sha256.Sum256([]byte(token))
	return hex.EncodeToString(sum[:])
}
