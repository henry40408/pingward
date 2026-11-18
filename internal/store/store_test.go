package store_test

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

var (
	ctx = context.Background()
	t0  = time.Date(2026, 1, 1, 12, 0, 0, 0, time.UTC)
)

func str(s string) *string { return &s }

func mustUser(t *testing.T, s *store.Store, name string) int64 {
	t.Helper()
	id, err := s.CreateUser(ctx, name, str("hash"), false, t0)
	if err != nil {
		t.Fatal(err)
	}
	return id
}

func newSession(t *testing.T, s *store.Store, id string, userID int64, created, expires time.Time) {
	t.Helper()
	if err := s.CreateSession(ctx, store.NewSession{ID: id, UserID: userID, ExpiresAt: expires}, created); err != nil {
		t.Fatal(err)
	}
}

func TestDuplicateUsernameIsTaken(t *testing.T) {
	s := testutil.NewStore(t)
	mustUser(t, s, "alice")
	if _, err := s.CreateUser(ctx, "alice", nil, false, t0); !errors.Is(err, store.ErrUsernameTaken) {
		t.Fatalf("got %v", err)
	}
	// Case differs: a distinct name.
	if _, err := s.CreateUser(ctx, "Alice", nil, false, t0); err != nil {
		t.Fatal(err)
	}
}

func TestFirstAdminOnlyWhileEmpty(t *testing.T) {
	s := testutil.NewStore(t)
	id, found, err := s.CreateFirstAdmin(ctx, "root", "h", t0)
	if err != nil || !found {
		t.Fatalf("%v %v", found, err)
	}
	user, _ := s.FindUserByID(ctx, id)
	if user == nil || !user.IsAdmin || user.Disabled {
		t.Fatalf("%+v", user)
	}
	if _, found, err := s.CreateFirstAdmin(ctx, "x", "h", t0); err != nil || found {
		t.Fatalf("second admin: %v %v", found, err)
	}
	second, err := s.CreatePasswordlessUserFirstIsAdmin(ctx, "sso", t0)
	if err != nil {
		t.Fatal(err)
	}
	if u, _ := s.FindUserByID(ctx, second); u.IsAdmin || u.PasswordHash != nil {
		t.Fatalf("%+v", u)
	}
}

func TestPasswordlessFirstUserIsAdmin(t *testing.T) {
	s := testutil.NewStore(t)
	id, err := s.CreatePasswordlessUserFirstIsAdmin(ctx, "first", t0)
	if err != nil {
		t.Fatal(err)
	}
	if u, _ := s.FindUserByID(ctx, id); !u.IsAdmin {
		t.Fatal("first passwordless user must be admin")
	}
}

func TestUserMutations(t *testing.T) {
	s := testutil.NewStore(t)
	a := mustUser(t, s, "a")
	b := mustUser(t, s, "b")
	if err := s.SetUserAdmin(ctx, a, true); err != nil {
		t.Fatal(err)
	}
	if n, _ := s.CountEnabledAdmins(ctx); n != 1 {
		t.Fatalf("admins %d", n)
	}
	if err := s.SetUserDisabled(ctx, a, true); err != nil {
		t.Fatal(err)
	}
	if n, _ := s.CountEnabledAdmins(ctx); n != 0 {
		t.Fatalf("admins %d", n)
	}
	if err := s.SetUserPassword(ctx, b, "new"); err != nil {
		t.Fatal(err)
	}
	if u, _ := s.FindUserByUsername(ctx, "b"); u == nil || *u.PasswordHash != "new" || u.Disabled {
		t.Fatalf("%+v", u)
	}
	users, _ := s.ListUsers(ctx)
	if len(users) != 2 || users[0].ID != a {
		t.Fatalf("%+v", users)
	}
	if err := s.DeleteUser(ctx, b); err != nil {
		t.Fatal(err)
	}
	if n, _ := s.CountUsers(ctx); n != 1 {
		t.Fatalf("users %d", n)
	}
	if u, err := s.FindUserByUsername(ctx, "gone"); u != nil || err != nil {
		t.Fatalf("%v %v", u, err)
	}
}

