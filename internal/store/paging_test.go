package store_test

import (
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
)

var pageBase = time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)

func auditIDs(items []models.AuditLog) []int64 {
	ids := make([]int64, len(items))
	for i, item := range items {
		ids[i] = item.ID
	}
	return ids
}

// seededAudit has n audit rows one second apart; row i is (actor{i%2}, action{i%2}).
func seededAudit(t *testing.T, n int) *store.Store {
	t.Helper()
	s := seeded(t)
	base := time.Date(2026, 7, 14, 8, 0, 0, 0, time.UTC)
	for i := 0; i < n; i++ {
		entry := store.NewAudit{
			ActorUserID: int64(i%2 + 1), ActorUsername: "alice", Action: "admin.access",
			TargetType: str("project"), TargetID: i64(int64(i)),
		}
		if i%2 == 1 {
			entry.ActorUsername, entry.Action = "bob", "user.create"
		}
		if _, err := s.RecordAudit(ctx, entry, base.Add(time.Duration(i)*time.Second)); err != nil {
			t.Fatal(err)
		}
	}
	return s
}

func TestAuditRoundtripsWithTargets(t *testing.T) {
	s := seeded(t)
	uid, err := s.CreateUser(ctx, "adm", str("phc"), true, t0)
	if err != nil {
		t.Fatal(err)
	}
	_, err = s.RecordAudit(ctx, store.NewAudit{
		ActorUserID: uid, ActorUsername: "adm", Action: "admin.access", TargetType: str("project"),
		TargetID: i64(7), TargetOwnerID: i64(42), Method: str("GET"), Path: str("/admin/projects/7"),
	}, t0)
	if err != nil {
		t.Fatal(err)
	}
	rows, err := s.ListAudit(ctx, 10)
	if err != nil || len(rows) != 1 || rows[0].Action != "admin.access" ||
		rows[0].TargetOwnerID == nil || *rows[0].TargetOwnerID != 42 || rows[0].ActorUsername != "adm" {
		t.Fatalf("%+v %v", rows, err)
	}
}

// The unscoped path through the keyset query.
func TestListAuditPageKeysetPagination(t *testing.T) {
	s := seededAudit(t, 25)
	none := store.AuditFilter{}
	first, err := s.ListAuditPage(ctx, store.PageCursor{}, 10, none)
	if err != nil || len(first.Items) != 10 || first.HasNewer || !first.HasOlder || first.Items[0].ID <= first.Items[9].ID {
		t.Fatalf("%+v %v", first, err)
	}
	older, _ := s.ListAuditPage(ctx, store.Before(first.Items[9].ID), 10, none)
	if len(older.Items) != 10 || !older.HasNewer || !older.HasOlder || older.Items[0].ID >= first.Items[9].ID {
		t.Fatalf("%+v", older)
	}
	last, _ := s.ListAuditPage(ctx, store.Before(older.Items[9].ID), 10, none)
	if len(last.Items) != 5 || last.HasOlder {
		t.Fatalf("25 rows = 10 + 10 + 5: %+v", last)
	}
	back, _ := s.ListAuditPage(ctx, store.After(older.Items[0].ID), 10, none)
	if !reflect.DeepEqual(auditIDs(back.Items), auditIDs(first.Items)) {
		t.Fatalf("paging back: %v vs %v", auditIDs(back.Items), auditIDs(first.Items))
	}
}

