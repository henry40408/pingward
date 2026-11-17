package store_test

import (
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

	s.FindSessionUser(ctx, "fresh", t0.Add(30*time.Second))
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
