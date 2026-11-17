package view

import (
	"time"

	"github.com/henry40408/pingward/internal/models"
)

// NextDue is the "when is the next ping expected" line on the check page.
type NextDue struct {
	// Label is the visible text, such as "due in 57m" or "overdue by 12m 00s".
	Label string
	// ISO is the RFC 3339 deadline for the title; empty when Label names a state.
	ISO string
}

// NextDueFor labels the check's next deadline. due is the scheduler's deadline
// (the one a scan evaluates), not the stored next_due_at, which is NULL for a
// never-pinged check or one downed by a fail ping. nil means it cannot be
// evaluated. The deadline includes grace, hence "due" rather than "expected".
func NextDueFor(check models.Check, due *time.Time, now time.Time) NextDue {
	if check.Status == models.StatusPaused {
		return NextDue{Label: "not scheduled while paused"}
	}
	if due == nil {
		return NextDue{Label: "next due unknown"}
	}
	label := "due " + FmtUntil(*due, now)
	switch {
	case !now.Before(*due):
		label = "overdue by " + FmtSecs(int64(now.Sub(*due)/time.Second))
	case check.LastPingAt == nil:
		// Anchored on creation; don't imply a run already happened.
		label = "first ping due " + FmtUntil(*due, now)
	}
	return NextDue{Label: label, ISO: RFC3339(*due)}
}

// RFC3339 prints a UTC time like chrono's to_rfc3339: "+00:00" rather than "Z",
// and fractional digits in groups of three, only when the time has any.
func RFC3339(at time.Time) string {
	at = at.UTC()
	layout := "2006-01-02T15:04:05"
	switch nanos := at.Nanosecond(); {
	case nanos == 0:
	case nanos%1_000_000 == 0:
		layout += ".000"
	case nanos%1000 == 0:
		layout += ".000000"
	default:
		layout += ".000000000"
	}
	return at.Format(layout + "-07:00")
}
