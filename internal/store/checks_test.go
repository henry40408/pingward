package store_test

import (
	"reflect"
	"sort"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/db"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

func i64(v int64) *int64 { return &v }

// seeded is a store holding user 1 ("u") and its project 1 ("p").
func seeded(t *testing.T) *store.Store {
	t.Helper()
	s := testutil.NewStore(t)
	if _, err := s.DB.ExecContext(ctx,
		"INSERT INTO users (username, is_admin, created_at) VALUES ('u', 0, $1)", db.FormatTime(t0)); err != nil {
		t.Fatal(err)
	}
	if _, err := s.DB.ExecContext(ctx,
		"INSERT INTO projects (user_id, name, created_at) VALUES (1, 'p', $1)", db.FormatTime(t0)); err != nil {
		t.Fatal(err)
	}
	return s
}

func periodCheck(projectID int64, uuid string) store.NewCheck {
	return store.NewCheck{
		ProjectID: projectID, Name: "job", PingUUID: uuid, Kind: models.SchedulePeriod,
		PeriodSecs: i64(60), GraceSecs: 30, Timezone: "UTC",
	}
}

func mustCheck(t *testing.T, s *store.Store, check store.NewCheck) int64 {
	t.Helper()
	id, err := s.CreateCheck(ctx, check, t0)
	if err != nil {
		t.Fatal(err)
	}
	return id
}

func mustProject(t *testing.T, s *store.Store, name string, scanInterval *int64) int64 {
	t.Helper()
	id, err := s.CreateProject(ctx, 1, store.ProjectSettings{Name: name, ScanIntervalSecs: scanInterval}, t0)
	if err != nil {
		t.Fatal(err)
	}
	return id
}

func mustChannel(t *testing.T, s *store.Store, projectID int64, name string) int64 {
	t.Helper()
	id, err := s.CreateChannel(ctx, store.NewChannel{
		ProjectID: projectID, Kind: models.ChannelWebhook, Name: name, ConfigJSON: "{}",
	}, t0)
	if err != nil {
		t.Fatal(err)
	}
	return id
}

func mustCheckFound(t *testing.T, s *store.Store, id int64) *models.Check {
	t.Helper()
	check, err := s.FindCheck(ctx, id)
	if err != nil || check == nil {
		t.Fatalf("find check %d: %v %v", id, check, err)
	}
	return check
}

func sortedIDs(ids []int64) []int64 {
	out := append([]int64{}, ids...)
	sort.Slice(out, func(i, j int) bool { return out[i] < out[j] })
	return out
}

func TestFindByUUIDRoundtrip(t *testing.T) {
	s := seeded(t)
	id := mustCheck(t, s, periodCheck(1, "uuid-1"))
	found, err := s.FindCheckByUUID(ctx, "uuid-1")
	if err != nil || found == nil || found.ID != id || found.Status != models.StatusNew {
		t.Fatalf("%+v %v", found, err)
	}
	if missing, err := s.FindCheckByUUID(ctx, "nope"); err != nil || missing != nil {
		t.Fatalf("%+v %v", missing, err)
	}
}

func TestProjectAndCheckDescriptionPersistAndUpdate(t *testing.T) {
	s := seeded(t)
	pid, err := s.CreateProject(ctx, 1, store.ProjectSettings{Name: "described", Description: "hello **world**"}, t0)
	if err != nil {
		t.Fatal(err)
	}
	project, _ := s.FindProject(ctx, pid)
	if project.Description != "hello **world**" {
		t.Fatalf("%q", project.Description)
	}
	if err := s.UpdateProject(ctx, pid, store.ProjectSettings{Name: "described", Description: "updated *desc*"}); err != nil {
		t.Fatal(err)
	}
	project, _ = s.FindProject(ctx, pid)
	if project.Description != "updated *desc*" {
		t.Fatalf("%q", project.Description)
	}

	newCheck := periodCheck(pid, "uuid-desc")
	newCheck.Description = "runs `nightly`"
	cid := mustCheck(t, s, newCheck)
	if got := mustCheckFound(t, s, cid).Description; got != "runs `nightly`" {
		t.Fatalf("%q", got)
	}
	err = s.UpdateCheckSchedule(ctx, cid, store.UpdateCheck{
		Name: "job", Description: "updated check desc", Kind: models.SchedulePeriod,
		PeriodSecs: i64(60), GraceSecs: 30, Timezone: "UTC",
	})
	if err != nil {
		t.Fatal(err)
	}
	if got := mustCheckFound(t, s, cid).Description; got != "updated check desc" {
		t.Fatalf("%q", got)
	}
}

func TestInsertPingAndListActive(t *testing.T) {
	s := seeded(t)
	id := mustCheck(t, s, periodCheck(1, "u"))
	exit, source := int64(0), "1.2.3.4"
	err := s.InsertPing(ctx, store.NewPing{
		CheckID: id, Kind: models.PingSuccess, ExitCode: &exit, Body: "hello", SourceIP: &source,
	}, time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC))
	if err != nil {
		t.Fatal(err)
	}
	pings, err := s.ListRecentPings(ctx, id, 10)
	if err != nil || len(pings) != 1 {
		t.Fatalf("%v %v", pings, err)
	}
	ping := pings[0]
	if ping.Kind != models.PingSuccess || ping.ExitCode == nil || *ping.ExitCode != 0 ||
		ping.Body != "hello" || ping.SourceIP == nil || *ping.SourceIP != "1.2.3.4" {
		t.Fatalf("%+v", ping)
	}
	if active, _ := s.ListActiveChecks(ctx); len(active) != 1 {
		t.Fatalf("active %d", len(active))
	}
	if err := s.SetStatus(ctx, id, models.StatusPaused); err != nil {
		t.Fatal(err)
	}
	if active, _ := s.ListActiveChecks(ctx); len(active) != 0 {
		t.Fatalf("active after pause %d", len(active))
	}
}

