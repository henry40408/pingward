package store_test

import (
	"errors"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/db"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

func sessionAt(t *testing.T, s *store.Store, id string, userID int64, created, expires time.Time) {
	t.Helper()
	newSession(t, s, id, userID, created, expires)
}

func storedExpiry(t *testing.T, s *store.Store, id string) string {
	t.Helper()
	var expiresAt string
	if err := s.DB.QueryRowContext(ctx, "SELECT expires_at FROM sessions WHERE id = $1", id).Scan(&expiresAt); err != nil {
		t.Fatal(err)
	}
	return expiresAt
}

func requireNoSession(t *testing.T, s *store.Store, id string, now time.Time) {
	t.Helper()
	if u, err := s.FindSessionUser(ctx, id, now); err != nil || u != nil {
		t.Fatalf("session %s resolved: %v %v", id, u, err)
	}
}

func TestADuplicateUsernameIsClassifiedNotSwallowed(t *testing.T) {
	s := testutil.NewStore(t)
	if _, err := s.CreateUser(ctx, "admin", str("phc"), true, t0); err != nil {
		t.Fatal(err)
	}
	_, err := s.CreateUser(ctx, "admin", str("other"), false, t0)
	if !errors.Is(err, store.ErrUsernameTaken) {
		t.Fatalf("expected ErrUsernameTaken, got %v", err)
	}
	if n, _ := s.CountUsers(ctx); n != 1 {
		t.Fatalf("users %d", n)
	}
}

func TestUsernamesDifferingOnlyInCaseAreDistinct(t *testing.T) {
	s := testutil.NewStore(t)
	if _, err := s.CreateUser(ctx, "admin", str("phc"), true, t0); err != nil {
		t.Fatal(err)
	}
	if _, err := s.CreateUser(ctx, "Admin", str("phc"), false, t0); err != nil {
		t.Fatalf("the constraint is case-sensitive on both backends: %v", err)
	}
	if n, _ := s.CountUsers(ctx); n != 2 {
		t.Fatalf("users %d", n)
	}
}

func TestUserAndSessionLifecycle(t *testing.T) {
	s := testutil.NewStore(t)
	seed := mustUser(t, s, "seed")
	uid, err := s.CreateUser(ctx, "bob", str("phc"), true, t0)
	if err != nil {
		t.Fatal(err)
	}
	if n, _ := s.CountUsers(ctx); n != 2 {
		t.Fatalf("users %d", n)
	}
	bob, _ := s.FindUserByUsername(ctx, "bob")
	if bob == nil || bob.ID != uid || !bob.IsAdmin || bob.PasswordHash == nil || *bob.PasswordHash != "phc" {
		t.Fatalf("%+v", bob)
	}
	if nobody, _ := s.FindUserByUsername(ctx, "nobody"); nobody != nil {
		t.Fatal("unknown user found")
	}

	err = s.CreateSession(ctx, store.NewSession{ID: "sess-1", UserID: uid, ExpiresAt: t0.Add(time.Hour),
		UserAgent: str("curl/8.0"), IP: str("127.0.0.1")}, t0)
	if err != nil {
		t.Fatal(err)
	}
	if u, _ := s.FindSessionUser(ctx, "sess-1", t0); u == nil || u.ID != uid {
		t.Fatalf("%+v", u)
	}
	rows, _ := s.ListSessionsForUser(ctx, uid, t0)
	if len(rows) != 1 || rows[0].ID != "sess-1" || !rows[0].CreatedAt.Equal(t0) || !rows[0].LastSeenAt.Equal(t0) ||
		*rows[0].UserAgent != "curl/8.0" || *rows[0].IP != "127.0.0.1" {
		t.Fatalf("%+v", rows)
	}

	// Throttled: under 60s later, last_seen_at stays.
	_, _ = s.FindSessionUser(ctx, "sess-1", t0.Add(30*time.Second))
	rows, _ = s.ListSessionsForUser(ctx, uid, t0)
	if !rows[0].LastSeenAt.Equal(t0) {
		t.Fatal("last seen must be throttled")
	}
	later := t0.Add(61 * time.Second)
	_, _ = s.FindSessionUser(ctx, "sess-1", later)
	rows, _ = s.ListSessionsForUser(ctx, uid, later)
	if !rows[0].LastSeenAt.Equal(later) {
		t.Fatal("last seen must refresh")
	}

	sessionAt(t, s, "sess-2", uid, t0.Add(5*time.Second), t0.Add(2*time.Hour))
	rows, _ = s.ListSessionsForUser(ctx, uid, later)
	if len(rows) != 2 || rows[0].ID != "sess-2" {
		t.Fatalf("newest-created session lists first: %+v", rows)
	}
	if n, _ := s.DeleteOtherSessionsForUser(ctx, uid, "sess-2"); n != 1 {
		t.Fatalf("removed %d", n)
	}
	rows, _ = s.ListSessionsForUser(ctx, uid, later)
	if len(rows) != 1 || rows[0].ID != "sess-2" {
		t.Fatalf("%+v", rows)
	}

	// Another user's id is a silent no-op.
	if ok, _ := s.DeleteSessionOwned(ctx, "sess-2", seed); ok {
		t.Fatal("foreign delete succeeded")
	}
	if rows, _ = s.ListSessionsForUser(ctx, uid, later); len(rows) != 1 {
		t.Fatalf("%+v", rows)
	}
	if ok, _ := s.DeleteSessionOwned(ctx, "sess-2", uid); !ok {
		t.Fatal("owner delete failed")
	}
	if rows, _ = s.ListSessionsForUser(ctx, uid, later); len(rows) != 0 {
		t.Fatalf("%+v", rows)
	}

	// Plain DeleteSession (logout) works unscoped.
	sessionAt(t, s, "sess-3", uid, t0, t0.Add(time.Hour))
	_ = s.DeleteSession(ctx, "sess-3")
	requireNoSession(t, s, "sess-3", t0)

	// DeleteSessionsForUser keeps no row and spares other users.
	carol, _ := s.CreateUser(ctx, "carol", str("phc"), false, t0)
	sessionAt(t, s, "sess-4", uid, t0, t0.Add(time.Hour))
	sessionAt(t, s, "sess-5", uid, t0, t0.Add(time.Hour))
	sessionAt(t, s, "sess-carol", carol, t0, t0.Add(time.Hour))
	if n, _ := s.DeleteSessionsForUser(ctx, uid); n != 2 {
		t.Fatalf("removed %d", n)
	}
	if rows, _ = s.ListSessionsForUser(ctx, uid, t0); len(rows) != 0 {
		t.Fatalf("%+v", rows)
	}
	if rows, _ = s.ListSessionsForUser(ctx, carol, t0); len(rows) != 1 {
		t.Fatalf("%+v", rows)
	}

	// Absolute cap: created 40 days ago, idle window not lapsed.
	sessionAt(t, s, "sess-abs-cap", uid, t0.Add(-40*24*time.Hour), t0.Add(time.Hour))
	requireNoSession(t, s, "sess-abs-cap", t0)
	if rows, _ = s.ListSessionsForUser(ctx, uid, t0); len(rows) != 0 {
		t.Fatalf("%+v", rows)
	}
	sessionAt(t, s, "sess-idle-expiry", uid, t0, t0.Add(time.Hour))
	requireNoSession(t, s, "sess-idle-expiry", t0.Add(2*time.Hour))
}

func TestFindSessionUserDoesNotSlideAFreshSession(t *testing.T) {
	s := testutil.NewStore(t)
	uid := mustUser(t, s, "erin")
	original := t0.Add(70 * time.Hour) // more than half of 72h left
	sessionAt(t, s, "sess-fresh", uid, t0, original)
	_, _ = s.FindSessionUser(ctx, "sess-fresh", t0)
	if got := storedExpiry(t, s, "sess-fresh"); got != db.FormatTime(original) {
		t.Fatalf("expiry %s, want %s", got, db.FormatTime(original))
	}
}

func TestFindSessionUserSlidesExpiry(t *testing.T) {
	s := testutil.NewStore(t)
	uid := mustUser(t, s, "frank")
	sessionAt(t, s, "sess-stale", uid, t0, t0.Add(time.Hour))
	_, _ = s.FindSessionUser(ctx, "sess-stale", t0)
	want := db.FormatTime(t0.Add(72 * time.Hour))
	if got := storedExpiry(t, s, "sess-stale"); got != want {
		t.Fatalf("expiry %s, want %s", got, want)
	}
}

func TestNewUserIsNotDisabled(t *testing.T) {
	s := testutil.NewStore(t)
	u, _ := s.FindUserByID(ctx, mustUser(t, s, "u2"))
	if u.Disabled {
		t.Fatal("new user is disabled")
	}
}

func TestSetUserDisabledToggles(t *testing.T) {
	s := testutil.NewStore(t)
	id := mustUser(t, s, "u3")
	_ = s.SetUserDisabled(ctx, id, true)
	if u, _ := s.FindUserByID(ctx, id); !u.Disabled {
		t.Fatal("not disabled")
	}
	_ = s.SetUserDisabled(ctx, id, false)
	if u, _ := s.FindUserByID(ctx, id); u.Disabled {
		t.Fatal("still disabled")
	}
}

func TestSetPasswordThenLoginHashChanges(t *testing.T) {
	s := testutil.NewStore(t)
	id, _ := s.CreateUser(ctx, "u4", str("old"), false, t0)
	if err := s.SetUserPassword(ctx, id, "newphc"); err != nil {
		t.Fatal(err)
	}
	if u, _ := s.FindUserByID(ctx, id); u.PasswordHash == nil || *u.PasswordHash != "newphc" {
		t.Fatalf("%+v", u)
	}
}

func TestSetAdminAndCountEnabledAdmins(t *testing.T) {
	s := testutil.NewStore(t)
	a, _ := s.CreateUser(ctx, "a", str("p"), true, t0)
	b, _ := s.CreateUser(ctx, "b", str("p"), false, t0)
	want := func(n int64) {
		t.Helper()
		if got, _ := s.CountEnabledAdmins(ctx); got != n {
			t.Fatalf("admins %d, want %d", got, n)
		}
	}
	want(1)
	_ = s.SetUserAdmin(ctx, b, true)
	want(2)
	_ = s.SetUserDisabled(ctx, a, true)
	want(1)
}
