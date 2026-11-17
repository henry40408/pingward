package ids

import (
	"regexp"
	"testing"
)

func TestNewV4Shape(t *testing.T) {
	re := regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$`)
	a, b := NewV4(), NewV4()
	if !re.MatchString(a) {
		t.Fatalf("bad uuid %q", a)
	}
	if a == b {
		t.Fatal("uuids must differ")
	}
}

func TestNewV4SimpleShape(t *testing.T) {
	re := regexp.MustCompile(`^[0-9a-f]{12}4[0-9a-f]{3}[89ab][0-9a-f]{15}$`)
	if s := NewV4Simple(); !re.MatchString(s) {
		t.Fatalf("bad simple uuid %q", s)
	}
}
