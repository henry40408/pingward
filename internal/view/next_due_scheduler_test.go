package view

import (
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/scheduler"
)

// These feed NextDueFor the real scheduler rule, as the check page does.
func TestNextDueCountsDownFromTheLastPing(t *testing.T) {
	check := baseCheck()
	// 1h period + 5m grace, pinged 30m ago: 35m left on the deadline.
	check.LastPingAt = at(-30 * time.Minute)
	got := NextDueFor(check, scheduler.DueTime(check), noon)
	if got.Label != "due in 35m" {
		t.Fatalf("label %q", got.Label)
	}
	if want := RFC3339(noon.Add(35 * time.Minute)); got.ISO != want {
		t.Fatalf("the tooltip carries the exact deadline: %q, want %q", got.ISO, want)
	}
}

func TestNextDueIsUnknownWhenTheScheduleCannotBeEvaluated(t *testing.T) {
	check := baseCheck()
	check.ScheduleKind = models.ScheduleCron
	badCron := "not a cron"
	check.CronExpr = &badCron
	got := NextDueFor(check, scheduler.DueTime(check), noon)
	if got != (NextDue{Label: "next due unknown"}) {
		t.Fatalf("%+v", got)
	}
}
