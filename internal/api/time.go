package api

import (
	"strings"
	"time"
)

// Time is a timestamp as the API prints it: UTC, RFC 3339, with 0, 3, 6 or 9
// fractional digits, whichever is the shortest that keeps the value exact.
// That is what chrono's SecondsFormat::AutoSi wrote, and clients may compare
// the strings.
type Time time.Time

func (t Time) MarshalJSON() ([]byte, error) {
	utc := time.Time(t).UTC()
	var layout strings.Builder
	layout.WriteString(`"2006-01-02T15:04:05`)
	switch nanos := utc.Nanosecond(); {
	case nanos == 0:
	case nanos%1_000_000 == 0:
		layout.WriteString(".000")
	case nanos%1_000 == 0:
		layout.WriteString(".000000")
	default:
		layout.WriteString(".000000000")
	}
	layout.WriteString(`Z"`)
	return []byte(utc.Format(layout.String())), nil
}

func timeOf(t time.Time) Time { return Time(t) }

func optionalTime(t *time.Time) *Time {
	if t == nil {
		return nil
	}
	converted := Time(*t)
	return &converted
}
