// Package duration reads and writes human durations at the form and
// environment boundary; storage stays in whole seconds.
package duration

import (
	"math"
	"strconv"
	"strings"
)

var unitSeconds = map[byte]int64{'s': 1, 'm': 60, 'h': 3600, 'd': 86400}

// Parse reads whole seconds from a bare integer or combined s/m/h/d
// components, ignoring whitespace and case ("1h30m", "1H 30M"). ok is false
// unless the whole string is consumed without overflow.
func Parse(s string) (seconds int64, ok bool) {
	cleaned := strings.ToLower(removeASCIIWhitespace(s))
	rest, negative := strings.CutPrefix(cleaned, "-")
	if rest == "" {
		return 0, false
	}
	magnitude, ok := parseMagnitude(rest)
	if !ok {
		return 0, false
	}
	if negative {
		return -magnitude, true
	}
	return magnitude, true
}

func parseMagnitude(rest string) (int64, bool) {
	if isAllDigits(rest) {
		v, err := strconv.ParseInt(rest, 10, 64)
		return v, err == nil
	}
	var total int64
	for rest != "" {
		digitCount := countLeadingDigits(rest)
		if digitCount == 0 || digitCount == len(rest) {
			return 0, false
		}
		multiplier, known := unitSeconds[rest[digitCount]]
		if !known {
			return 0, false
		}
		n, err := strconv.ParseInt(rest[:digitCount], 10, 64)
		if err != nil || n > math.MaxInt64/multiplier {
			return 0, false
		}
		component := n * multiplier
		if total > math.MaxInt64-component {
			return 0, false
		}
		total += component
		rest = rest[digitCount+1:]
	}
	return total, true
}

// Format is the lossless inverse of Parse (5430 becomes "1h30m30s");
// negative input is treated as zero.
func Format(seconds int64) string {
	s := max(seconds, 0)
	if s == 0 {
		return "0s"
	}
	var out strings.Builder
	parts := []struct {
		value  int64
		suffix string
	}{{s / 86400, "d"}, {(s % 86400) / 3600, "h"}, {(s % 3600) / 60, "m"}, {s % 60, "s"}}
	for _, p := range parts {
		if p.value > 0 {
			out.WriteString(strconv.FormatInt(p.value, 10))
			out.WriteString(p.suffix)
		}
	}
	return out.String()
}

// removeASCIIWhitespace drops the characters Rust's is_ascii_whitespace
// accepts: space, \t, \n, \f and \r (not \v).
func removeASCIIWhitespace(s string) string {
	return strings.Map(func(r rune) rune {
		switch r {
		case ' ', '\t', '\n', '\f', '\r':
			return -1
		}
		return r
	}, s)
}

func isAllDigits(s string) bool {
	return s != "" && countLeadingDigits(s) == len(s)
}

func countLeadingDigits(s string) int {
	n := 0
	for n < len(s) && s[n] >= '0' && s[n] <= '9' {
		n++
	}
	return n
}