func TestListActiveChecksIncludesUpStatus(t *testing.T) {
	s := seeded(t)
	id := mustCheck(t, s, periodCheck(1, "up-uuid"))
	if err := s.SetStatus(ctx, id, models.StatusUp); err != nil {
		t.Fatal(err)
	}
	active, err := s.ListActiveChecks(ctx)
	if err != nil || len(active) != 1 || active[0].Status != models.StatusUp {
		t.Fatalf("%+v %v", active, err)
	}
}

func TestListActiveChecksSkipsACorruptRow(t *testing.T) {
	s := seeded(t)
	mustCheck(t, s, periodCheck(1, "good"))
	if _, err := s.DB.ExecContext(ctx,
		"INSERT INTO checks (project_id, name, ping_uuid, schedule_kind, status, created_at) "+
			"VALUES (1, 'bad', 'bad', 'period', 'up', 'not a time')"); err != nil {
		t.Fatal(err)
	}
	active, err := s.ListActiveChecks(ctx)
	if err != nil || len(active) != 1 || active[0].PingUUID != "good" {
		t.Fatalf("%+v %v", active, err)
	}
}

func TestMarkPingUpdatesStatusAndCoalescesTimestamps(t *testing.T) {
	s := seeded(t)
	id := mustCheck(t, s, periodCheck(1, "mark-uuid"))
	ping := time.Date(2026, 1, 1, 10, 0, 0, 0, time.UTC)
	start := time.Date(2026, 1, 1, 9, 59, 0, 0, time.UTC)
	due := time.Date(2026, 1, 1, 11, 0, 0, 0, time.UTC)
	err := s.MarkPing(ctx, id, store.PingMark{Status: models.StatusUp, LastPingAt: &ping, LastStartAt: &start, NextDueAt: &due})
	if err != nil {
		t.Fatal(err)
	}
	found := mustCheckFound(t, s, id)
	if found.Status != models.StatusUp || !found.LastPingAt.Equal(ping) || !found.LastStartAt.Equal(start) || !found.NextDueAt.Equal(due) {
		t.Fatalf("%+v", found)
	}

	// COALESCE keeps the timestamps, but next_due_at is overwritten to NULL.
	if err := s.MarkPing(ctx, id, store.PingMark{Status: models.StatusUp}); err != nil {
		t.Fatal(err)
	}
	found = mustCheckFound(t, s, id)
	if found.Status != models.StatusUp || !found.LastPingAt.Equal(ping) || !found.LastStartAt.Equal(start) || found.NextDueAt != nil {
		t.Fatalf("%+v", found)
	}
}

func TestBadStatusIsRejectedByCheckConstraint(t *testing.T) {
	s := seeded(t)
	_, err := s.DB.ExecContext(ctx,
		"INSERT INTO checks (project_id, name, ping_uuid, schedule_kind, status, created_at) "+
			"VALUES (1, 'x', 'bad-status-uuid', 'period', 'bogus', $1)", db.FormatTime(t0))
	if err == nil {
		t.Fatal("expected CHECK constraint to reject bad status")
	}
}

