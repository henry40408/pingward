package ping

import (
	"strings"
	"unicode/utf8"
)

// lossyUTF8 matches Rust's String::from_utf8_lossy: one U+FFFD per maximal
// invalid prefix, not per byte (strings.ToValidUTF8 would merge a whole run
// into one, utf8's own decoding replaces every byte).
func lossyUTF8(raw []byte) string {
	var out strings.Builder
	for len(raw) > 0 {
		r, size := utf8.DecodeRune(raw)
		if r != utf8.RuneError || size > 1 {
			out.Write(raw[:size])
			raw = raw[size:]
			continue
		}
		out.WriteRune(utf8.RuneError)
		raw = raw[invalidPrefixLen(raw):]
	}
	return out.String()
}

// invalidPrefixLen is how many bytes of raw (which starts with an invalid
// sequence) form one error: the lead byte plus every continuation byte that
// was still valid for it.
func invalidPrefixLen(raw []byte) int {
	lowest, highest := byte(0x80), byte(0xBF)
	var need int
	switch lead := raw[0]; {
	case lead >= 0xC2 && lead <= 0xDF:
		need = 2
	case lead == 0xE0:
		need, lowest = 3, 0xA0
	case lead == 0xED:
		need, highest = 3, 0x9F
	case lead >= 0xE1 && lead <= 0xEF:
		need = 3
	case lead == 0xF0:
		need, lowest = 4, 0x90
	case lead == 0xF4:
		need, highest = 4, 0x8F
	case lead >= 0xF1 && lead <= 0xF3:
		need = 4
	default:
		return 1
	}
	length := 1
	for length < need && length < len(raw) {
		next := raw[length]
		if length == 1 && (next < lowest || next > highest) {
			break
		}
		if length > 1 && (next < 0x80 || next > 0xBF) {
			break
		}
		length++
	}
	return length
}
