package scheduler

import (
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

func utc(year int, month time.Month, day, hour, minute int) time.Time {
	return time.Date(year, month, day, hour, minute, 0, 0, time.UTC)
}

func int64Ptr(v int64) *int64        { return &v }
func timePtr(v time.Time) *time.Time { return &v }
func stringPtr(v string) *string     { return &v }

func baseCheck() models.Check {
	return models.Check{
		ID: 1, ProjectID: 1, Name: "j", PingUUID: "u",
		ScheduleKind: models.SchedulePeriod, PeriodSecs: int64Ptr(3600), GraceSecs: 300,
		Timezone: "UTC", Status: models.StatusUp,
		LastPingAt: timePtr(utc(2026, 7, 12, 12, 0)),
		CreatedAt:  utc(2026, 7, 12, 11, 0),
	}
}

func cronCheck(expr, timezone string) models.Check {
	check := baseCheck()
	check.ScheduleKind = models.ScheduleCron
	check.PeriodSecs = nil
	check.CronExpr = stringPtr(expr)
	check.Timezone = timezone
	return check
}

func cronCheckAnchoredAt(expr, timezone string, anchor time.Time) models.Check {
	check := cronCheck(expr, timezone)
	check.GraceSecs = 0
	check.LastPingAt = &anchor
	return check
}

func requireDue(t *testing.T, check models.Check, want time.Time) {
	t.Helper()
	got := DueTime(check)
	if got == nil {
		t.Fatalf("DueTime = nil, want %v", want)
	}
	if !got.Equal(want) {
		t.Fatalf("DueTime = %v, want %v", got.UTC(), want)
	}
}

func TestPeriodDueIsLastPingPlusPeriodPlusGrace(t *testing.T) {
	requireDue(t, baseCheck(), utc(2026, 7, 12, 13, 5))
}

func TestCronDueIsNextTriggerPlusGrace(t *testing.T) {
	requireDue(t, cronCheck("0 0 * * * *", "UTC"), utc(2026, 7, 12, 13, 5))
}

func TestPeriodWithoutPeriodSecsIsNone(t *testing.T) {
	check := baseCheck()
	check.PeriodSecs = nil
	if DueTime(check) != nil {
		t.Fatal("want nil")
	}
}

func TestFirstRunAnchorIsCreatedAt(t *testing.T) {
	check := baseCheck()
	check.LastPingAt = nil
	check.CreatedAt = utc(2026, 7, 12, 9, 0)
	check.PeriodSecs = int64Ptr(1800)
	check.GraceSecs = 60
	requireDue(t, check, time.Date(2026, 7, 12, 9, 31, 0, 0, time.UTC))
}

func TestCronInvalidTimezoneFallsBackToUTC(t *testing.T) {
	invalid := DueTime(cronCheck("0 0 * * * *", "Not/AZone"))
	valid := DueTime(cronCheck("0 0 * * * *", "UTC"))
	if invalid == nil || valid == nil || !invalid.Equal(*valid) {
		t.Fatalf("invalid zone %v, UTC %v", invalid, valid)
	}
	requireDue(t, cronCheck("0 0 * * * *", "Not/AZone"), utc(2026, 7, 12, 13, 5))
}

// The cron crate skips local times that do not exist or that repeat; it never
// fires early. These tests pin that, so an upgrade cannot change it silently.

func TestCronDailyInDSTGapSkipsToNextDay(t *testing.T) {
	check := cronCheckAnchoredAt("0 30 2 * * *", "America/New_York", utc(2026, 3, 7, 12, 0))
	requireDue(t, check, utc(2026, 3, 9, 6, 30))
}

func TestCronHourlyAcrossDSTGapResumesAfterTheGap(t *testing.T) {
	check := cronCheckAnchoredAt("0 0 * * * *", "America/New_York", utc(2026, 3, 8, 6, 30))
	requireDue(t, check, utc(2026, 3, 8, 7, 0))
}

func TestCronDailyInDSTOverlapFiresOnce(t *testing.T) {
	check := cronCheckAnchoredAt("0 30 1 * * *", "America/New_York", utc(2026, 11, 1, 5, 45))
	requireDue(t, check, utc(2026, 11, 2, 6, 30))
}

func TestCronInZoneWithoutDSTIsUnaffectedOnTheSameDates(t *testing.T) {
	check := cronCheckAnchoredAt("0 30 2 * * *", "Asia/Taipei", utc(2026, 3, 7, 12, 0))
	requireDue(t, check, utc(2026, 3, 7, 18, 30))
}

func TestCronWithoutExprIsNone(t *testing.T) {
	check := cronCheck("", "UTC")
	check.CronExpr = nil
	if DueTime(check) != nil {
		t.Fatal("want nil")
	}
}

func TestCronWithMalformedExprIsNone(t *testing.T) {
	if DueTime(cronCheck("not a cron", "UTC")) != nil {
		t.Fatal("want nil")
	}
}

func runningCheck(maxRuntime *int64, start time.Time, lastPing *time.Time) models.Check {
	check := baseCheck()
	check.MaxRuntimeSecs = maxRuntime
	check.LastStartAt = &start
	check.LastPingAt = lastPing
	return check
}

func TestOverrunWhenInFlightPastMaxRuntime(t *testing.T) {
	start := utc(2026, 7, 12, 12, 0)
	got := OverrunTime(runningCheck(int64Ptr(60), start, timePtr(utc(2026, 7, 12, 11, 0))))
	if got == nil || !got.Equal(start.Add(60*time.Second)) {
		t.Fatalf("OverrunTime = %v", got)
	}
}

func TestNoOverrunWhenCompletedAfterStart(t *testing.T) {
	check := runningCheck(int64Ptr(60), utc(2026, 7, 12, 12, 0), timePtr(time.Date(2026, 7, 12, 12, 0, 30, 0, time.UTC)))
	if OverrunTime(check) != nil {
		t.Fatal("want nil")
	}
}

func TestNoOverrunWithoutMaxRuntimeOrStart(t *testing.T) {
	start := utc(2026, 7, 12, 12, 0)
	if OverrunTime(runningCheck(nil, start, nil)) != nil {
		t.Fatal("nil max runtime")
	}
	if OverrunTime(runningCheck(int64Ptr(0), start, nil)) != nil {
		t.Fatal("non-positive max runtime is off")
	}
	noStart := baseCheck()
	noStart.MaxRuntimeSecs = int64Ptr(60)
	if OverrunTime(noStart) != nil {
		t.Fatal("no start")
	}
}

func TestOverrunWithNeverPingedCheckIsInFlight(t *testing.T) {
	start := utc(2026, 7, 12, 12, 0)
	if OverrunTime(runningCheck(int64Ptr(60), start, nil)) == nil {
		t.Fatal("last ping none counts as in flight")
	}
}

func TestOverrunStartEqualToLastPingIsNotInFlight(t *testing.T) {
	start := utc(2026, 7, 12, 12, 0)
	if OverrunTime(runningCheck(int64Ptr(60), start, &start)) != nil {
		t.Fatal("start must be strictly newer than the last ping")
	}
}