func TestProjectChannelBindingAndSettings(t *testing.T) {
	s := seeded(t)
	pid := mustProject(t, s, "web", i64(15))
	if projects, _ := s.ListProjectsForUser(ctx, 1); len(projects) != 2 {
		t.Fatalf("projects %d", len(projects))
	}
	if project, _ := s.FindProject(ctx, pid); project.ScanIntervalSecs == nil || *project.ScanIntervalSecs != 15 {
		t.Fatalf("%+v", project)
	}

	cid, err := s.CreateChannel(ctx, store.NewChannel{
		ProjectID: pid, Kind: models.ChannelWebhook, Name: "hook", ConfigJSON: `{"url":"http://x"}`,
	}, t0)
	if err != nil {
		t.Fatal(err)
	}
	if channels, _ := s.ListChannelsForProject(ctx, pid); len(channels) != 1 {
		t.Fatalf("channels %d", len(channels))
	}

	chk := mustCheck(t, s, periodCheck(pid, "uuid-x"))
	if err := s.BindChannel(ctx, chk, cid); err != nil {
		t.Fatal(err)
	}
	if bound, _ := s.BoundChannelIDs(ctx, chk); !reflect.DeepEqual(bound, []int64{cid}) {
		t.Fatalf("%v", bound)
	}
	if channels, _ := s.ChannelsForCheck(ctx, chk); len(channels) != 1 {
		t.Fatalf("channels %d", len(channels))
	}
	if err := s.UnbindChannel(ctx, chk, cid); err != nil {
		t.Fatal(err)
	}
	if bound, _ := s.BoundChannelIDs(ctx, chk); len(bound) != 0 {
		t.Fatalf("%v", bound)
	}

	err = s.RecordNotification(ctx, store.NewNotification{
		CheckID: chk, ChannelID: cid, Event: models.EventDown, Status: models.NotifyOK,
	}, t0)
	if err != nil {
		t.Fatal(err)
	}
	if notifications, _ := s.ListRecentNotifications(ctx, chk, 10); len(notifications) != 1 {
		t.Fatalf("notifications %d", len(notifications))
	}

	if value, _ := s.GetSetting(ctx, "scan_interval"); value != nil {
		t.Fatalf("%v", *value)
	}
	for _, want := range []string{"45", "60"} { // the second write is an upsert
		if err := s.SetSetting(ctx, "scan_interval", want); err != nil {
			t.Fatal(err)
		}
		if got, _ := s.GetSetting(ctx, "scan_interval"); got == nil || *got != want {
			t.Fatalf("got %v want %s", got, want)
		}
	}

	intervals, err := s.AllProjectScanIntervals(ctx)
	if err != nil || intervals[pid] == nil || *intervals[pid] != 15 {
		t.Fatalf("%v %v", intervals, err)
	}
}

func TestBindAllProjectChannelsBindsEveryProjectChannel(t *testing.T) {
	s := seeded(t)
	pid := mustProject(t, s, "web", nil)
	otherPid := mustProject(t, s, "other", nil)
	c1 := mustChannel(t, s, pid, "hook1")
	c2 := mustChannel(t, s, pid, "hook2")
	mustChannel(t, s, otherPid, "hook-other")
	chk := mustCheck(t, s, periodCheck(pid, "uuid-bind-all"))

	want := sortedIDs([]int64{c1, c2})
	for _, round := range []string{"first call", "second call must be a no-op"} {
		if err := s.BindAllProjectChannels(ctx, chk, pid); err != nil {
			t.Fatalf("%s: %v", round, err)
		}
		bound, _ := s.BoundChannelIDs(ctx, chk)
		if !reflect.DeepEqual(sortedIDs(bound), want) {
			t.Fatalf("%s: bound %v want %v", round, bound, want)
		}
	}
}

func TestChecksWithChannelsReportsOnlyBoundChecks(t *testing.T) {
	s := seeded(t)
	pid := mustProject(t, s, "web", nil)
	cid := mustChannel(t, s, pid, "hook")
	withChannel := mustCheck(t, s, periodCheck(pid, "uuid-with-channel"))
	withoutChannel := mustCheck(t, s, periodCheck(pid, "uuid-without-channel"))
	if err := s.BindChannel(ctx, withChannel, cid); err != nil {
		t.Fatal(err)
	}
	if empty, err := s.ChecksWithChannels(ctx, nil); err != nil || len(empty) != 0 {
		t.Fatalf("%v %v", empty, err)
	}
	result, err := s.ChecksWithChannels(ctx, []int64{withChannel, withoutChannel})
	if err != nil || len(result) != 1 || !result[withChannel] {
		t.Fatalf("%v %v", result, err)
	}
}