func TestListAuditPageFiltersByActorActionAndDate(t *testing.T) {
	s := seededAudit(t, 10)
	base := time.Date(2026, 7, 14, 8, 0, 0, 0, time.UTC)
	page := func(filter store.AuditFilter) store.Page[models.AuditLog] {
		got, err := s.ListAuditPage(ctx, store.PageCursor{}, 50, filter)
		if err != nil {
			t.Fatal(err)
		}
		return got
	}
	if got := page(store.AuditFilter{Actor: str("bob")}); len(got.Items) != 5 || got.Items[0].ActorUsername != "bob" {
		t.Fatalf("by actor: %+v", got.Items)
	}
	if got := page(store.AuditFilter{Action: str("admin.access")}); len(got.Items) != 5 || got.Items[0].Action != "admin.access" {
		t.Fatalf("by action: %+v", got.Items)
	}
	// AND-ed: alice never performs user.create.
	if got := page(store.AuditFilter{Actor: str("alice"), Action: str("user.create")}); len(got.Items) != 0 {
		t.Fatalf("contradictory: %+v", got.Items)
	}
	// Inclusive bounds: rows 2..=4.
	from, to := base.Add(2*time.Second), base.Add(4*time.Second)
	if got := page(store.AuditFilter{From: &from, To: &to}); len(got.Items) != 3 {
		t.Fatalf("windowed: %+v", got.Items)
	}
	if got := page(store.AuditFilter{Actor: str("nobody")}); len(got.Items) != 0 || got.HasOlder {
		t.Fatalf("nobody: %+v", got)
	}
}

func TestAuditFilterOptionsAreDistinctAndSorted(t *testing.T) {
	s := seededAudit(t, 6)
	actors, actions, err := s.AuditFilterOptions(ctx)
	if err != nil || !reflect.DeepEqual(actors, []string{"alice", "bob"}) ||
		!reflect.DeepEqual(actions, []string{"admin.access", "user.create"}) {
		t.Fatalf("%v %v %v", actors, actions, err)
	}
}

func TestStatusCountsAndScale(t *testing.T) {
	s := seeded(t)
	uid, err := s.CreateUser(ctx, "u2", str("p"), false, t0)
	if err != nil {
		t.Fatal(err)
	}
	pid, err := s.CreateProject(ctx, uid, store.ProjectSettings{Name: "p2"}, t0)
	if err != nil {
		t.Fatal(err)
	}
	newPeriod := func(name, uuid string) int64 {
		check := periodCheck(pid, uuid)
		check.Name, check.PeriodSecs, check.GraceSecs = name, i64(3600), 300
		return mustCheck(t, s, check)
	}
	newPeriod("a", "uuid-a")
	bid := newPeriod("b", "uuid-b")

	assertCounts := func(want store.CheckStatusCounts, wantChecks int64) {
		t.Helper()
		counts, err := s.CountChecksByStatus(ctx)
		total, _ := s.CountChecks(ctx)
		if err != nil || counts != want || total != wantChecks || counts.New+counts.Up+counts.Down+counts.Paused != total {
			t.Fatalf("counts %+v want %+v, checks %d: %v", counts, want, total, err)
		}
	}
	assertCounts(store.CheckStatusCounts{New: 2}, 2)
	if projects, _ := s.CountProjects(ctx); projects != 2 {
		t.Fatalf("projects %d", projects)
	}

	first := time.Date(2026, 7, 14, 8, 0, 0, 0, time.UTC)
	second := first.Add(time.Second)
	mark := func(id int64, status models.CheckStatus, ping, start *time.Time) {
		t.Helper()
		if err := s.MarkPing(ctx, id, store.PingMark{Status: status, LastPingAt: ping, LastStartAt: start}); err != nil {
			t.Fatal(err)
		}
	}
	// b starts and never finishes: stored new, running.
	mark(bid, models.StatusNew, nil, &first)
	// c succeeds, then starts again: running on top of up.
	cid := newPeriod("c", "uuid-c")
	mark(cid, models.StatusUp, &first, nil)
	mark(cid, models.StatusUp, nil, &second)
	// d fails, then starts again: down beats running.
	did := newPeriod("d", "uuid-d")
	mark(did, models.StatusDown, &first, nil)
	mark(did, models.StatusDown, nil, &second)

	assertCounts(store.CheckStatusCounts{New: 2, Up: 1, Down: 1, Running: 2}, 4)
}

