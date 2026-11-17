package server_test

import (
	"context"
	"database/sql/driver"
	"net/http"
	"net/url"
	"sync"
	"testing"
	"time"

	"modernc.org/sqlite"

	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

// cancelRequest is the cancel func of the request in flight. A SQLite trigger
// calls it right after the password hash changes, which is how a browser
// hanging up between the two writes is simulated deterministically.
var cancelRequest struct {
	sync.Mutex
	cancel func()
}

func init() {
	sqlite.MustRegisterScalarFunction("pw_cancel_request", 0, func(*sqlite.FunctionContext, []driver.Value) (driver.Value, error) {
		cancelRequest.Lock()
		defer cancelRequest.Unlock()
		if cancelRequest.cancel != nil {
			cancelRequest.cancel()
		}
		return int64(0), nil
	})
}

// disconnectAfterPasswordWrite replaces a's client with one whose requests are
// cancelled the moment a password hash is written.
func (a *app) disconnectAfterPasswordWrite(t *testing.T) {
	t.Helper()
	trigger := "CREATE TRIGGER cancel_after_password AFTER UPDATE OF password_hash ON users BEGIN SELECT pw_cancel_request(); END"
	if _, err := a.store.DB.ExecContext(t.Context(), trigger); err != nil {
		t.Fatal(err)
	}
	handler := newHandler(a.deps)
	a.client = testutil.NewClient(t, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		ctx, cancel := context.WithCancel(r.Context())
		cancelRequest.Lock()
		cancelRequest.cancel = cancel
		cancelRequest.Unlock()
		defer func() {
			cancelRequest.Lock()
			cancelRequest.cancel = nil
			cancelRequest.Unlock()
			cancel()
		}()
		handler.ServeHTTP(w, r.WithContext(ctx))
	}))
}

// A browser that hangs up after the new password is stored must not leave the
// old sessions valid: the revocation runs on a context the hang-up cannot cancel.
func TestPasswordChangeStillRevokesOtherSessionsAfterTheClientDisconnects(t *testing.T) {
	a, _ := memberApp(t)
	other := testutil.NewClient(t, newHandler(a.deps))
	csrf := testutil.AnonymousCSRF(t, other, "/login")
	wantStatus(t, other.PostForm("/login", url.Values{"_csrf": {csrf}, "username": {"member"}, "password": {testPassword}}), http.StatusSeeOther)
	a.disconnectAfterPasswordWrite(t)
	a.login(t, "member")

	a.post(t, "/account/password", passwordForm(testPassword, "a-brand-new-passphrase", "a-brand-new-passphrase"))

	wantBouncedToLogin(t, other.Get("/account"))
	var detail string
	err := a.store.DB.QueryRowContext(t.Context(), "SELECT detail FROM audit_log WHERE action = 'user.password_change'").Scan(&detail)
	if err != nil || detail != "sessions_revoked=2" {
		t.Fatalf("audit detail = %q (%v): the audit must be written too", detail, err)
	}
}

func TestAdminResetStillRevokesTheTargetsSessionsAfterTheClientDisconnects(t *testing.T) {
	a, memberID := adminApp(t)
	if err := a.store.CreateSession(t.Context(), store.NewSession{ID: "stolen", UserID: memberID, ExpiresAt: time.Now().Add(time.Hour)}, time.Now()); err != nil {
		t.Fatal(err)
	}
	a.disconnectAfterPasswordWrite(t)
	a.login(t, "admin")
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)

	a.post(t, userPath(memberID, "password"), url.Values{"password": {"a-brand-new-passphrase"}})

	if n := countRows(t, a, "SELECT COUNT(*) FROM sessions WHERE user_id = $1", memberID); n != 0 {
		t.Fatalf("the target keeps %d sessions", n)
	}
	if detail := a.lastAudit(t, "user.password_reset").Detail; detail == nil || *detail != "sessions_revoked=1" {
		t.Fatalf("audit detail = %v", detail)
	}
}