func TestAPIKeyLifecycle(t *testing.T) {
	s := testutil.NewStore(t)
	owner, other := mustUser(t, s, "o"), mustUser(t, s, "p")
	expires := t0.Add(time.Hour)
	first, err := s.InsertAPIKey(ctx, store.NewAPIKey{UserID: owner, Name: "k1", TokenHash: "h1", Prefix: "pw_a"}, t0)
	if err != nil {
		t.Fatal(err)
	}
	second, _ := s.InsertAPIKey(ctx, store.NewAPIKey{UserID: owner, Name: "k2", TokenHash: "h2", Prefix: "pw_b", ExpiresAt: &expires}, t0)
	keys, _ := s.ListAPIKeysForUser(ctx, owner)
	if len(keys) != 2 || keys[0].ID != second || keys[1].LastUsedAt != nil {
		t.Fatalf("%+v", keys)
	}
	if uid, ok, _ := s.ValidateAPIKey(ctx, "h1", t0); !ok || uid != owner {
		t.Fatal("h1 should validate")
	}
	if _, ok, _ := s.ValidateAPIKey(ctx, "nope", t0); ok {
		t.Fatal("unknown hash validated")
	}
	if _, ok, _ := s.ValidateAPIKey(ctx, "h2", expires); ok {
		t.Fatal("expiry instant must be rejected")
	}
	// last_used_at is throttled to once per 60s.
	s.ValidateAPIKey(ctx, "h1", t0.Add(30*time.Second))
	keys, _ = s.ListAPIKeysForUser(ctx, owner)
	if got := keys[1].LastUsedAt; got == nil || !got.Equal(t0) {
		t.Fatalf("last used %v", got)
	}
	s.ValidateAPIKey(ctx, "h1", t0.Add(60*time.Second))
	keys, _ = s.ListAPIKeysForUser(ctx, owner)
	if got := keys[1].LastUsedAt; !got.Equal(t0.Add(60 * time.Second)) {
		t.Fatalf("last used must refresh at exactly 60s, got %v", got)
	}
	s.ValidateAPIKey(ctx, "h1", t0.Add(121*time.Second))
	keys, _ = s.ListAPIKeysForUser(ctx, owner)
	if got := keys[1].LastUsedAt; !got.Equal(t0.Add(121 * time.Second)) {
		t.Fatalf("last used %v", got)
	}
	if ok, _ := s.DeleteAPIKey(ctx, first, other); ok {
		t.Fatal("foreign delete succeeded")
	}
	if ok, _ := s.DeleteAPIKey(ctx, first, owner); !ok {
		t.Fatal("owner delete failed")
	}
}

func TestSessionSlidingAndThrottle(t *testing.T) {
	s := testutil.NewStore(t)
	uid := mustUser(t, s, "u")
	newSession(t, s, "fresh", uid, t0, t0.Add(70*time.Hour))
	if u, err := s.FindSessionUser(ctx, "fresh", t0); err != nil || u == nil || u.ID != uid {
		t.Fatalf("%v %v", u, err)
	}
	sessions, _ := s.ListSessionsForUser(ctx, uid, t0)
	if len(sessions) != 1 || !sessions[0].ExpiresAt.Equal(t0.Add(70*time.Hour)) {
		t.Fatalf("a fresh session must not slide: %+v", sessions)
	}
	if sessions[0].LastSeenAt == nil || !sessions[0].LastSeenAt.Equal(t0) {
		t.Fatalf("last seen %v", sessions[0].LastSeenAt)
	}
	s.FindSessionUser(ctx, "fresh", t0.Add(30*time.Second))
	sessions, _ = s.ListSessionsForUser(ctx, uid, t0)
	if !sessions[0].LastSeenAt.Equal(t0) {
		t.Fatal("last seen must be throttled at 30s")
	}
	s.FindSessionUser(ctx, "fresh", t0.Add(60*time.Second))
	sessions, _ = s.ListSessionsForUser(ctx, uid, t0)
	if !sessions[0].LastSeenAt.Equal(t0.Add(60 * time.Second)) {
		t.Fatal("last seen must refresh at exactly 60s")
	}
	s.FindSessionUser(ctx, "fresh", t0.Add(121*time.Second))
	sessions, _ = s.ListSessionsForUser(ctx, uid, t0)
	if !sessions[0].LastSeenAt.Equal(t0.Add(121 * time.Second)) {
		t.Fatal("last seen must refresh at 61s")
	}

	// Past the half-life the expiry slides to now + 72h.
	newSession(t, s, "old", uid, t0, t0.Add(10*time.Hour))
	s.FindSessionUser(ctx, "old", t0)
	sessions, _ = s.ListSessionsForUser(ctx, uid, t0)
	for _, session := range sessions {
		if session.ID == "old" && !session.ExpiresAt.Equal(t0.Add(72*time.Hour)) {
			t.Fatalf("expiry %v", session.ExpiresAt)
		}
	}
}

