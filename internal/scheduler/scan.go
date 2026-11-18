package scheduler

import (
	"context"
	"strconv"
	"time"

	"github.com/henry40408/pingward/internal/config"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/notify"
)

// ScanOnce downs every active (new or up) check that is overdue or overrun and
// returns a Down event for each. A failure on one check is logged and skipped
// rather than aborting the round.
func (d Deps) ScanOnce(ctx context.Context, now time.Time) ([]notify.Event, error) {
	projectNames, err := d.Store.AllProjectNames(ctx)
	if err != nil {
		return nil, err
	}
	displayTimezone := d.Store.DisplayTimezone(ctx)
	checks, err := d.Store.ListActiveChecks(ctx)
	if err != nil {
		return nil, err
	}
	var events []notify.Event
	for _, check := range checks {
		overdue := reached(DueTime(check), now)
		overrun := reached(OverrunTime(check), now)
		if !overdue && !overrun {
			continue
		}
		// check is the snapshot from before the status update; the event detail
		// is built from it, not re-read.
		if err := d.Store.SetStatus(ctx, check.ID, models.StatusDown); err != nil {
			d.Logger.Error("failed to down check "+strconv.FormatInt(check.ID, 10), "error", err)
			continue
		}
		if err := d.Store.BeginDownAlert(ctx, check.ID, now); err != nil {
			d.Logger.Error("failed to set alert baseline for "+strconv.FormatInt(check.ID, 10), "error", err)
		}
		detail := d.detail(check, projectNames, displayTimezone).WithCause(downCause(check, overrun))
		events = append(events, notify.Event{
			CheckID: check.ID, CheckName: check.Name, Kind: models.EventDown, At: now,
			ProjectID: check.ProjectID, Detail: detail,
		})
	}
	return events, nil
}

// reached is now >= deadline; the boundary counts as due.
func reached(deadline *time.Time, now time.Time) bool {
	return deadline != nil && !now.Before(*deadline)
}

// downCause lets overrun win over overdue: it is the more specific cause.
func downCause(check models.Check, overrun bool) notify.DownCause {
	if overrun && check.MaxRuntimeSecs != nil && check.LastStartAt != nil {
		return notify.DownCause{
			Kind: notify.CauseOverrun, MaxRuntimeSecs: *check.MaxRuntimeSecs, StartedAt: *check.LastStartAt,
		}
	}
	return notify.DownCause{Kind: notify.CauseOverdue}
}

func (d Deps) detail(check models.Check, projectNames map[int64]string, displayTimezone string) notify.EventDetail {
	var projectName *string
	if name, ok := projectNames[check.ProjectID]; ok {
		projectName = &name
	}
	detail := notify.DetailFromCheck(check, projectName, d.BaseURL)
	if displayTimezone == "" {
		return detail
	}
	return detail.WithDisplayTimezone(&displayTimezone)
}

// NagOnce returns a Reminder for every down, unacknowledged check whose nag
// interval has elapsed since last_alert_at, which it advances to now.
func (d Deps) NagOnce(ctx context.Context, now time.Time) ([]notify.Event, error) {
	projectNags, err := d.Store.AllProjectNagIntervals(ctx)
	if err != nil {
		return nil, err
	}
	projectNames, err := d.Store.AllProjectNames(ctx)
	if err != nil {
		return nil, err
	}
	displayTimezone := d.Store.DisplayTimezone(ctx)
	globalNag, err := d.intervalSetting(ctx, "nag_interval")
	if err != nil {
		return nil, err
	}
	checks, err := d.Store.ListDownChecks(ctx)
	if err != nil {
		return nil, err
	}
	var events []notify.Event
	for _, check := range checks {
		if !reminderDue(check, projectNags[check.ProjectID], globalNag, now) {
			continue
		}
		if err := d.Store.RecordReminder(ctx, check.ID, now); err != nil {
			d.Logger.Error("failed to record reminder for "+strconv.FormatInt(check.ID, 10), "error", err)
			continue
		}
		events = append(events, notify.Event{
			CheckID: check.ID, CheckName: check.Name, Kind: models.EventReminder, At: now,
			ProjectID: check.ProjectID, Detail: d.detail(check, projectNames, displayTimezone),
		})
	}
	return events, nil
}

func reminderDue(check models.Check, projectNag, globalNag *int64, now time.Time) bool {
	if check.Acknowledged || check.LastAlertAt == nil {
		return false
	}
	interval, ok := config.EffectiveNagInterval(check.NagIntervalSecs, projectNag, globalNag)
	if !ok {
		return false
	}
	return !now.Before(check.LastAlertAt.Add(time.Duration(interval) * time.Second))
}

// intervalSetting is nil when the setting is unset or not a number.
func (d Deps) intervalSetting(ctx context.Context, key string) (*int64, error) {
	value, err := d.Store.GetSetting(ctx, key)
	if err != nil || value == nil {
		return nil, err
	}
	seconds, err := strconv.ParseInt(*value, 10, 64)
	if err != nil {
		return nil, nil
	}
	return &seconds, nil
}

// loopIntervalSecs is the smallest effective scan interval across checks, else
// the environment default, at least 1.
func loopIntervalSecs(checks []models.Check, projectIntervals map[int64]*int64, global *int64, envDefault int64) int64 {
	smallest := int64(0)
	for _, check := range checks {
		seconds := config.EffectiveScanInterval(check.ScanIntervalSecs, projectIntervals[check.ProjectID], global, envDefault)
		if smallest == 0 || seconds < smallest {
			smallest = seconds
		}
	}
	if smallest == 0 {
		return max(envDefault, 1)
	}
	return smallest
}
