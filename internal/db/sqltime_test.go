package db

import (
	"testing"
	"time"
)

func TestFormatTimeMatchesChronoToRFC3339(t *testing.T) {
	base := time.Date(2026, 3, 4, 5, 6, 7, 0, time.UTC)
	cases := []struct {
		nanos int
		want  string
	}{
		{0, "2026-03-04T05:06:07+00:00"},
		{500_000_000, "2026-03-04T05:06:07.500+00:00"},
		{123_000_000, "2026-03-04T05:06:07.123+00:00"},
		{123_456_000, "2026-03-04T05:06:07.123456+00:00"},
		{1_000, "2026-03-04T05:06:07.000001+00:00"},
		{123_456_789, "2026-03-04T05:06:07.123456789+00:00"},
		{1, "2026-03-04T05:06:07.000000001+00:00"},
	}
	for _, c := range cases {
		if got := FormatTime(base.Add(time.Duration(c.nanos))); got != c.want {
			t.Errorf("%d ns: got %s, want %s", c.nanos, got, c.want)
		}
	}
}

func TestFormatTimeConvertsToUTC(t *testing.T) {
	taipei := time.FixedZone("UTC+8", 8*3600)
	got := FormatTime(time.Date(2026, 3, 4, 13, 6, 7, 0, taipei))
	if got != "2026-03-04T05:06:07+00:00" {
		t.Fatal(got)
	}
}

func TestParseTimeAcceptsEveryStoredForm(t *testing.T) {
	want := time.Date(2026, 3, 4, 5, 6, 7, 0, time.UTC)
	for _, text := range []string{
		"2026-03-04T05:06:07+00:00",
		"2026-03-04T05:06:07Z",
		"2026-03-04 05:06:07",
		"2026-03-04T13:06:07+08:00",
	} {
		got, err := ParseTime(text)
		if err != nil || !got.Equal(want) || got.Location() != time.UTC {
			t.Errorf("%q: %v %v", text, got, err)
		}
	}
	got, err := ParseTime("2026-03-04T05:06:07.123456+00:00")
	if err != nil || got.Nanosecond() != 123_456_000 {
		t.Errorf("fraction: %v %v", got, err)
	}
	for _, junk := range []string{"", "yesterday", "2026-03-04"} {
		if _, err := ParseTime(junk); err == nil {
			t.Errorf("%q must not parse", junk)
		}
	}
}

func TestFormatThenParseRoundTrips(t *testing.T) {
	original := time.Date(2026, 3, 4, 5, 6, 7, 123_456_789, time.UTC)
	back, err := ParseTime(FormatTime(original))
	if err != nil || !back.Equal(original) {
		t.Fatalf("%v %v", back, err)
	}
}
