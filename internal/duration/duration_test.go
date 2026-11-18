package duration

import "testing"

func TestParse(t *testing.T) {
	cases := []struct {
		name  string
		input string
		want  int64
		ok    bool
	}{
		{"bare zero", "0", 0, true},
		{"bare 30", "30", 30, true},
		{"bare 3600", "3600", 3600, true},
		{"seconds", "45s", 45, true},
		{"minutes", "5m", 300, true},
		{"hours", "2h", 7200, true},
		{"days", "3d", 259200, true},
		{"combo", "1h30m", 5400, true},
		{"long combo", "1d2h3m4s", 93784, true},
		{"repeated unit", "1h1h", 7200, true},
		{"whitespace and uppercase", "1H 30M", 5400, true},
		{"surrounding whitespace", "  1h30m  ", 5400, true},
		{"negative", "-1h", -3600, true},
		{"negative bare", "-5", -5, true},
		{"empty", "", 0, false},
		{"blank", "   ", 0, false},
		{"unknown unit", "1x", 0, false},
		{"missing unit", "1h30", 0, false},
		{"letters", "abc", 0, false},
		{"unit first", "m5", 0, false},
		{"lone minus", "-", 0, false},
		{"lone unit", "h", 0, false},
		{"double unit", "1hh", 0, false},
		{"plus sign", "+5", 0, false},
		{"vertical tab is not whitespace", "1\v", 0, false},
		{"component overflow", "9223372036854775807d", 0, false},
		{"sum overflow", "106751991167300d106751991167300d", 0, false},
		{"bare overflow", "9223372036854775808", 0, false},
		{"non-ASCII digit", "１h", 0, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, ok := Parse(tc.input)
			if got != tc.want || ok != tc.ok {
				t.Fatalf("Parse(%q) = %d, %v; want %d, %v", tc.input, got, ok, tc.want, tc.ok)
			}
		})
	}
}

func TestFormat(t *testing.T) {
	cases := map[int64]string{
		0: "0s", 86400: "1d", 5430: "1h30m30s", 300: "5m", 90: "1m30s", 45: "45s", -7: "0s",
	}
	for seconds, want := range cases {
		if got := Format(seconds); got != want {
			t.Errorf("Format(%d) = %q, want %q", seconds, got, want)
		}
	}
}

func TestFormatRoundTripsThroughParse(t *testing.T) {
	for _, v := range []int64{0, 1, 45, 60, 90, 300, 3600, 5430, 86400, 90061} {
		got, ok := Parse(Format(v))
		if !ok || got != v {
			t.Errorf("Parse(Format(%d)) = %d, %v", v, got, ok)
		}
	}
}

func TestParseDigitAndOverflowBoundaries(t *testing.T) {
	cases := []struct {
		name  string
		input string
		want  int64
		ok    bool
	}{
		{"digit nine bare", "9", 9, true},
		{"digit nine with unit", "9s", 9, true},
		{"nines", "99m", 5940, true},
		{"zero then nine", "09s", 9, true},
		{"slash below zero", "1/s", 0, false},
		{"colon above nine", "1:s", 0, false},
		{"largest seconds component", "9223372036854775807s", 9223372036854775807, true},
		{"largest minutes component", "153722867280912930m", 9223372036854775800, true},
		{"largest days component", "106751991167300d", 9223372036854720000, true},
		{"one past largest seconds component", "9223372036854775808s", 0, false},
		{"one past largest minutes component", "153722867280912931m", 0, false},
		{"sum landing exactly on max int64", "9223372036854775800s7s", 9223372036854775807, true},
		{"sum one past max int64", "9223372036854775800s8s", 0, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, ok := Parse(tc.input)
			if got != tc.want || ok != tc.ok {
				t.Fatalf("Parse(%q) = %d, %v; want %d, %v", tc.input, got, ok, tc.want, tc.ok)
			}
		})
	}
}
