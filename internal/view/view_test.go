package view

import (
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/duration"
	"github.com/henry40408/pingward/internal/models"
)

var noon = time.Date(2026, 7, 14, 12, 0, 0, 0, time.UTC)

func baseCheck() models.Check {
	period := int64(3600)
	return models.Check{
		ID: 1, ProjectID: 1, Name: "c", PingUUID: "u", ScheduleKind: models.SchedulePeriod,
		PeriodSecs: &period, GraceSecs: 300, Timezone: "UTC", Status: models.StatusUp, CreatedAt: noon,
	}
}

func at(offset time.Duration) *time.Time {
	t := noon.Add(offset)
	return &t
}

func TestStatusLateInsideTheGraceWindow(t *testing.T) {
	check := baseCheck()
	check.NextDueAt = at(120 * time.Second)
	if got := Status(check, noon); got != StatusLate {
		t.Fatal(got)
	}
	check.NextDueAt = at(3000 * time.Second)
	if got := Status(check, noon); got != StatusUp {
		t.Fatal(got)
	}
}

func TestStatusRunningBeatsLateAndNew(t *testing.T) {
	check := baseCheck()
	check.NextDueAt = at(120 * time.Second)
	check.LastPingAt = at(-4000 * time.Second)
	check.LastStartAt = at(-10 * time.Second)
	if got := Status(check, noon); got != StatusRunning {
		t.Fatal(got)
	}
	fresh := baseCheck()
	fresh.Status = models.StatusNew
	fresh.LastStartAt = at(0)
	if got := Status(fresh, noon); got != StatusRunning {
		t.Fatal(got)
	}
}

func TestStatusDownAndPausedIgnoreRunning(t *testing.T) {
	check := baseCheck()
	check.LastPingAt = at(-100 * time.Second)
	check.LastStartAt = at(0)
	for stored, want := range map[models.CheckStatus]DisplayStatus{
		models.StatusDown: StatusDown, models.StatusPaused: StatusPaused,
	} {
		check.Status = stored
		if got := Status(check, noon); got != want {
			t.Fatalf("%s: %s", stored, got)
		}
	}
}

func TestStatusRunningClearedByALaterSuccessAndNoneIsNotRunning(t *testing.T) {
	check := baseCheck()
	check.LastStartAt = at(-50 * time.Second)
	check.LastPingAt = at(0)
	if got := Status(check, noon); got != StatusUp {
		t.Fatal(got)
	}
	empty := baseCheck()
	empty.Status = models.StatusNew
	if got := Status(empty, noon); got != StatusNew {
		t.Fatal(got)
	}
}

func ping(id int64, kind models.PingKind, offset time.Duration) models.PingSummary {
	return models.PingSummary{ID: id, CheckID: 1, Kind: kind, CreatedAt: noon.Add(offset)}
}

func TestRunDurationPairsStartWithTheNextFinish(t *testing.T) {
	got := RunDurations([]models.PingSummary{
		ping(2, models.PingSuccess, 242*time.Second), ping(1, models.PingStart, 0),
	})
	if got[2] != 242 || len(got) != 1 {
		t.Fatal(got)
	}
}

func TestHeartbeatVariants(t *testing.T) {
	unknown := Heartbeat([]models.PingSummary{
		ping(1, models.PingSuccess, 0), ping(2, models.PingSuccess, time.Minute),
	}, HeartbeatOptions{Count: 6})
	for _, b := range unknown {
		if b.Class != "none" || b.Title != "duration unknown" {
			t.Fatal(b)
		}
	}
	timed := []models.PingSummary{ping(1, models.PingStart, 0), ping(2, models.PingSuccess, 42*time.Second)}
	if b := Heartbeat(timed, HeartbeatOptions{Count: 6}); b[0].Title != "no runtime limit set" {
		t.Fatal(b)
	}
	limit := int64(100)
	hot := []models.PingSummary{ping(1, models.PingStart, 0), ping(2, models.PingSuccess, 90*time.Second)}
	if b := Heartbeat(hot, HeartbeatOptions{Count: 6, MaxRuntimeSecs: &limit}); b[0].Class != "hot" || b[0].Height != 23 || b[0].Title != "1m 30s / 1m 40s" {
		t.Fatal(b)
	}
	paused := Heartbeat(nil, HeartbeatOptions{Count: 6, Paused: true})
	if len(paused) != 6 || paused[0].Class != "pausedbar" {
		t.Fatal(paused)
	}
}

func TestFormattersFloorAtZeroAndPickCoarseUnits(t *testing.T) {
	cases := map[string]string{
		FmtUntil(noon.Add(45*time.Second), noon):     "in 45s",
		FmtUntil(noon.Add(750*time.Second), noon):    "in 12m",
		FmtUntil(noon.Add(3*time.Hour), noon):        "in 3h",
		FmtUntil(noon.Add(48*time.Hour), noon):       "in 2d",
		FmtUntil(noon.Add(-time.Hour), noon):         "in 0s",
		FmtRelative(noon.Add(-90*time.Second), noon): "1m ago",
		FmtSecs(-5):   "0s",
		FmtSecs(185):  "3m 05s",
		FmtSecs(7620): "2h 07m",
		FmtUTC(noon):  "2026-07-14 12:00:00 UTC",
	}
	for got, want := range cases {
		if got != want {
			t.Fatalf("%q != %q", got, want)
		}
	}
}

func TestSuggestionsAreAcceptedSortedAndDistinct(t *testing.T) {
	for _, list := range [][]string{DurationSuggestions(), ExpirySuggestions()} {
		previous := int64(0)
		for _, text := range list {
			secs, ok := duration.Parse(text)
			if !ok || secs <= previous {
				t.Fatalf("%q: %d %v after %d", text, secs, ok, previous)
			}
			previous = secs
		}
	}
}

func TestNextDueLabels(t *testing.T) {
	check := baseCheck()
	check.LastPingAt = at(-30 * time.Minute)
	due := at(35 * time.Minute)
	got := NextDueFor(check, due, noon)
	if got.Label != "due in 35m" || got.ISO != "2026-07-14T12:35:00+00:00" {
		t.Fatal(got)
	}
	check.LastPingAt = nil
	if got := NextDueFor(check, at(time.Hour), noon); got.Label != "first ping due in 1h" {
		t.Fatal(got)
	}
	check.Status = models.StatusDown
	if got := NextDueFor(check, at(-10*time.Minute), noon); got.Label != "overdue by 10m 00s" || got.ISO == "" {
		t.Fatal(got)
	}
	if got := NextDueFor(check, nil, noon); got != (NextDue{Label: "next due unknown"}) {
		t.Fatal(got)
	}
	check.Status = models.StatusPaused
	if got := NextDueFor(check, at(-time.Hour), noon); got != (NextDue{Label: "not scheduled while paused"}) {
		t.Fatal(got)
	}
}

func TestRFC3339UsesChronoDigitGroups(t *testing.T) {
	base := time.Date(2026, 11, 17, 15, 31, 6, 0, time.UTC)
	for nanos, want := range map[int]string{
		0:           "2026-11-17T15:31:06+00:00",
		120_000_000: "2026-11-17T15:31:06.120+00:00",
		120_500_000: "2026-11-17T15:31:06.120500+00:00",
		949262560:   "2026-11-17T15:31:06.949262560+00:00",
	} {
		if got := rfc3339(base.Add(time.Duration(nanos))); got != want {
			t.Fatalf("%s != %s", got, want)
		}
	}
}
