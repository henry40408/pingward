package store_test

import (
	"reflect"
	"testing"
	"time"

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
