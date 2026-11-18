package prune

import (
	"context"
	"strconv"
	"sync/atomic"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/db"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

var now = time.Date(2026, 7, 13, 12, 0, 0, 0, time.UTC)

func ptr[T any](v T) *T { return &v }

type fixture struct {
	store     *store.Store
	checkID   int64
	channelID int64
	userID    int64
}

func newFixture(t *testing.T) fixture {
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
	checkID, err := st.CreateCheck(ctx, store.NewCheck{
		ProjectID: projectID, Name: "c", PingUUID: "uu", Kind: models.SchedulePeriod,
		PeriodSecs: ptr(int64(60)), GraceSecs: 30, Timezone: "UTC",
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	channelID, err := st.CreateChannel(ctx, store.NewChannel{
		ProjectID: projectID, Kind: models.ChannelWebhook, Name: "h", ConfigJSON: `{"url":"http://x"}`,
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return fixture{store: st, checkID: checkID, channelID: channelID, userID: userID}
}

func (f fixture) insertPing(t *testing.T, at time.Time) {
	t.Helper()
	err := f.store.InsertPing(context.Background(), store.NewPing{CheckID: f.checkID, Kind: models.PingSuccess}, at)
	if err != nil {
		t.Fatal(err)
	}
}

func (f fixture) pingCount(t *testing.T) int {
	t.Helper()
	pings, err := f.store.ListRecentPings(context.Background(), f.checkID, 10)
	if err != nil {
		t.Fatal(err)
	}
	return len(pings)
}

func setSetting(t *testing.T, s *store.Store, key, value string) {
	t.Helper()
	if err := s.SetSetting(context.Background(), key, value); err != nil {
		t.Fatal(err)
	}
}

func TestParseDaysOffAndPositiveCases(t *testing.T) {
	for _, off := range []*string{nil, ptr(""), ptr("   "), ptr("abc"), ptr("0"), ptr("-5")} {
		if days, ok := parseDays(off); ok {
			t.Errorf("parseDays(%v) = %d, want off", off, days)
		}
	}
	for text, want := range map[string]int64{"7": 7, "  30 ": 30} {
		if days, ok := parseDays(ptr(text)); !ok || days != want {
			t.Errorf("parseDays(%q) = %d, %v", text, days, ok)
		}
	}
}

func TestRetentionCutoffOffOverflowAndValid(t *testing.T) {
	if _, ok := retentionCutoff(now, nil); ok {
		t.Error("unset must be off")
	}
	if _, ok := retentionCutoff(now, ptr("0")); ok {
		t.Error("0 must be off")
	}
	cutoff, ok := retentionCutoff(now, ptr("7"))
	if !ok || cutoff != db.FormatTime(now.AddDate(0, 0, -7)) {
		t.Errorf("cutoff = %q, %v", cutoff, ok)
	}
	if cutoff != "2026-07-06T12:00:00+00:00" {
		t.Errorf("cutoff is not chrono to_rfc3339 text: %q", cutoff)
	}
	// An absurd value must not wrap around; the overflow branch fails safe.
	if _, ok := retentionCutoff(now, ptr("999999999999999999")); ok {
		t.Error("overflow must be off")
	}
}

func TestPruneOnceDeletesOldWhenRetentionSet(t *testing.T) {
	f := newFixture(t)
	ctx := context.Background()
	f.insertPing(t, now.AddDate(0, 0, -10))
	f.insertPing(t, now.AddDate(0, 0, -1))
	err := f.store.RecordNotification(ctx, store.NewNotification{
		CheckID: f.checkID, ChannelID: f.channelID, Event: models.EventDown, Status: models.NotifyOK,
	}, now.AddDate(0, 0, -10))
	if err != nil {
		t.Fatal(err)
	}
	setSetting(t, f.store, "pings_retention_days", "7")
	setSetting(t, f.store, "notifications_retention_days", "7")

	counts, err := Once(ctx, f.store, now)
	if err != nil {
		t.Fatal(err)
	}
	if counts != (Counts{Pings: 1, Notifications: 1}) {
		t.Fatalf("counts = %+v", counts)
	}
	if f.pingCount(t) != 1 {
		t.Fatal("the recent ping must stay")
	}
}

func TestPruneOnceAuditRetentionIsOffByDefaultAndDeletesWhenSet(t *testing.T) {
	f := newFixture(t)
	ctx := context.Background()
	for name, at := range map[string]time.Time{"old": now.AddDate(0, 0, -100), "recent": now} {
		_, err := f.store.RecordAudit(ctx, store.NewAudit{
			ActorUserID: f.userID, ActorUsername: "adm", Action: "admin.access", Detail: ptr(name),
		}, at)
		if err != nil {
			t.Fatal(err)
		}
	}
	auditRows := func() []models.AuditLog {
		rows, err := f.store.ListAudit(ctx, 10)
		if err != nil {
			t.Fatal(err)
		}
		return rows
	}
	prune := func() int64 {
		counts, err := Once(ctx, f.store, now)
		if err != nil {
			t.Fatal(err)
		}
		return counts.Audit
	}

	if prune() != 0 || len(auditRows()) != 2 {
		t.Fatal("unset must be off")
	}
	setSetting(t, f.store, "audit_retention_days", "0")
	if prune() != 0 || len(auditRows()) != 2 {
		t.Fatal("explicit 0 must be off")
	}
	setSetting(t, f.store, "audit_retention_days", "30")
	if prune() != 1 {
		t.Fatal("the old row must go")
	}
	left := auditRows()
	if len(left) != 1 || left[0].Detail == nil || *left[0].Detail != "recent" {
		t.Fatalf("left = %+v", left)
	}
}

func TestPruneOnceOffWhenUnsetOrZero(t *testing.T) {
	f := newFixture(t)
	ctx := context.Background()
	f.insertPing(t, now.AddDate(0, 0, -100))

	counts, err := Once(ctx, f.store, now)
	if err != nil || !counts.empty() {
		t.Fatalf("unset: %+v, %v", counts, err)
	}
	setSetting(t, f.store, "pings_retention_days", "0")
	counts, err = Once(ctx, f.store, now)
	if err != nil || !counts.empty() {
		t.Fatalf("zero: %+v, %v", counts, err)
	}
	if f.pingCount(t) != 1 {
		t.Fatal("the ping must stay")
	}
}

func TestPruneOnceDeletesExpiredSessions(t *testing.T) {
	f := newFixture(t)
	ctx := context.Background()
	for _, session := range []struct {
		id      string
		expires time.Time
		created time.Time
	}{
		{"sess-expired", now.Add(-time.Hour), now.Add(-2 * time.Hour)},
		{"sess-valid", now.Add(time.Hour), now},
	} {
		err := f.store.CreateSession(ctx, store.NewSession{
			ID: session.id, UserID: f.userID, ExpiresAt: session.expires,
		}, session.created)
		if err != nil {
			t.Fatal(err)
		}
	}

	counts, err := Once(ctx, f.store, now)
	if err != nil {
		t.Fatal(err)
	}
	if counts != (Counts{Sessions: 1}) {
		t.Fatalf("counts = %+v", counts)
	}
	remaining, err := f.store.ListSessionsForUser(ctx, f.userID, now)
	if err != nil || len(remaining) != 1 || remaining[0].ID != "sess-valid" {
		t.Fatalf("remaining = %+v, %v", remaining, err)
	}
}

func newLoop(t *testing.T, f fixture) (Loop, *testutil.LockedBuffer) {
	logger, logs := testutil.JSONLogger()
	return Loop{Store: f.store, Logger: logger, IntervalSecs: 3600, Now: time.Now}, logs
}

func TestRunLoopWritesHeartbeatAndReturnsOnShutdown(t *testing.T) {
	f := newFixture(t)
	loop, _ := newLoop(t, f)
	ctx, stop := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() { loop.Run(ctx); close(done) }()
	// One hour: only the ctx select can end the sleep in time.
	deadline := time.After(5 * time.Second)
	for {
		heartbeat, _ := f.store.GetSetting(context.Background(), "last_prune_at")
		if heartbeat != nil {
			break
		}
		select {
		case <-deadline:
			t.Fatal("no last_prune_at heartbeat")
		case <-time.After(10 * time.Millisecond):
		}
	}
	stop()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("Run must return promptly after shutdown")
	}
}

// Control for the test above.
func TestRunLoopKeepsRunningWithoutShutdown(t *testing.T) {
	f := newFixture(t)
	loop, _ := newLoop(t, f)
	ctx, stop := context.WithCancel(context.Background())
	defer stop()
	done := make(chan struct{})
	go func() { loop.Run(ctx); close(done) }()
	select {
	case <-done:
		t.Fatal("Run must not exit while ctx is live")
	case <-time.After(300 * time.Millisecond):
	}
}

func TestRunLoopLogsWhatItPrunedAndTheExpiredSessions(t *testing.T) {
	f := newFixture(t)
	f.insertPing(t, time.Now().AddDate(0, 0, -10))
	setSetting(t, f.store, "pings_retention_days", "7")
	err := f.store.CreateSession(context.Background(), store.NewSession{
		ID: "gone", UserID: f.userID, ExpiresAt: time.Now().Add(-time.Hour),
	}, time.Now().Add(-2*time.Hour))
	if err != nil {
		t.Fatal(err)
	}
	loop, logs := newLoop(t, f)
	loop.pass(context.Background())

	pruned := logs.EventsNamed(t, "pruned 1 pings, 0 notifications, 0 audit entries, 1 sessions")
	if len(pruned) != 1 {
		t.Fatalf("logs: %s", logs.String())
	}
	destroyed := logs.EventsNamed(t, "session.destroyed")
	if len(destroyed) != 1 || destroyed[0]["reason"] != "expired" || destroyed[0]["count"] != float64(1) {
		t.Fatalf("destroyed = %v", destroyed)
	}
}

func TestHeartbeatSettingWrites(t *testing.T) {
	f := newFixture(t)
	loop, _ := newLoop(t, f)
	loop.pass(context.Background())
	if value, err := f.store.GetSetting(context.Background(), "last_prune_at"); err != nil || value == nil {
		t.Fatalf("value = %v, err = %v", value, err)
	}
}

func TestRetentionCutoffAcceptsTheLargestRangeAndRefusesOnePast(t *testing.T) {
	largest := (now.Unix() - minCutoff.Unix()) / 86400
	if _, ok := retentionCutoff(now, ptr(strconv.FormatInt(largest, 10))); !ok {
		t.Errorf("%d days reaches the earliest representable date and must be accepted", largest)
	}
	if _, ok := retentionCutoff(now, ptr(strconv.FormatInt(largest+1, 10))); ok {
		t.Errorf("%d days goes past the earliest representable date and must be refused", largest+1)
	}
}

func TestRunLoopWaitsForTheIntervalBetweenPasses(t *testing.T) {
	f := newFixture(t)
	loop, _ := newLoop(t, f)
	var clockReads atomic.Int64
	loop.Now = func() time.Time {
		clockReads.Add(1)
		return time.Now()
	}
	ctx, stop := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() { loop.Run(ctx); close(done) }()
	time.Sleep(300 * time.Millisecond)
	stop()
	<-done
	// One pass reads the clock a couple of times; a loop that does not wait
	// would pass hundreds of times.
	if reads := clockReads.Load(); reads > 10 {
		t.Fatalf("the clock was read %d times in 300ms: the loop is not waiting for its interval", reads)
	}
}

func TestRunLoopOmitsTheSessionLineWhenNoSessionExpired(t *testing.T) {
	f := newFixture(t)
	f.insertPing(t, time.Now().AddDate(0, 0, -10))
	setSetting(t, f.store, "pings_retention_days", "7")
	loop, logs := newLoop(t, f)
	loop.pass(context.Background())
	if len(logs.EventsNamed(t, "pruned 1 pings, 0 notifications, 0 audit entries, 0 sessions")) != 1 {
		t.Fatalf("logs: %s", logs.String())
	}
	if destroyed := logs.EventsNamed(t, "session.destroyed"); len(destroyed) != 0 {
		t.Fatalf("no session expired, yet %v was logged", destroyed)
	}
}
