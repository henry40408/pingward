package ping

import "testing"

func TestLossyUTF8MatchesRustFromUTF8Lossy(t *testing.T) {
	cases := []struct {
		name string
		in   []byte
		want string
	}{
		{"valid", []byte("héllo ✓ 😀"), "héllo ✓ 😀"},
		{"lone continuation bytes are one each", []byte{'a', 0x80, 0x80, 'b'}, "a��b"},
		{"truncated 3-byte sequence is one", []byte{'a', 0xE2, 0x82, 'b'}, "a�b"},
		{"truncated at the end is one", []byte{'a', 0xE2, 0x82}, "a�"},
		{"invalid second byte after E0", []byte{0xE0, 0x80, 0x80}, "���"},
		{"surrogate range ED A0", []byte{0xED, 0xA0, 0x80}, "���"},
		{"overlong C0", []byte{0xC0, 0xAF}, "��"},
		{"4-byte truncated after two", []byte{0xF0, 0x9F, 0x98, 'x'}, "�x"},
		{"F5 and above are single", []byte{0xF5, 0x80}, "��"},
		{"empty", nil, ""},
	}
	for _, c := range cases {
		if got := lossyUTF8(c.in); got != c.want {
			t.Errorf("%s: got %q, want %q", c.name, got, c.want)
		}
	}
}