func TestSessionExpiryAndAbsoluteCap(t *testing.T) {
	s := testutil.NewStore(t)
	uid := mustUser(t, s, "u")
	newSession(t, s, "idle", uid, t0, t0.Add(time.Hour))
	if u, _ := s.FindSessionUser(ctx, "idle", t0.Add(time.Hour)); u != nil {
		t.Fatal("expires_at == now must be rejected")
	}
	// Kept alive by a far expiry, but 40 days old: the cap wins.
	newSession(t, s, "capped", uid, t0, t0.Add(60*24*time.Hour))
	later := t0.Add(40 * 24 * time.Hour)
	if u, _ := s.FindSessionUser(ctx, "capped", later); u != nil {
		t.Fatal("session past the absolute cap must not resolve")
	}
	if got, _ := s.ListSessionsForUser(ctx, uid, later); len(got) != 0 {
		t.Fatalf("capped session listed: %+v", got)
	}
	newSession(t, s, "recent", uid, later.Add(-time.Hour), later.Add(time.Hour))
	newSession(t, s, "at-cap", uid, later.Add(-auth.SessionAbsoluteMax), later.Add(time.Hour))
	if n, _ := s.DeleteCappedSessionsForUser(ctx, uid, later); n != 3 { // "idle" is as old as "capped"
		t.Fatalf("deleted %d, want idle, capped and at-cap but not recent", n)
	}
	if got, _ := s.ListSessionsForUser(ctx, uid, later); len(got) != 1 || got[0].ID != "recent" {
		t.Fatalf("only the recent session may survive: %+v", got)
	}
}

func TestDeleteExpiredSessionsBoundary(t *testing.T) {
	s := testutil.NewStore(t)
	uid := mustUser(t, s, "u")
	now := t0.Add(40 * 24 * time.Hour)
	newSession(t, s, "boundary", uid, now.Add(-time.Hour), now)
	newSession(t, s, "capped", uid, t0, now.Add(time.Hour))
	newSession(t, s, "live", uid, now.Add(-time.Hour), now.Add(time.Hour))
	if _, err := s.DB.ExecContext(ctx,
		"INSERT INTO sessions (id, user_id, expires_at, created_at) VALUES ('legacy', $1, $2, '')",
		uid, now.Add(time.Hour).Format(time.RFC3339)); err != nil {
		t.Fatal(err)
	}
	if n, err := s.DeleteExpiredSessions(ctx, now); err != nil || n != 2 {
		t.Fatalf("deleted %d, %v", n, err)
	}
	for id, want := range map[string]bool{"boundary": false, "capped": false, "live": true, "legacy": true} {
		var n int
		s.DB.QueryRowContext(ctx, "SELECT COUNT(*) FROM sessions WHERE id = $1", id).Scan(&n)
		if (n == 1) != want {
			t.Errorf("%s present=%v want %v", id, n == 1, want)
		}
	}
}

