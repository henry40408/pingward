package view

import (
	"time"

	"github.com/henry40408/pingward/internal/models"
)

// DisplayStatus is the stored status refined by what is happening now.
type DisplayStatus string

const (
	StatusNew     DisplayStatus = "new"
	StatusUp      DisplayStatus = "up"
	StatusRunning DisplayStatus = "running"
	StatusLate    DisplayStatus = "late"
	StatusDown    DisplayStatus = "down"
	StatusPaused  DisplayStatus = "paused"
)

// isRunning: a start is newer than the last finish. A start with no finish
// at all also counts.
func isRunning(check models.Check) bool {
	if check.LastStartAt == nil {
		return false
	}
	return check.LastPingAt == nil || check.LastStartAt.After(*check.LastPingAt)
}

// Status applies the precedence Paused > Down > Running > Late > Up: a long job
// may drift past its expected time, but an in-flight run never masks an alert.
// Late is a stored-Up check inside (next_due_at - grace, next_due_at].
func Status(check models.Check, now time.Time) DisplayStatus {
	switch check.Status {
	case models.StatusDown:
		return StatusDown
	case models.StatusPaused:
		return StatusPaused
	case models.StatusNew:
		if isRunning(check) {
			return StatusRunning
		}
		return StatusNew
	}
	if isRunning(check) {
		return StatusRunning
	}
	if check.NextDueAt == nil {
		return StatusUp
	}
	due := *check.NextDueAt
	expected := due.Add(-time.Duration(check.GraceSecs) * time.Second)
	if now.After(expected) && !now.After(due) {
		return StatusLate
	}
	return StatusUp
}