func TestNotificationCountsSplitOkError(t *testing.T) {
	s := seeded(t)
	cid := mustCheck(t, s, periodCheck(1, "notif-uuid"))
	chann := mustChannel(t, s, 1, "h")
	for _, n := range []store.NewNotification{
		{CheckID: cid, ChannelID: chann, Event: models.EventUp, Status: models.NotifyOK},
		{CheckID: cid, ChannelID: chann, Event: models.EventDown, Status: models.NotifyError, Error: str("boom")},
	} {
		if err := s.RecordNotification(ctx, n, t0); err != nil {
			t.Fatal(err)
		}
	}
	ok, failed, err := s.NotificationCountsSince(ctx, t0.Add(-24*time.Hour))
	if err != nil || ok != 1 || failed != 1 {
		t.Fatalf("%d %d %v", ok, failed, err)
	}
	if failedRows, _ := s.RecentFailedNotifications(ctx, 10); len(failedRows) != 1 || failedRows[0].Error == nil {
		t.Fatalf("%+v", failedRows)
	}
}

func TestChannelFailureCountsDoesNotMergeSameNamedChannels(t *testing.T) {
	s := seeded(t)
	cid := mustCheck(t, s, periodCheck(1, "dup-uuid"))
	a, b := mustChannel(t, s, 1, "dup"), mustChannel(t, s, 1, "dup")
	for _, channelID := range []int64{a, b} {
		err := s.RecordNotification(ctx, store.NewNotification{
			CheckID: cid, ChannelID: channelID, Event: models.EventDown, Status: models.NotifyError, Error: str("boom"),
		}, t0)
		if err != nil {
			t.Fatal(err)
		}
	}
	rows, err := s.ChannelFailureCountsSince(ctx, t0.Add(-24*time.Hour))
	if err != nil || len(rows) != 2 {
		t.Fatalf("same-named channels must stay two rows: %+v %v", rows, err)
	}
	for _, row := range rows {
		if row.ChannelName != "dup" || row.OK != 0 || row.Errors != 1 {
			t.Fatalf("%+v", row)
		}
	}
}

func TestDownChecksOrderNeverPingedLast(t *testing.T) {
	s := seeded(t)
	a := periodCheck(1, "uuid-a")
	a.Name = "A"
	aid := mustCheck(t, s, a)
	b := periodCheck(1, "uuid-b")
	b.Name = "B"
	bid := mustCheck(t, s, b)
	at := time.Date(2026, 7, 12, 12, 0, 0, 0, time.UTC)
	if err := s.MarkPing(ctx, aid, store.PingMark{Status: models.StatusDown, LastPingAt: &at, LastStartAt: &at}); err != nil {
		t.Fatal(err)
	}
	if err := s.SetStatus(ctx, bid, models.StatusDown); err != nil {
		t.Fatal(err)
	}
	rows, err := s.ListDownChecksWithOwner(ctx)
	if err != nil || len(rows) != 2 || rows[0].Check.Name != "A" || rows[1].Check.Name != "B" {
		t.Fatalf("pinged check must come before never-pinged: %+v %v", rows, err)
	}
	if rows[0].ProjectName != "p" || rows[0].OwnerUsername != "u" {
		t.Fatalf("%+v", rows[0])
	}
}

func checkIDs(checks []models.Check) []int64 {
	ids := []int64{}
	for _, check := range checks {
		ids = append(ids, check.ID)
	}
	return ids
}

