package secret

import (
	"bytes"
	"testing"
)

func TestResolveUsesALongEnoughValueAsRawBytes(t *testing.T) {
	got, source := Resolve("0123456789abcdef")
	if source != SourceEnv || string(got) != "0123456789abcdef" {
		t.Fatalf("source %d, secret %q", source, got)
	}
}

func TestResolveCountsBytesNotCharacters(t *testing.T) {
	// 6 characters, 18 bytes: long enough, as in Rust's str::len.
	if _, source := Resolve("日本語日本語"); source != SourceEnv {
		t.Fatalf("source %d", source)
	}
}

func TestResolveRejectsAShortValue(t *testing.T) {
	got, source := Resolve("short")
	if source != SourceRejected || len(got) != generatedLen {
		t.Fatalf("source %d, len %d", source, len(got))
	}
}

func TestResolveGeneratesWhenUnset(t *testing.T) {
	first, source := Resolve("")
	second, _ := Resolve("")
	if source != SourceGenerated || len(first) != generatedLen || bytes.Equal(first, second) {
		t.Fatalf("source %d, equal %v", source, bytes.Equal(first, second))
	}
}
