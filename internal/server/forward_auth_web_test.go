package server_test

import (
	"net/http"
	"strings"
	"sync"
	"testing"

	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/secret"
	"github.com/henry40408/pingward/internal/testutil"
)

func sessionCookieSet(res *testutil.Response) *http.Cookie {
	for _, cookie := range res.Cookies() {
		if cookie.Name == auth.SessionCookieName(false) {
			return cookie
		}
	}
	return nil
}

// signedInViaGateway returns alice's session cookie, minted by the gateway header.
func signedInViaGateway(t *testing.T, a *app) *http.Cookie {
	t.Helper()
	res := a.client.Do("GET", "/projects/new", nil, asUser("alice"))
	wantStatus(t, res, http.StatusOK)
	cookie := sessionCookieSet(res)
	if cookie == nil {
		t.Fatal("forward auth must mint a session cookie")
	}
	return cookie
}

func sessionRows(t *testing.T, a *app) int {
	t.Helper()
	var rows int
	if err := a.store.DB.QueryRowContext(t.Context(), "SELECT COUNT(*) FROM sessions").Scan(&rows); err != nil {
		t.Fatal(err)
	}
	return rows
}

func TestForwardAuthSessionIsReusedAcrossRequests(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	cookie := signedInViaGateway(t, a)
	res := a.client.Do("GET", "/projects/new", nil, http.Header{
		"X-Forwarded-User": {"alice"}, "Cookie": {cookie.Name + "=" + cookie.Value},
	})
	wantStatus(t, res, http.StatusOK)
	if sessionCookieSet(res) != nil {
		t.Error("a live session must not be replaced")
	}
	if rows := sessionRows(t, a); rows != 1 {
		t.Fatalf("sessions = %d, want 1", rows)
	}
}

// Every page sees the flash cookie; one that cleared it would swallow the
// warning before the dashboard renders it.
func TestForwardAuthLogoutFlashDoesNotLeakOntoOtherPages(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	cookie := signedInViaGateway(t, a)
	flash := "pingward_flash=" + testutil.SignedFlashCookie("forward_auth_logout")
	res := a.client.Do("GET", "/projects/new", nil, http.Header{
		"X-Forwarded-User": {"alice"}, "Cookie": {cookie.Name + "=" + cookie.Value + "; " + flash},
	})
	wantStatus(t, res, http.StatusOK)
	for _, set := range res.Header.Values("Set-Cookie") {
		if strings.HasPrefix(set, "pingward_flash=") {
			t.Errorf("a page that does not own the surface must leave the flash intact: %s", set)
		}
	}
	if strings.Contains(res.Body, "forward-auth-logout-flash") {
		t.Error("the warning must not render off the dashboard")
	}
}

func TestForwardAuthSessionIsFlaggedSSOOnTheAccountPage(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	cookie := signedInViaGateway(t, a)
	res := a.client.Do("GET", "/account", nil, http.Header{
		"X-Forwarded-User": {"alice"}, "Cookie": {cookie.Name + "=" + cookie.Value},
	})
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, `data-testid="session-sso"`) {
		t.Fatal("a forward-auth-minted session must render the SSO pill")
	}
	if strings.Contains(res.Body, `id="current_password"`) {
		t.Error("a passwordless account has no password card")
	}
}

// A cookie outliving its row (pruned, or wiped by a migration) would otherwise
// be a phantom session, absent from /account.
func TestStaleSessionCookieIsReplacedRatherThanTrusted(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	stale := signedInViaGateway(t, a)
	if _, err := a.store.DB.ExecContext(t.Context(), "DELETE FROM sessions"); err != nil {
		t.Fatal(err)
	}
	a.client.ClearCookies()
	res := a.client.Do("GET", "/projects/new", nil, http.Header{
		"X-Forwarded-User": {"alice"}, "Cookie": {stale.Name + "=" + stale.Value},
	})
	wantStatus(t, res, http.StatusOK)
	fresh := sessionCookieSet(res)
	if fresh == nil || fresh.Value == stale.Value {
		t.Fatalf("a stale cookie must be replaced, got %v", fresh)
	}
	// This request's own form must already use the fresh cookie's token.
	id, ok := secret.VerifySession([]byte(testutil.TestSecret), fresh.Value)
	if !ok {
		t.Fatal("the fresh cookie must verify")
	}
	if got, want := extractCSRF(t, res.Body), secret.DeriveCSRF([]byte(testutil.TestSecret), id); got != want {
		t.Fatalf("csrf = %s, want the fresh cookie's %s", got, want)
	}
}

// Simultaneous first logins must not each decide they are first.
func TestConcurrentFirstForwardAuthLoginsYieldOneAdmin(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	var wg sync.WaitGroup
	for _, name := range []string{"u1", "u2", "u3", "u4", "u5"} {
		wg.Add(1)
		go func() {
			defer wg.Done()
			req, err := http.NewRequest("GET", a.client.Server.URL+"/projects/new", nil)
			if err != nil {
				return
			}
			req.Header.Set("X-Forwarded-User", name)
			if res, err := http.DefaultClient.Do(req); err == nil {
				_ = res.Body.Close()
			}
		}()
	}
	wg.Wait()
	var admins int
	if err := a.store.DB.QueryRowContext(t.Context(), "SELECT COUNT(*) FROM users WHERE is_admin = 1").Scan(&admins); err != nil {
		t.Fatal(err)
	}
	if countUsers(t, a) != 5 || admins != 1 {
		t.Fatalf("users = %d, admins = %d; want 5 and exactly one first user", countUsers(t, a), admins)
	}
}

// The first admin ends the setup window, however that admin was created.
func TestFirstForwardAuthAdminRetiresTheSetupToken(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	a.deps.SetupToken.Arm("ABCDE-12345")
	wantStatus(t, a.client.Do("GET", "/projects/new", nil, asUser("alice")), http.StatusOK)
	if a.deps.SetupToken.Get() != "" {
		t.Fatal("the setup token must be cleared")
	}
}