func TestBatchChecksForProjectsMatchesPerProject(t *testing.T) {
	s := seeded(t)
	mustProject(t, s, "p2", nil)
	mustProject(t, s, "p3", nil)
	// Interleaved, to catch rows leaking across groups or insertion-order reliance.
	for _, c := range []struct {
		project    int64
		name, uuid string
	}{{1, "a1", "u-a1"}, {2, "b1", "u-b1"}, {1, "a2", "u-a2"}, {2, "b2", "u-b2"}, {1, "a3", "u-a3"}} {
		check := periodCheck(c.project, c.uuid)
		check.Name = c.name
		mustCheck(t, s, check)
	}
	batch, err := s.ListChecksForProjects(ctx, []int64{1, 2, 3})
	if err != nil || len(batch[1]) != 3 || len(batch[2]) != 2 {
		t.Fatalf("%v %v", batch, err)
	}
	if _, present := batch[3]; present {
		t.Fatal("a project with no checks must be absent")
	}
	for _, pid := range []int64{1, 2, 3} {
		single, _ := s.ListChecksForProject(ctx, pid)
		if got := checkIDs(batch[pid]); !reflect.DeepEqual(got, checkIDs(single)) {
			t.Fatalf("project %d: %v vs %v", pid, got, checkIDs(single))
		}
	}
	if empty, err := s.ListChecksForProjects(ctx, nil); err != nil || len(empty) != 0 {
		t.Fatalf("%v %v", empty, err)
	}
}

// A drift from the wide query would silently redraw every strip.
func TestPingSummariesMatchTheWideQueryRowForRow(t *testing.T) {
	s := seeded(t)
	var ids []int64
	for n := 0; n < 3; n++ {
		check := periodCheck(1, "u"+string(rune('0'+n)))
		cid := mustCheck(t, s, check)
		ids = append(ids, cid)
		for i := 0; i < 5; i++ {
			kind := models.PingSuccess
			if i%2 == 1 {
				kind = models.PingStart
			}
			err := s.InsertPing(ctx, store.NewPing{
				CheckID: cid, Kind: kind, ExitCode: i64(0), Body: strings.Repeat("x", 4096), SourceIP: str("10.0.0.1"),
			}, pageBase.Add(time.Duration(i)*time.Second))
			if err != nil {
				t.Fatal(err)
			}
		}
	}
	silent := mustCheck(t, s, periodCheck(1, "u-silent"))

	// The limit differs from every check ID so a wrong placeholder index
	// binds the wrong number.
	narrow, err := s.ListRecentPingSummariesForChecks(ctx, append(append([]int64{}, ids...), silent), 2)
	if err != nil || len(narrow) != len(ids) {
		t.Fatalf("%v %v", narrow, err)
	}
	if _, present := narrow[silent]; present {
		t.Fatal("a check with no pings must be absent from the batch")
	}
	for _, cid := range ids {
		wide, _ := s.ListRecentPings(ctx, cid, 2)
		want := make([]models.PingSummary, len(wide))
		for i, ping := range wide {
			want[i] = ping.Summary()
		}
		if len(want) != 2 {
			t.Fatalf("per-check limit: %d", len(want))
		}
		if !reflect.DeepEqual(want, narrow[cid]) {
			t.Fatalf("batched, check %d: %v vs %v", cid, want, narrow[cid])
		}
		single, _ := s.ListRecentPingSummaries(ctx, cid, 2)
		if !reflect.DeepEqual(want, single) {
			t.Fatalf("per-check, check %d", cid)
		}
	}
	if empty, err := s.ListRecentPingSummariesForChecks(ctx, nil, 2); err != nil || len(empty) != 0 {
		t.Fatalf("%v %v", empty, err)
	}
}

func pingIDs(items []models.Ping) []int64 {
	ids := []int64{}
	for _, item := range items {
		ids = append(ids, item.ID)
	}
	return ids
}

func notificationIDs(items []models.Notification) []int64 {
	ids := []int64{}
	for _, item := range items {
		ids = append(ids, item.ID)
	}
	return ids
}

