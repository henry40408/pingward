package store_test

import (
	"errors"
	"reflect"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/db"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

// Postgres-only paths: the generated IN ($1,...) lists and the ROW_NUMBER() batch.
func TestPostgresEntityRoundTrip(t *testing.T) {
	s := testutil.NewPostgresStore(t)
	_, err := s.CreateUser(ctx, "alice", str("phc"), true, t0)
	if err != nil {
		t.Fatal(err)
	}
	pid := mustProject(t, s, "web", nil)
	cid := mustCheck(t, s, periodCheck(pid, "uuid-1"))
	chid := mustChannel(t, s, pid, "hook")
	if err := s.BindChannel(ctx, cid, chid); err != nil {
		t.Fatal(err)
	}
	if bound, err := s.BoundChannelIDs(ctx, cid); err != nil || !reflect.DeepEqual(bound, []int64{chid}) {
		t.Fatalf("%v %v", bound, err)
	}
	if active, err := s.ListActiveChecks(ctx); err != nil || len(active) != 1 {
		t.Fatalf("%v %v", active, err)
	}

	cid2 := mustCheck(t, s, periodCheck(pid, "uuid-2"))
	for i := 0; i < 3; i++ {
		err := s.InsertPing(ctx, store.NewPing{CheckID: cid2, Kind: models.PingSuccess, Body: "p"}, pageBase.Add(time.Duration(i)*time.Second))
		if err != nil {
			t.Fatal(err)
		}
	}
	batch, err := s.ListRecentPingSummariesForChecks(ctx, []int64{cid, cid2}, 2)
	if err != nil || len(batch[cid2]) != 2 || len(batch[cid]) != 0 {
		t.Fatalf("%v %v", batch, err)
	}
	empty := mustProject(t, s, "proj2", nil)
	byProject, err := s.ListChecksForProjects(ctx, []int64{pid, empty})
	if _, present := byProject[empty]; err != nil || present || len(byProject[pid]) != 2 {
		t.Fatalf("%v %v", byProject, err)
	}
}

// Postgres-only: timestamps are TEXT, so sliding, the absolute cap and the
// expiry boundary all rest on string comparisons that SQLite does not exercise.
func TestPostgresSessionSlidingAndAbsoluteCap(t *testing.T) {
	s := testutil.NewPostgresStore(t)
	uid := mustUser(t, s, "u")

	newSession(t, s, "fresh", uid, t0, t0.Add(70*time.Hour))
	newSession(t, s, "stale", uid, t0, t0.Add(time.Hour))
	for _, id := range []string{"fresh", "stale"} {
		if u, err := s.FindSessionUser(ctx, id, t0); err != nil || u == nil || u.ID != uid {
			t.Fatalf("%s: %v %v", id, u, err)
		}
	}
	if got, want := storedExpiry(t, s, "fresh"), db.FormatTime(t0.Add(70*time.Hour)); got != want {
		t.Fatalf("a fresh session must not slide: %s != %s", got, want)
	}
	if got, want := storedExpiry(t, s, "stale"), db.FormatTime(t0.Add(72*time.Hour)); got != want {
		t.Fatalf("a stale session must slide: %s != %s", got, want)
	}

	_, _ = s.FindSessionUser(ctx, "fresh", t0.Add(30*time.Second))
	list, _ := s.ListSessionsForUser(ctx, uid, t0)
	for _, session := range list {
		if session.ID == "fresh" && !session.LastSeenAt.Equal(t0) {
			t.Fatalf("last seen must be throttled: %v", session.LastSeenAt)
		}
	}

	newSession(t, s, "idle", uid, t0, t0.Add(time.Hour))
	requireNoSession(t, s, "idle", t0.Add(time.Hour))

	later := t0.Add(40 * 24 * time.Hour)
	newSession(t, s, "capped", uid, t0, later.Add(time.Hour))
	requireNoSession(t, s, "capped", later)
	if n, err := s.DeleteExpiredSessions(ctx, later); err != nil || n != 4 {
		t.Fatalf("deleted %d, %v", n, err)
	}
}

// The Rust suite kept every Postgres assertion in one test because they share
// a schema; this is that test. Entities, then the session lifecycle.
func TestPostgresFullRoundTrip(t *testing.T) {
	s := testutil.NewPostgresStore(t)
	uid, err := s.CreateUser(ctx, "alice", str("phc"), true, t0)
	if err != nil {
		t.Fatal(err)
	}
	// Postgres's unique-violation code (23505) differs from SQLite's.
	if _, err := s.CreateUser(ctx, "alice", str("other"), false, t0); !errors.Is(err, store.ErrUsernameTaken) {
		t.Fatalf("expected ErrUsernameTaken on postgres, got %v", err)
	}
	if user, err := s.FindUserByUsername(ctx, "alice"); err != nil || user == nil || !user.IsAdmin {
		t.Fatalf("%+v %v", user, err)
	}
	pid := mustProject(t, s, "web", i64(45))
	if projects, err := s.ListProjectsForUser(ctx, uid); err != nil || len(projects) != 1 {
		t.Fatalf("%v %v", projects, err)
	}
	cid := mustCheck(t, s, periodCheck(pid, "uuid-1"))
	if checks, err := s.ListChecksForProject(ctx, pid); err != nil || len(checks) != 1 {
		t.Fatalf("%v %v", checks, err)
	}
	chid := mustChannel(t, s, pid, "hook")
	if err := s.BindChannel(ctx, cid, chid); err != nil {
		t.Fatal(err)
	}
	if channels, err := s.ChannelsForCheck(ctx, cid); err != nil || len(channels) != 1 {
		t.Fatalf("%v %v", channels, err)
	}
	if err := s.UpdateChannel(ctx, chid, "renamed", `{"url":"http://y"}`); err != nil {
		t.Fatal(err)
	}
	edited, err := s.FindChannel(ctx, chid)
	if err != nil || edited == nil || edited.Name != "renamed" || edited.ConfigJSON != `{"url":"http://y"}` || edited.Kind != models.ChannelWebhook {
		t.Fatalf("update must not touch the immutable kind: %+v %v", edited, err)
	}

	ip := "127.0.0.1"
	if err := s.InsertPing(ctx, store.NewPing{CheckID: cid, Kind: models.PingSuccess, Body: "ok", SourceIP: &ip}, t0); err != nil {
		t.Fatal(err)
	}
	if err := s.MarkPing(ctx, cid, store.PingMark{Status: models.StatusUp, LastPingAt: &t0}); err != nil {
		t.Fatal(err)
	}
	cid2 := mustCheck(t, s, periodCheck(pid, "uuid-2"))
	for i := 0; i < 3; i++ {
		if err := s.InsertPing(ctx, store.NewPing{CheckID: cid2, Kind: models.PingSuccess, Body: "p"}, t0.Add(time.Duration(i)*time.Second)); err != nil {
			t.Fatal(err)
		}
	}
	batch, err := s.ListRecentPingSummariesForChecks(ctx, []int64{cid, cid2}, 2)
	if err != nil || len(batch[cid]) != 1 || len(batch[cid2]) != 2 {
		t.Fatalf("the per-check limit must hold: %v %v", batch, err)
	}
	perCheck, err := s.ListRecentPings(ctx, cid2, 2)
	if err != nil {
		t.Fatal(err)
	}
	for i, summary := range batch[cid2] {
		if summary.ID != perCheck[i].ID {
			t.Fatalf("batch order must match the per-check query: %v vs %v", batch[cid2], perCheck)
		}
	}

	if err := s.RecordNotification(ctx, store.NewNotification{CheckID: cid, ChannelID: chid, Event: models.EventDown, Status: models.NotifyOK}, t0); err != nil {
		t.Fatal(err)
	}
	if notifications, err := s.ListRecentNotifications(ctx, cid, 10); err != nil || len(notifications) != 1 {
		t.Fatalf("%v %v", notifications, err)
	}
	if err := s.SetSetting(ctx, "scan_interval", "45"); err != nil {
		t.Fatal(err)
	}
	if value, err := s.GetSetting(ctx, "scan_interval"); err != nil || value == nil || *value != "45" {
		t.Fatalf("%v %v", value, err)
	}

	postgresSessionLifecycle(t, s, uid)
}

func postgresSessionLifecycle(t *testing.T, s *store.Store, uid int64) {
	t.Helper()
	now := t0
	agent, ip := "curl/8.0", "127.0.0.1"
	create := func(id string, expires, created time.Time) {
		t.Helper()
		if err := s.CreateSession(ctx, store.NewSession{ID: id, UserID: uid, ExpiresAt: expires}, created); err != nil {
			t.Fatal(err)
		}
	}
	if err := s.CreateSession(ctx, store.NewSession{ID: "sess-active", UserID: uid, ExpiresAt: now.Add(time.Hour), UserAgent: &agent, IP: &ip}, now); err != nil {
		t.Fatal(err)
	}
	if user, err := s.FindSessionUser(ctx, "sess-active", now); err != nil || user == nil || user.Username != "alice" {
		t.Fatalf("%+v %v", user, err)
	}
	create("sess-expired", now.Add(-time.Hour), now)
	requireNoSession(t, s, "sess-expired", now)

	sessions, err := s.ListSessionsForUser(ctx, uid, now)
	if err != nil || len(sessions) != 1 || sessions[0].ID != "sess-active" ||
		sessions[0].UserAgent == nil || *sessions[0].UserAgent != agent ||
		sessions[0].IP == nil || *sessions[0].IP != ip ||
		sessions[0].LastSeenAt == nil || !sessions[0].LastSeenAt.Equal(now) {
		t.Fatalf("last_seen_at is stamped by the lookup: %+v %v", sessions, err)
	}

	// Past the absolute cap but inside its idle window.
	create("sess-abscap", now.Add(time.Hour), now.Add(-40*24*time.Hour))
	requireNoSession(t, s, "sess-abscap", now)
	list, _ := s.ListSessionsForUser(ctx, uid, now)
	for _, session := range list {
		if session.ID == "sess-abscap" {
			t.Fatal("a session past its absolute cap must not be listed")
		}
	}
	if err := s.DeleteSession(ctx, "sess-abscap"); err != nil {
		t.Fatal(err)
	}

	create("sess-second", now.Add(time.Hour), now.Add(time.Second))
	// Two: the already-expired "sess-expired" too, since the filter is only != keep.
	if removed, err := s.DeleteOtherSessionsForUser(ctx, uid, "sess-second"); err != nil || removed != 2 {
		t.Fatalf("removed %d, %v", removed, err)
	}
	requireNoSession(t, s, "sess-active", now)
	if owned, err := s.DeleteSessionOwned(ctx, "sess-second", uid+1); err != nil || owned {
		t.Fatalf("another user's session must survive: %v %v", owned, err)
	}
	if owned, err := s.DeleteSessionOwned(ctx, "sess-second", uid); err != nil || !owned {
		t.Fatalf("%v %v", owned, err)
	}
	create("sess-a", now.Add(time.Hour), now)
	create("sess-b", now.Add(time.Hour), now)
	if removed, err := s.DeleteSessionsForUser(ctx, uid); err != nil || removed != 2 {
		t.Fatalf("removed %d, %v", removed, err)
	}

	// Both expiry reasons are reclaimed.
	create("sess-idle-expired", now.Add(-time.Hour), now.Add(-2*time.Hour))
	create("sess-cap-expired", now.Add(time.Hour), now.Add(-40*24*time.Hour))
	if deleted, err := s.DeleteExpiredSessions(ctx, now); err != nil || deleted != 2 {
		t.Fatalf("deleted %d, %v", deleted, err)
	}
}
