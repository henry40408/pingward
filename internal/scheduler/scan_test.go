package scheduler

import (
	"context"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/events"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/notify"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

const testBaseURL = "https://pingward.test"

type fixture struct {
	deps      Deps
	projectID int64
	logs      *testutil.LockedBuffer
}

func newFixture(t *testing.T) *fixture {
	t.Helper()
	st := testutil.NewStore(t)
	ctx := context.Background()
	userID, err := st.CreateUser(ctx, "u", nil, false, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	projectID, err := st.CreateProject(ctx, userID, store.ProjectSettings{Name: "p"}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	logger, logs := testutil.JSONLogger()
	deps := Deps{Store: st, Logger: logger, BaseURL: testBaseURL, EnvDefaultScanSecs: 1, Now: time.Now}
	return &fixture{deps: deps, projectID: projectID, logs: logs}
}

func (f *fixture) createCheck(t *testing.T, periodSecs, graceSecs int64) int64 {
	t.Helper()
	id, err := f.deps.Store.CreateCheck(context.Background(), store.NewCheck{
		ProjectID: f.projectID, Name: "job", PingUUID: "u1", Kind: models.SchedulePeriod,
		PeriodSecs: &periodSecs, GraceSecs: graceSecs, Timezone: "UTC",
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return id
}

func (f *fixture) markPing(t *testing.T, id int64, mark store.PingMark) {
	t.Helper()
	if err := f.deps.Store.MarkPing(context.Background(), id, mark); err != nil {
		t.Fatal(err)
	}
}

func (f *fixture) upCheckPingedAt(t *testing.T, periodSecs, graceSecs int64, lastPing time.Time) int64 {
	t.Helper()
	id := f.createCheck(t, periodSecs, graceSecs)
	f.markPing(t, id, store.PingMark{Status: models.StatusUp, LastPingAt: &lastPing})
	return id
}

func (f *fixture) status(t *testing.T, id int64) models.CheckStatus {
	t.Helper()
	check, err := f.deps.Store.FindCheck(context.Background(), id)
	if err != nil || check == nil {
		t.Fatalf("FindCheck: %v, %v", check, err)
	}
	return check.Status
}

func (f *fixture) update(t *testing.T, id int64, change func(*store.UpdateCheck)) {
	t.Helper()
	update := store.UpdateCheck{
		Name: "job", Kind: models.SchedulePeriod, PeriodSecs: int64Ptr(3600), GraceSecs: 300, Timezone: "UTC",
	}
	change(&update)
	if err := f.deps.Store.UpdateCheckSchedule(context.Background(), id, update); err != nil {
		t.Fatal(err)
	}
}

func scan(t *testing.T, f *fixture, now time.Time) []notify.Event {
	t.Helper()
	got, err := f.deps.ScanOnce(context.Background(), now)
	if err != nil {
		t.Fatal(err)
	}
	return got
}

func nag(t *testing.T, f *fixture, now time.Time) []notify.Event {
	t.Helper()
	got, err := f.deps.NagOnce(context.Background(), now)
	if err != nil {
		t.Fatal(err)
	}
	return got
}

func TestOverdueCheckTransitionsToDownAndEmitsEvent(t *testing.T) {
	f := newFixture(t)
	// 60 + 30 = 90s window; last ping 200s ago.
	id := f.upCheckPingedAt(t, 60, 30, time.Now().Add(-200*time.Second))
	got := scan(t, f, time.Now())
	if len(got) != 1 || got[0].CheckID != id || got[0].Kind != models.EventDown {
		t.Fatalf("events = %+v", got)
	}
	if f.status(t, id) != models.StatusDown {
		t.Fatal("check must be down")
	}
}

func TestHealthyCheckIsNotDowned(t *testing.T) {
	f := newFixture(t)
	id := f.upCheckPingedAt(t, 60, 30, time.Now().Add(-10*time.Second))
	if got := scan(t, f, time.Now()); len(got) != 0 {
		t.Fatalf("events = %+v", got)
	}
	if f.status(t, id) != models.StatusUp {
		t.Fatal("check must stay up")
	}
}

func TestScanOnceIsIdempotent(t *testing.T) {
	f := newFixture(t)
	id := f.upCheckPingedAt(t, 60, 30, time.Now().Add(-200*time.Second))
	now := time.Now()
	if len(scan(t, f, now)) != 1 {
		t.Fatal("first scan must down the check")
	}
	// Already down, so excluded from the active list: no second event.
	if got := scan(t, f, now); len(got) != 0 {
		t.Fatalf("second scan events = %+v", got)
	}
	if f.status(t, id) != models.StatusDown {
		t.Fatal("check must stay down")
	}
}

func TestScanOnceDownsCheckExactlyAtDueBoundary(t *testing.T) {
	f := newFixture(t)
	t0 := utc(2026, 7, 12, 12, 0)
	id := f.upCheckPingedAt(t, 60, 30, t0)
	// Pins now >= due, not >.
	if len(scan(t, f, t0.Add(90*time.Second))) != 1 || f.status(t, id) != models.StatusDown {
		t.Fatal("the due instant itself must down the check")
	}
}

func TestScanOnceDoesNotDownCheckOneSecondBeforeDue(t *testing.T) {
	f := newFixture(t)
	t0 := utc(2026, 7, 12, 12, 0)
	id := f.upCheckPingedAt(t, 60, 30, t0)
	if got := scan(t, f, t0.Add(89*time.Second)); len(got) != 0 {
		t.Fatalf("events = %+v", got)
	}
	if f.status(t, id) != models.StatusUp {
		t.Fatal("check must stay up")
	}
}

func TestScanOnceOnlyWatchesNewAndUpChecks(t *testing.T) {
	f := newFixture(t)
	id := f.upCheckPingedAt(t, 60, 30, utc(2026, 7, 12, 12, 0))
	if err := f.deps.Store.SetStatus(context.Background(), id, models.StatusPaused); err != nil {
		t.Fatal(err)
	}
	if got := scan(t, f, utc(2026, 7, 13, 12, 0)); len(got) != 0 {
		t.Fatalf("a paused check must not go down: %+v", got)
	}
}

func TestScanOnceDownEventCarriesThePreUpdateSnapshot(t *testing.T) {
	f := newFixture(t)
	f.upCheckPingedAt(t, 60, 30, utc(2026, 7, 12, 12, 0))
	got := scan(t, f, utc(2026, 7, 12, 13, 0))
	if len(got) != 1 {
		t.Fatalf("events = %+v", got)
	}
	detail := got[0].Detail
	if detail.LastPingAt == nil || !detail.LastPingAt.Equal(utc(2026, 7, 12, 12, 0)) {
		t.Errorf("LastPingAt = %v", detail.LastPingAt)
	}
	if detail.ProjectName == nil || *detail.ProjectName != "p" {
		t.Errorf("ProjectName = %v", detail.ProjectName)
	}
	if detail.URL == nil || *detail.URL != testBaseURL+"/checks/1" {
		t.Errorf("URL = %v", detail.URL)
	}
	if detail.Cause == nil || detail.Cause.Kind != notify.CauseOverdue {
		t.Errorf("Cause = %+v", detail.Cause)
	}
}

func TestScanOnceUsesTheDisplayTimezoneSetting(t *testing.T) {
	f := newFixture(t)
	f.upCheckPingedAt(t, 60, 30, utc(2026, 7, 12, 12, 0))
	if err := f.deps.Store.SetSetting(context.Background(), "display_timezone", "Asia/Taipei"); err != nil {
		t.Fatal(err)
	}
	got := scan(t, f, utc(2026, 7, 12, 13, 0))
	if len(got) != 1 || got[0].Detail.Timezone == nil || *got[0].Detail.Timezone != "Asia/Taipei" {
		t.Fatalf("events = %+v", got)
	}
}

func TestScanOnceDownsOverrunCheck(t *testing.T) {
	f := newFixture(t)
	start := utc(2026, 7, 12, 12, 0)
	// A long period so the check is NOT overdue; only the in-flight start and
	// the short max runtime can down it.
	id := f.createCheck(t, 3_600_000, 300)
	f.update(t, id, func(u *store.UpdateCheck) {
		u.PeriodSecs = int64Ptr(3_600_000)
		u.MaxRuntimeSecs = int64Ptr(60)
	})
	f.markPing(t, id, store.PingMark{Status: models.StatusUp, LastStartAt: &start})

	got := scan(t, f, start.Add(61*time.Second))
	if len(got) != 1 || got[0].Kind != models.EventDown {
		t.Fatalf("events = %+v", got)
	}
	if f.status(t, id) != models.StatusDown {
		t.Fatal("check must be down")
	}
	cause := got[0].Detail.Cause
	if cause == nil || cause.Kind != notify.CauseOverrun || cause.MaxRuntimeSecs != 60 || !cause.StartedAt.Equal(start) {
		t.Fatalf("cause = %+v", cause)
	}
}

func TestOverrunWinsOverOverdue(t *testing.T) {
	f := newFixture(t)
	start := utc(2026, 7, 12, 12, 0)
	id := f.createCheck(t, 60, 30)
	f.update(t, id, func(u *store.UpdateCheck) {
		u.PeriodSecs = int64Ptr(60)
		u.GraceSecs = 30
		u.MaxRuntimeSecs = int64Ptr(60)
	})
	older := start.Add(-time.Hour)
	f.markPing(t, id, store.PingMark{Status: models.StatusUp, LastPingAt: &older, LastStartAt: &start})

	got := scan(t, f, start.Add(time.Hour))
	if len(got) != 1 || got[0].Detail.Cause == nil || got[0].Detail.Cause.Kind != notify.CauseOverrun {
		t.Fatalf("events = %+v", got)
	}
}

func TestScanOnceSetsTheAlertBaselineAndClearsAcknowledgement(t *testing.T) {
	f := newFixture(t)
	id := f.upCheckPingedAt(t, 60, 30, utc(2026, 7, 12, 12, 0))
	now := utc(2026, 7, 12, 13, 0)
	scan(t, f, now)
	check, err := f.deps.Store.FindCheck(context.Background(), id)
	if err != nil || check.LastAlertAt == nil || !check.LastAlertAt.Equal(now) || check.Acknowledged {
		t.Fatalf("check = %+v, err = %v", check, err)
	}
}

func TestOverdueDownsAndDeliversToBoundChannel(t *testing.T) {
	var requests atomic.Int32
	hook := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		requests.Add(1)
	}))
	defer hook.Close()

	f := newFixture(t)
	id := f.upCheckPingedAt(t, 60, 30, time.Now().Add(-200*time.Second))
	ctx := context.Background()
	channelID, err := f.deps.Store.CreateChannel(ctx, store.NewChannel{
		ProjectID: f.projectID, Kind: models.ChannelWebhook, Name: "hook", ConfigJSON: `{"url":"` + hook.URL + `"}`,
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if err := f.deps.Store.BindChannel(ctx, id, channelID); err != nil {
		t.Fatal(err)
	}

	now := time.Now()
	got := scan(t, f, now)
	if len(got) != 1 || got[0].CheckID != id {
		t.Fatalf("events = %+v", got)
	}
	deliverer := notify.Deliverer{Store: f.deps.Store, Retry: notify.RetryPolicy{MaxAttempts: 1}}
	for _, event := range got {
		deliverer.DeliverEvent(ctx, event, now)
	}
	if requests.Load() != 1 {
		t.Fatalf("webhook requests = %d", requests.Load())
	}
	notifications, err := f.deps.Store.ListRecentNotifications(ctx, id, 10)
	if err != nil || len(notifications) != 1 || notifications[0].Status != models.NotifyOK {
		t.Fatalf("notifications = %+v, err = %v", notifications, err)
	}
}

func TestRunScanLoopPublishesDownTransitionToLiveTail(t *testing.T) {
	f := newFixture(t)
	id := f.upCheckPingedAt(t, 60, 30, time.Now().Add(-200*time.Second))
	f.deps.Events = events.NewBroker()
	// Subscribe before starting: publishing is gated on there being subscribers.
	feed, cancel := f.deps.Events.Subscribe()
	defer cancel()
	ctx, stop := context.WithCancel(context.Background())
	defer stop()
	go f.deps.RunScanLoop(ctx)

	select {
	case got := <-feed:
		if got != id {
			t.Fatalf("published %d, want %d", got, id)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("timed out waiting for the live-tail signal from RunScanLoop")
	}
}

func TestRunScanLoopWritesTheScanHeartbeat(t *testing.T) {
	f := newFixture(t)
	ctx, stop := context.WithCancel(context.Background())
	defer stop()
	go f.deps.RunScanLoop(ctx)
	waitFor(t, "last_scan_at", func() bool {
		value, _ := f.deps.Store.GetSetting(context.Background(), "last_scan_at")
		return value != nil
	})
}

// The caller joins the loop before closing the pool, so it must return.
func TestRunScanLoopReturnsOnShutdown(t *testing.T) {
	f := newFixture(t)
	f.deps.EnvDefaultScanSecs = 3600 // only the ctx select can end the sleep in time
	f.upCheckPingedAt(t, 60, 30, time.Now().Add(-200*time.Second))
	ctx, stop := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() { f.deps.RunScanLoop(ctx); close(done) }()
	waitFor(t, "first pass", func() bool {
		value, _ := f.deps.Store.GetSetting(context.Background(), "last_scan_at")
		return value != nil
	})
	stop()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("RunScanLoop must return promptly after shutdown")
	}
}

func TestRunScanLoopDeliversEachDownEvent(t *testing.T) {
	f := newFixture(t)
	id := f.upCheckPingedAt(t, 60, 30, time.Now().Add(-200*time.Second))
	delivered := make(chan notify.Event, 4)
	f.deps.Deliver = func(_ context.Context, event notify.Event, _ time.Time) { delivered <- event }
	ctx, stop := context.WithCancel(context.Background())
	defer stop()
	go f.deps.RunScanLoop(ctx)
	select {
	case event := <-delivered:
		if event.CheckID != id || event.Kind != models.EventDown {
			t.Fatalf("event = %+v", event)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("no delivery")
	}
}

func waitFor(t *testing.T, what string, done func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for !done() {
		if time.Now().After(deadline) {
			t.Fatalf("timed out waiting for %s", what)
		}
		time.Sleep(10 * time.Millisecond)
	}
}

func TestLoopIntervalPicksMinEffectiveAcrossChecks(t *testing.T) {
	// Check 1 in project 1: no project override, so its own 50 applies.
	one := baseCheck()
	one.ID, one.ProjectID, one.ScanIntervalSecs = 1, 1, int64Ptr(50)
	// Check 2 in project 2: no check override, so the project override 10 applies.
	two := baseCheck()
	two.ID, two.ProjectID = 2, 2
	intervals := map[int64]*int64{1: nil, 2: int64Ptr(10)}

	if got := loopIntervalSecs([]models.Check{one, two}, intervals, int64Ptr(30), 30); got != 10 {
		t.Fatalf("interval = %d, want 10", got)
	}
}

func TestLoopIntervalEmptyFallsBackToEnvDefault(t *testing.T) {
	if got := loopIntervalSecs(nil, nil, nil, 30); got != 30 {
		t.Fatalf("interval = %d, want 30", got)
	}
	// An env default of 0 is clamped to 1 so the timer stays valid.
	if got := loopIntervalSecs(nil, nil, nil, 0); got != 1 {
		t.Fatalf("interval = %d, want 1", got)
	}
}

func downCheck(t *testing.T) (*fixture, int64) {
	t.Helper()
	f := newFixture(t)
	id := f.createCheck(t, 3600, 300)
	if err := f.deps.Store.SetStatus(context.Background(), id, models.StatusDown); err != nil {
		t.Fatal(err)
	}
	return f, id
}

func setNagEvery60s(t *testing.T, f *fixture, id int64) {
	t.Helper()
	f.update(t, id, func(u *store.UpdateCheck) { u.NagIntervalSecs = int64Ptr(60) })
}

func TestNagOnceRemindsDueUnackedCheck(t *testing.T) {
	f, id := downCheck(t)
	t0 := utc(2026, 7, 12, 12, 0)
	setNagEvery60s(t, f, id)
	if err := f.deps.Store.BeginDownAlert(context.Background(), id, t0); err != nil {
		t.Fatal(err)
	}

	if got := nag(t, f, t0.Add(59*time.Second)); len(got) != 0 {
		t.Fatalf("not yet due: %+v", got)
	}
	got := nag(t, f, t0.Add(60*time.Second))
	if len(got) != 1 || got[0].Kind != models.EventReminder || got[0].CheckID != id {
		t.Fatalf("due: %+v", got)
	}
	// The baseline advanced, so right after it the check is not due again.
	if got := nag(t, f, t0.Add(61*time.Second)); len(got) != 0 {
		t.Fatalf("baseline must advance: %+v", got)
	}
}

func TestNagOnceSkipsAckedAndOffAndNoBaseline(t *testing.T) {
	f, id := downCheck(t)
	ctx := context.Background()
	t0 := utc(2026, 7, 12, 12, 0)
	later := t0.Add(time.Hour)

	// No baseline yet: skipped even with an interval configured.
	setNagEvery60s(t, f, id)
	if got := nag(t, f, later); len(got) != 0 {
		t.Fatalf("no baseline: %+v", got)
	}

	// Nag interval off everywhere: skipped.
	f.update(t, id, func(*store.UpdateCheck) {})
	if err := f.deps.Store.BeginDownAlert(ctx, id, t0); err != nil {
		t.Fatal(err)
	}
	if got := nag(t, f, later); len(got) != 0 {
		t.Fatalf("nag off: %+v", got)
	}

	// Interval configured but acknowledged: skipped.
	setNagEvery60s(t, f, id)
	if err := f.deps.Store.Acknowledge(ctx, id); err != nil {
		t.Fatal(err)
	}
	if got := nag(t, f, later); len(got) != 0 {
		t.Fatalf("acknowledged: %+v", got)
	}
}

func TestNagOnceFallsBackToProjectThenGlobalInterval(t *testing.T) {
	f, id := downCheck(t)
	ctx := context.Background()
	t0 := utc(2026, 7, 12, 12, 0)
	if err := f.deps.Store.BeginDownAlert(ctx, id, t0); err != nil {
		t.Fatal(err)
	}
	if err := f.deps.Store.SetSetting(ctx, "nag_interval", "120"); err != nil {
		t.Fatal(err)
	}
	if got := nag(t, f, t0.Add(119*time.Second)); len(got) != 0 {
		t.Fatalf("global not yet due: %+v", got)
	}
	if got := nag(t, f, t0.Add(120*time.Second)); len(got) != 1 {
		t.Fatalf("global due: %+v", got)
	}
	// A project override beats the global value.
	if err := f.deps.Store.UpdateProject(ctx, f.projectID, store.ProjectSettings{Name: "p", NagIntervalSecs: int64Ptr(10)}); err != nil {
		t.Fatal(err)
	}
	if got := nag(t, f, t0.Add(120*time.Second+10*time.Second)); len(got) != 1 {
		t.Fatalf("project due: %+v", got)
	}
}

func TestNagOnceIgnoresANonNumericGlobalInterval(t *testing.T) {
	f, id := downCheck(t)
	ctx := context.Background()
	t0 := utc(2026, 7, 12, 12, 0)
	if err := f.deps.Store.BeginDownAlert(ctx, id, t0); err != nil {
		t.Fatal(err)
	}
	if err := f.deps.Store.SetSetting(ctx, "nag_interval", "soon"); err != nil {
		t.Fatal(err)
	}
	if got := nag(t, f, t0.Add(24*time.Hour)); len(got) != 0 {
		t.Fatalf("events = %+v", got)
	}
}
