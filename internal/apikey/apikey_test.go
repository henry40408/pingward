package apikey

import (
	"regexp"
	"strings"
	"testing"
)

func TestGeneratedKeyHasExpectedShape(t *testing.T) {
	k := Generate()
	if !regexp.MustCompile(`^pw_[0-9a-f]{64}$`).MatchString(k.Full) {
		t.Fatalf("bad key %q", k.Full)
	}
	if len(k.Prefix) != len(Prefix)+displayBodyChars || !strings.HasPrefix(k.Full, k.Prefix) {
		t.Fatalf("bad prefix %q", k.Prefix)
	}
	if len(k.Hash) != 64 || k.Hash != Hash(k.Full) {
		t.Fatalf("bad hash %q", k.Hash)
	}
}

func TestDistinctKeysAndHashes(t *testing.T) {
	a, b := Generate(), Generate()
	if a.Full == b.Full || a.Hash == b.Hash {
		t.Fatal("keys must differ")
	}
}

func TestHashIsSHA256Hex(t *testing.T) {
	const want = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
	if got := Hash("abc"); got != want {
		t.Fatalf("got %s", got)
	}
}