func TestNewCheckHasNagDefaults(t *testing.T) {
	s := seeded(t)
	check := mustCheckFound(t, s, mustCheck(t, s, periodCheck(1, "uu")))
	if check.NagIntervalSecs != nil || check.LastAlertAt != nil || check.Acknowledged {
		t.Fatalf("%+v", check)
	}
}

func TestNagStateMethodsRoundtrip(t *testing.T) {
	s := seeded(t)
	id := mustCheck(t, s, periodCheck(1, "uu"))
	if err := s.SetStatus(ctx, id, models.StatusDown); err != nil {
		t.Fatal(err)
	}
	if down, _ := s.ListDownChecks(ctx); len(down) != 1 || down[0].ID != id {
		t.Fatalf("%+v", down)
	}

	first := time.Date(2026, 7, 12, 12, 0, 0, 0, time.UTC)
	if err := s.BeginDownAlert(ctx, id, first); err != nil {
		t.Fatal(err)
	}
	check := mustCheckFound(t, s, id)
	if check.LastAlertAt == nil || !check.LastAlertAt.Equal(first) || check.Acknowledged {
		t.Fatalf("%+v", check)
	}
	if err := s.Acknowledge(ctx, id); err != nil {
		t.Fatal(err)
	}
	if !mustCheckFound(t, s, id).Acknowledged {
		t.Fatal("not acknowledged")
	}
	second := first.Add(90 * time.Second)
	if err := s.RecordReminder(ctx, id, second); err != nil {
		t.Fatal(err)
	}
	if got := mustCheckFound(t, s, id).LastAlertAt; got == nil || !got.Equal(second) {
		t.Fatalf("%v", got)
	}
	if err := s.ClearNag(ctx, id); err != nil {
		t.Fatal(err)
	}
	check = mustCheckFound(t, s, id)
	if check.LastAlertAt != nil || check.Acknowledged {
		t.Fatalf("%+v", check)
	}
	if intervals, _ := s.AllProjectNagIntervals(ctx); intervals[1] != nil || len(intervals) != 1 {
		t.Fatalf("%v", intervals)
	}
}

func TestDeleteBeforeRemovesOnlyOldRows(t *testing.T) {
	s := seeded(t)
	cid := mustCheck(t, s, periodCheck(1, "uu"))
	chann := mustChannel(t, s, 1, "h")
	now := time.Date(2026, 7, 13, 12, 0, 0, 0, time.UTC)
	old, recent := now.Add(-10*24*time.Hour), now.Add(-24*time.Hour)
	for _, at := range []time.Time{old, recent} {
		if err := s.InsertPing(ctx, store.NewPing{CheckID: cid, Kind: models.PingSuccess}, at); err != nil {
			t.Fatal(err)
		}
	}
	for event, at := range map[models.EventKind]time.Time{models.EventDown: old, models.EventUp: recent} {
		err := s.RecordNotification(ctx, store.NewNotification{
			CheckID: cid, ChannelID: chann, Event: event, Status: models.NotifyOK,
		}, at)
		if err != nil {
			t.Fatal(err)
		}
	}
	cutoff := db.FormatTime(now.Add(-7 * 24 * time.Hour))
	if n, err := s.DeletePingsBefore(ctx, cutoff); err != nil || n != 1 {
		t.Fatalf("%d %v", n, err)
	}
	if n, err := s.DeleteNotificationsBefore(ctx, cutoff); err != nil || n != 1 {
		t.Fatalf("%d %v", n, err)
	}
	if pings, _ := s.ListRecentPings(ctx, cid, 10); len(pings) != 1 {
		t.Fatalf("pings %d", len(pings))
	}
	if notifications, _ := s.ListRecentNotifications(ctx, cid, 10); len(notifications) != 1 {
		t.Fatalf("notifications %d", len(notifications))
	}
	far := db.FormatTime(now.Add(-365 * 24 * time.Hour))
	if n, _ := s.DeletePingsBefore(ctx, far); n != 0 {
		t.Fatalf("far cutoff deleted %d", n)
	}
}
