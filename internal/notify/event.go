// Package notify renders and delivers down/up/reminder/test notifications.
package notify

import (
	"fmt"
	"strings"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

// CauseKind is the stable machine name of why a check went down.
type CauseKind string

const (
	CauseOverdue CauseKind = "overdue"
	CauseOverrun CauseKind = "overrun"
	CauseFailed  CauseKind = "failed"
)

// DownCause says why a check went down: nothing pinged, a run overran its
// budget, or the job reported failure.
type DownCause struct {
	Kind CauseKind
	// MaxRuntimeSecs and StartedAt are set for CauseOverrun.
	MaxRuntimeSecs int64
	StartedAt      time.Time
	// ExitCode is set for CauseFailed when the ping carried one.
	ExitCode *int64
}

// EventDetail is the context around a state change. Every field is optional so
// a failed lookup drops a line, not the notification.
//
// Build it from the pre-update check snapshot, never re-read during delivery:
// on an up event LastPingAt must be the ping before the recovery.
type EventDetail struct {
	ProjectName *string
	// URL is the absolute check page, derived from the base URL.
	URL      *string
	Schedule *string
	// LastPingAt is the check's last completion before this event.
	LastPingAt *time.Time
	// Timezone is the zone timestamps render in (UTC if unset or unknown).
	Timezone *string
	// Cause is set on down events only.
	Cause *DownCause
}

// Event is one notification to deliver.
type Event struct {
	CheckID   int64
	CheckName string
	Kind      models.EventKind
	At        time.Time
	ProjectID int64
	Detail    EventDetail
}

// DetailFromCheck snapshots check for an event.
func DetailFromCheck(check models.Check, projectName *string, baseURL string) EventDetail {
	schedule := ScheduleSummary(check)
	timezone := check.Timezone
	return EventDetail{
		ProjectName: projectName,
		URL:         checkURL(baseURL, check.ID),
		Schedule:    &schedule,
		LastPingAt:  check.LastPingAt,
		Timezone:    &timezone,
	}
}

// WithCause returns a copy of the detail carrying cause.
func (d EventDetail) WithCause(cause DownCause) EventDetail {
	d.Cause = &cause
	return d
}

// WithDisplayTimezone overrides the zone with the instance-wide
// display_timezone, since a notification has no browser to localise it. A
// blank value keeps the check's zone.
func (d EventDetail) WithDisplayTimezone(timezone *string) EventDetail {
	if timezone == nil {
		return d
	}
	trimmed := strings.TrimSpace(*timezone)
	if trimmed == "" {
		return d
	}
	d.Timezone = &trimmed
	return d
}

// checkURL is nil without a base URL, so the message omits the link rather
// than showing a bare path.
func checkURL(baseURL string, checkID int64) *string {
	base := strings.TrimRight(strings.TrimSpace(baseURL), "/")
	if base == "" {
		return nil
	}
	url := fmt.Sprintf("%s/checks/%d", base, checkID)
	return &url
}
