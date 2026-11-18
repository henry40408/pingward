// Package auth holds password hashing, account-name validation and session
// lifetime rules.
package auth

import (
	"crypto/rand"
	"crypto/subtle"
	"encoding/base64"
	"fmt"
	"strconv"
	"strings"
	"sync"
	"unicode/utf8"

	"golang.org/x/crypto/argon2"

	"github.com/henry40408/pingward/internal/ids"
)

// Argon2id parameters for new hashes: the OWASP minimum profile, equal to the
// Rust argon2 crate defaults, so hashes stay interchangeable between builds.
const (
	argonMemoryKiB = 19456
	argonTime      = 2
	argonThreads   = 1
	argonKeyLen    = 32
	argonSaltLen   = 16
)

// MinPasswordChars and MaxPasswordChars count characters, not bytes. NIST
// SP800-63B wants 15 without MFA; pingward has no second factor.
const (
	MinPasswordChars = 15
	MaxPasswordChars = 128
)

// ValidatePassword returns the user-facing refusal, or "" when acceptable.
// There are no composition rules.
func ValidatePassword(plain string) string {
	n := utf8.RuneCountInString(plain)
	if n < MinPasswordChars {
		return fmt.Sprintf("Password must be at least %d characters.", MinPasswordChars)
	}
	if n > MaxPasswordChars {
		return fmt.Sprintf("Password must be at most %d characters.", MaxPasswordChars)
	}
	return ""
}

var b64 = base64.RawStdEncoding

// HashPassword returns a PHC string ($argon2id$v=19$m=...,t=...,p=...$salt$hash).
func HashPassword(plain string) (string, error) {
	salt := make([]byte, argonSaltLen)
	if _, err := rand.Read(salt); err != nil {
		return "", fmt.Errorf("read random salt: %w", err)
	}
	key := argon2.IDKey([]byte(plain), salt, argonTime, argonMemoryKiB, argonThreads, argonKeyLen)
	return fmt.Sprintf("$argon2id$v=%d$m=%d,t=%d,p=%d$%s$%s",
		argon2.Version, argonMemoryKiB, argonTime, argonThreads,
		b64.EncodeToString(salt), b64.EncodeToString(key)), nil
}

type phcParams struct {
	memory  uint32
	time    uint32
	threads uint8
	salt    []byte
	hash    []byte
}

// parsePHC accepts only argon2id version 19. The cost parameters come from the
// string, so hashes made with other settings still verify. A malformed string
// is an error, never a panic.
func parsePHC(phc string) (phcParams, bool) {
	parts := strings.Split(phc, "$")
	if len(parts) != 6 || parts[0] != "" || parts[1] != "argon2id" {
		return phcParams{}, false
	}
	if parts[2] != "v="+strconv.Itoa(argon2.Version) {
		return phcParams{}, false
	}
	var p phcParams
	if !parseCost(parts[3], &p) {
		return phcParams{}, false
	}
	var err error
	if p.salt, err = b64.DecodeString(parts[4]); err != nil || len(p.salt) == 0 {
		return phcParams{}, false
	}
	if p.hash, err = b64.DecodeString(parts[5]); err != nil || len(p.hash) == 0 {
		return phcParams{}, false
	}
	return p, true
}

func parseCost(s string, p *phcParams) bool {
	fields := strings.Split(s, ",")
	if len(fields) != 3 {
		return false
	}
	var values [3]uint64
	for i, key := range []string{"m=", "t=", "p="} {
		rest, ok := strings.CutPrefix(fields[i], key)
		if !ok {
			return false
		}
		v, err := strconv.ParseUint(rest, 10, 32)
		if err != nil || v == 0 {
			return false
		}
		values[i] = v
	}
	if values[2] > 255 {
		return false
	}
	p.memory, p.time, p.threads = uint32(values[0]), uint32(values[1]), uint8(values[2])
	return true
}

// VerifyPassword reports whether plain matches the PHC string. A malformed
// stored hash is a non-match.
func VerifyPassword(plain, phc string) bool {
	p, ok := parsePHC(phc)
	if !ok {
		return false
	}
	key := argon2.IDKey([]byte(plain), p.salt, p.time, p.memory, p.threads, uint32(len(p.hash)))
	return subtle.ConstantTimeCompare(key, p.hash) == 1
}

var (
	dummyOnce sync.Once
	dummyPHC  string
)

// dummyPasswordHash is a hash of a random secret, built once per process, for
// VerifyPasswordOrDummy to verify against.
func dummyPasswordHash() string {
	dummyOnce.Do(func() {
		hash, err := HashPassword(ids.NewV4())
		if err != nil {
			panic("auth: hashing a fixed-length uuid failed: " + err.Error())
		}
		dummyPHC = hash
	})
	return dummyPHC
}

// VerifyPasswordOrDummy spends one argon2 verification even when stored is nil
// (no such user, or a passwordless forward-auth account), so response time
// does not reveal which usernames exist.
func VerifyPasswordOrDummy(plain string, stored *string) bool {
	if stored != nil {
		return VerifyPassword(plain, *stored)
	}
	VerifyPassword(plain, dummyPasswordHash())
	return false
}