// pageWalk pins the keyset behaviour shared by pings and notifications over
// five rows with ascending ids a..e.
func pageWalk[T any](t *testing.T, ids []int64, page func(store.PageCursor) store.Page[T], idsOf func([]T) []int64) {
	t.Helper()
	id1, id2, id3, id4, id5 := ids[0], ids[1], ids[2], ids[3], ids[4]
	steps := []struct {
		name               string
		cursor             store.PageCursor
		want               []int64
		hasNewer, hasOlder bool
	}{
		{"latest", store.PageCursor{}, []int64{id5, id4}, false, true},
		{"before id4", store.Before(id4), []int64{id3, id2}, true, true},
		{"before id2", store.Before(id2), []int64{id1}, true, false},
		{"after id1", store.After(id1), []int64{id3, id2}, true, true},
	}
	for _, step := range steps {
		got := page(step.cursor)
		if !reflect.DeepEqual(idsOf(got.Items), step.want) || got.HasNewer != step.hasNewer || got.HasOlder != step.hasOlder {
			t.Fatalf("%s: ids %v newer %v older %v", step.name, idsOf(got.Items), got.HasNewer, got.HasOlder)
		}
	}
}

func TestListPingsPageKeysetPagination(t *testing.T) {
	s := seeded(t)
	cid := mustCheck(t, s, periodCheck(1, "page-uuid"))
	for i := 0; i < 5; i++ {
		err := s.InsertPing(ctx, store.NewPing{CheckID: cid, Kind: models.PingSuccess, ExitCode: i64(0)},
			pageBase.Add(time.Duration(i)*time.Second))
		if err != nil {
			t.Fatal(err)
		}
	}
	recent, _ := s.ListRecentPings(ctx, cid, 10)
	ids := pingIDs(recent) // newest first
	reverse := []int64{ids[4], ids[3], ids[2], ids[1], ids[0]}
	pageWalk(t, reverse, func(cursor store.PageCursor) store.Page[models.Ping] {
		page, err := s.ListPingsPage(ctx, cid, cursor, 2, store.PingFilter{})
		if err != nil {
			t.Fatal(err)
		}
		return page
	}, pingIDs)
}

func TestListNotificationsPageKeysetPagination(t *testing.T) {
	s := seeded(t)
	cid := mustCheck(t, s, periodCheck(1, "notif-page-uuid"))
	chann := mustChannel(t, s, 1, "h")
	for i := 0; i < 5; i++ {
		err := s.RecordNotification(ctx, store.NewNotification{
			CheckID: cid, ChannelID: chann, Event: models.EventDown, Status: models.NotifyOK,
		}, pageBase.Add(time.Duration(i)*time.Second))
		if err != nil {
			t.Fatal(err)
		}
	}
	recent, _ := s.ListRecentNotifications(ctx, cid, 10)
	ids := notificationIDs(recent)
	reverse := []int64{ids[4], ids[3], ids[2], ids[1], ids[0]}
	pageWalk(t, reverse, func(cursor store.PageCursor) store.Page[models.Notification] {
		page, err := s.ListNotificationsPage(ctx, cid, cursor, 2, store.NotifFilter{})
		if err != nil {
			t.Fatal(err)
		}
		return page
	}, notificationIDs)
}

