package db

import (
	"fmt"
	"time"
)

// Timestamps are stored as TEXT and compared as text, so the form must match
// what the Rust build wrote. Never bind a time.Time directly: the driver picks
// its own text form.

const (
	rfc3339Seconds = "2006-01-02T15:04:05+00:00"
	rfc3339Millis  = "2006-01-02T15:04:05.000+00:00"
	rfc3339Micros  = "2006-01-02T15:04:05.000000+00:00"
	rfc3339Nanos   = "2006-01-02T15:04:05.000000000+00:00"
	spaceLayout    = "2006-01-02 15:04:05"
)

// FormatTime renders t like chrono's to_rfc3339 in UTC: "+00:00" for the zone
// and a 3, 6 or 9 digit fraction only when the sub-second part is non-zero.
func FormatTime(t time.Time) string {
	t = t.UTC()
	nanos := t.Nanosecond()
	switch {
	case nanos == 0:
		return t.Format(rfc3339Seconds)
	case nanos%1_000_000 == 0:
		return t.Format(rfc3339Millis)
	case nanos%1_000 == 0:
		return t.Format(rfc3339Micros)
	default:
		return t.Format(rfc3339Nanos)
	}
}

// ParseTime decodes a stored timestamp: RFC 3339 with "+00:00" or "Z" (the
// e2e seed uses Z), or "YYYY-MM-DD HH:MM:SS" (SQLite's datetime('now')).
func ParseTime(text string) (time.Time, error) {
	if t, err := time.Parse(time.RFC3339Nano, text); err == nil {
		return t.UTC(), nil
	}
	if t, err := time.ParseInLocation(spaceLayout, text, time.UTC); err == nil {
		return t, nil
	}
	return time.Time{}, fmt.Errorf("unrecognised timestamp %q", text)
}
