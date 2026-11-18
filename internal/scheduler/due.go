// Package scheduler decides when checks are overdue and sends the down and
// reminder notifications.
package scheduler

import (
	"log/slog"
	"time"

	"github.com/henry40408/pingward/internal/cron"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/tz"
)

func anchor(check models.Check) time.Time {
	if check.LastPingAt != nil {
		return *check.LastPingAt
	}
	return check.CreatedAt
}

// DueTime is when check becomes overdue, grace included. It is nil without a
// usable period or cron expression, so such a check never goes overdue.
func DueTime(check models.Check) *time.Time {
	grace := time.Duration(check.GraceSecs) * time.Second
	if check.ScheduleKind == models.SchedulePeriod {
		if check.PeriodSecs == nil {
			return nil
		}
		due := anchor(check).Add(time.Duration(*check.PeriodSecs)*time.Second + grace)
		return &due
	}
	if check.CronExpr == nil {
		return nil
	}
	schedule, err := cron.Parse(*check.CronExpr)
	if err != nil {
		return nil
	}
	next, ok := schedule.After(anchor(check), locationOrUTC(check))
	if !ok {
		return nil
	}
	due := next.Add(grace)
	return &due
}

func locationOrUTC(check models.Check) *time.Location {
	loc, err := tz.Load(check.Timezone)
	if err != nil {
		slog.Warn("invalid timezone on check, falling back to UTC",
			"check_id", check.ID, "timezone", check.Timezone)
		return time.UTC
	}
	return loc
}

// OverrunTime is last_start_at + max_runtime for a run in flight (a start newer
// than the last ping). It is nil when no run is in flight or max_runtime is
// not positive.
func OverrunTime(check models.Check) *time.Time {
	if check.MaxRuntimeSecs == nil || *check.MaxRuntimeSecs <= 0 || check.LastStartAt == nil {
		return nil
	}
	start := *check.LastStartAt
	if check.LastPingAt != nil && !start.After(*check.LastPingAt) {
		return nil
	}
	deadline := start.Add(time.Duration(*check.MaxRuntimeSecs) * time.Second)
	return &deadline
}