func TestSessionListingAndOwnedDeletes(t *testing.T) {
	s := testutil.NewStore(t)
	a, b := mustUser(t, s, "a"), mustUser(t, s, "b")
	newSession(t, s, "s1", a, t0, t0.Add(time.Hour))
	newSession(t, s, "s2", a, t0.Add(time.Minute), t0.Add(time.Hour))
	newSession(t, s, "s3", b, t0, t0.Add(time.Hour))
	list, _ := s.ListSessionsForUser(ctx, a, t0)
	if len(list) != 2 || list[0].ID != "s2" {
		t.Fatalf("newest first: %+v", list)
	}
	if ok, _ := s.DeleteSessionOwned(ctx, "s3", a); ok {
		t.Fatal("foreign session deleted")
	}
	if n, _ := s.DeleteOtherSessionsForUser(ctx, a, "s2"); n != 1 {
		t.Fatalf("others %d", n)
	}
	if n, _ := s.DeleteSessionsForUser(ctx, a); n != 1 {
		t.Fatalf("all %d", n)
	}
	s.DeleteSession(ctx, "s3")
}

func TestSessionSSOFlagRoundTrips(t *testing.T) {
	s := testutil.NewStore(t)
	uid := mustUser(t, s, "u")
	err := s.CreateSession(ctx, store.NewSession{ID: "x", UserID: uid, ExpiresAt: t0.Add(time.Hour), SSO: true,
		UserAgent: str("ua"), IP: str("1.2.3.4")}, t0)
	if err != nil {
		t.Fatal(err)
	}
	list, _ := s.ListSessionsForUser(ctx, uid, t0)
	if !list[0].SSO || *list[0].IP != "1.2.3.4" || *list[0].UserAgent != "ua" {
		t.Fatalf("%+v", list[0])
	}
}

func TestSettings(t *testing.T) {
	s := testutil.NewStore(t)
	if v, err := s.GetSetting(ctx, "k"); v != nil || err != nil {
		t.Fatalf("%v %v", v, err)
	}
	s.SetSetting(ctx, "k", "1")
	s.SetSetting(ctx, "k", "2")
	if v, _ := s.GetSetting(ctx, "k"); v == nil || *v != "2" {
		t.Fatalf("%v", v)
	}
}

func TestAuditRoundTrips(t *testing.T) {
	s := testutil.NewStore(t)
	entry := store.NewAudit{ActorUserID: 1, ActorUsername: "root", Action: "user.create",
		TargetType: str("user"), Detail: str("d")}
	if _, err := s.RecordAudit(ctx, entry, t0); err != nil {
		t.Fatal(err)
	}
	entry.Action, entry.ActorUsername = "user.delete", "zed"
	s.RecordAudit(ctx, entry, t0.Add(time.Second))
	rows, err := s.ListAudit(ctx, 10)
	if err != nil || len(rows) != 2 || rows[0].Action != "user.delete" || rows[1].TargetID != nil {
		t.Fatalf("%+v %v", rows, err)
	}
	actors, actions, _ := s.AuditFilterOptions(ctx)
	if len(actors) != 2 || actors[0] != "root" || len(actions) != 2 {
		t.Fatalf("%v %v", actors, actions)
	}
}

func TestPostgresRoundTrip(t *testing.T) {
	s := testutil.NewPostgresStore(t)
	id, found, err := s.CreateFirstAdmin(ctx, "root", "h", t0)
	if err != nil || !found {
		t.Fatalf("%v %v", found, err)
	}
	if _, err := s.CreatePasswordlessUserFirstIsAdmin(ctx, "sso", t0); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateUser(ctx, "root", nil, false, t0); !errors.Is(err, store.ErrUsernameTaken) {
		t.Fatalf("%v", err)
	}
	newSession(t, s, "pg", id, t0, t0.Add(time.Hour))
	if u, err := s.FindSessionUser(ctx, "pg", t0); err != nil || u == nil {
		t.Fatalf("%v %v", u, err)
	}
}

func TestSlidingTheExpiryLogsTheRenewal(t *testing.T) {
	s := testutil.NewStore(t)
	var logs *testutil.LockedBuffer
	s.Logger, logs = testutil.JSONLogger()
	uid := mustUser(t, s, "u")
	newSession(t, s, "old", uid, t0, t0.Add(10*time.Hour))
	if _, err := s.FindSessionUser(ctx, "old", t0); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(logs.String(), `"session.renewed"`) {
		t.Fatalf("a renewed session must be logged, got %q", logs.String())
	}
}