func TestListPingsPageFiltersByKindAndDate(t *testing.T) {
	s := seeded(t)
	cid := mustCheck(t, s, periodCheck(1, "ping-filter-uuid"))
	kinds := []models.PingKind{models.PingSuccess, models.PingFail, models.PingStart, models.PingSuccess, models.PingFail}
	for i, kind := range kinds {
		if err := s.InsertPing(ctx, store.NewPing{CheckID: cid, Kind: kind}, pageBase.Add(time.Duration(i)*time.Second)); err != nil {
			t.Fatal(err)
		}
	}
	at := func(seconds int) *time.Time {
		moment := pageBase.Add(time.Duration(seconds) * time.Second)
		return &moment
	}
	seconds := func(items []models.Ping) []int {
		out := []int{}
		for _, item := range items {
			out = append(out, int(item.CreatedAt.Sub(pageBase)/time.Second))
		}
		return out
	}
	list := func(filter store.PingFilter) store.Page[models.Ping] {
		page, err := s.ListPingsPage(ctx, cid, store.PageCursor{}, 20, filter)
		if err != nil {
			t.Fatal(err)
		}
		return page
	}
	if page := list(store.PingFilter{Kinds: []models.PingKind{models.PingFail}}); !reflect.DeepEqual(seconds(page.Items), []int{4, 1}) || page.HasNewer || page.HasOlder {
		t.Fatalf("kind filter: %+v", page)
	}
	if page := list(store.PingFilter{From: at(1), To: at(3)}); !reflect.DeepEqual(seconds(page.Items), []int{3, 2, 1}) {
		t.Fatalf("inclusive range: %v", seconds(page.Items))
	}
	page := list(store.PingFilter{Kinds: []models.PingKind{models.PingFail}, From: at(3), To: at(5)})
	if !reflect.DeepEqual(seconds(page.Items), []int{4}) || page.Items[0].Kind != models.PingFail {
		t.Fatalf("kind and date: %v", seconds(page.Items))
	}
}

func TestListNotificationsPageFiltersByEventAndStatus(t *testing.T) {
	s := seeded(t)
	cid := mustCheck(t, s, periodCheck(1, "notif-filter-uuid"))
	chann := mustChannel(t, s, 1, "h")
	rows := []struct {
		event  models.EventKind
		status models.NotifyStatus
	}{
		{models.EventDown, models.NotifyOK}, {models.EventUp, models.NotifyError}, {models.EventDown, models.NotifyError},
		{models.EventReminder, models.NotifyOK}, {models.EventUp, models.NotifyOK},
	}
	for i, row := range rows {
		n := store.NewNotification{CheckID: cid, ChannelID: chann, Event: row.event, Status: row.status}
		if row.status == models.NotifyError {
			n.Error = str("boom")
		}
		if err := s.RecordNotification(ctx, n, pageBase.Add(time.Duration(i)*time.Second)); err != nil {
			t.Fatal(err)
		}
	}
	list := func(filter store.NotifFilter) []models.Notification {
		page, err := s.ListNotificationsPage(ctx, cid, store.PageCursor{}, 20, filter)
		if err != nil {
			t.Fatal(err)
		}
		return page.Items
	}
	for _, n := range list(store.NotifFilter{Events: []models.EventKind{models.EventUp}}) {
		if n.Event != models.EventUp {
			t.Fatalf("%+v", n)
		}
	}
	if got := list(store.NotifFilter{Events: []models.EventKind{models.EventUp}}); len(got) != 2 {
		t.Fatalf("event filter: %d", len(got))
	}
	got := list(store.NotifFilter{Statuses: []models.NotifyStatus{models.NotifyError}})
	if len(got) != 2 || got[0].Status != models.NotifyError || got[1].Status != models.NotifyError {
		t.Fatalf("status filter: %+v", got)
	}
	got = list(store.NotifFilter{Events: []models.EventKind{models.EventUp}, Statuses: []models.NotifyStatus{models.NotifyError}})
	if len(got) != 1 || got[0].CreatedAt.Sub(pageBase) != time.Second {
		t.Fatalf("event and status: %+v", got)
	}
}

func TestFiltersReportWhetherTheyConstrainAnything(t *testing.T) {
	now := t0
	if !(store.PingFilter{}).IsEmpty() || (store.PingFilter{From: &now}).IsEmpty() {
		t.Fatal("PingFilter.IsEmpty")
	}
	if !(store.NotifFilter{}).IsEmpty() || (store.NotifFilter{Statuses: []models.NotifyStatus{models.NotifyOK}}).IsEmpty() {
		t.Fatal("NotifFilter.IsEmpty")
	}
	empty := ""
	if !(store.AuditFilter{}).IsEmpty() || (store.AuditFilter{Actor: &empty}).IsEmpty() {
		t.Fatal("AuditFilter.IsEmpty: a pointer to an empty string still constrains")
	}
}
