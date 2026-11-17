package server_test

import (
	"net/http"
	"net/url"
	"testing"

	"github.com/henry40408/pingward/internal/testutil"
)

var forwardAuthEnv = map[string]string{
	"PINGWARD_FORWARD_AUTH_HEADER": "X-Forwarded-User",
	"PINGWARD_TRUSTED_PROXIES":     "127.0.0.1",
}

func asUser(name string) http.Header { return http.Header{"X-Forwarded-User": {name}} }

func countUsers(t *testing.T, a *app) int64 {
	t.Helper()
	n, err := a.store.CountUsers(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	return n
}

// The first forward-auth identity on a fresh instance becomes the admin and
// gets a real, SSO-marked session, so the CSRF token of the page it sees works.
func TestTrustedHeaderProvisionsASessionAndTheFirstUserIsAdmin(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	res := a.client.Do("GET", "/projects/new", nil, asUser("sso-admin"))
	wantStatus(t, res, http.StatusOK)
	user, err := a.store.FindUserByUsername(t.Context(), "sso-admin")
	if err != nil || user == nil || !user.IsAdmin || user.PasswordHash != nil {
		t.Fatalf("want a passwordless admin, got %+v (%v)", user, err)
	}
	var sso bool
	if err := a.store.DB.QueryRowContext(t.Context(), "SELECT sso FROM sessions").Scan(&sso); err != nil || !sso {
		t.Fatalf("session must be marked sso: %v %v", sso, err)
	}
	// The form rendered in that same request already carries a working token.
	token := extractCSRF(t, res.Body)
	wantStatus(t, a.client.PostForm("/projects", withToken(newProjectForm(), token)), http.StatusSeeOther)
}

// Fail closed: the header from a peer that is not a trusted proxy is ignored,
// so a client cannot name itself an admin.
func TestHeaderFromAnUntrustedPeerIsIgnored(t *testing.T) {
	env := map[string]string{"PINGWARD_FORWARD_AUTH_HEADER": "X-Forwarded-User", "PINGWARD_TRUSTED_PROXIES": "203.0.113.9"}
	a := newApp(t, env)
	a.addUser(t, "admin", true)
	wantBouncedToLogin(t, a.client.Do("GET", "/projects/new", nil, asUser("admin")))
	if countUsers(t, a) != 1 {
		t.Fatal("an untrusted header must not create a user")
	}
}

func TestHeaderIsIgnoredWithoutAnyTrustedProxyConfigured(t *testing.T) {
	a := newApp(t, map[string]string{"PINGWARD_FORWARD_AUTH_HEADER": "X-Forwarded-User"})
	a.addUser(t, "admin", true)
	wantBouncedToLogin(t, a.client.Do("GET", "/projects/new", nil, asUser("admin")))
}

func TestForwardAuthIsOffWithoutTheHeaderSetting(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "admin", true)
	wantBouncedToLogin(t, a.client.Do("GET", "/projects/new", nil, asUser("admin")))
}

func TestDisabledAccountIsNotLetInByTheHeader(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	id := a.addUser(t, "bob", false)
	if err := a.store.SetUserDisabled(t.Context(), id, true); err != nil {
		t.Fatal(err)
	}
	wantBouncedToLogin(t, a.client.Do("GET", "/projects/new", nil, asUser("bob")))
}

// A name the other surfaces would refuse must not mint an account.
func TestInvalidHeaderUsernameDoesNotProvisionAnAccount(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	a.addUser(t, "admin", true)
	wantBouncedToLogin(t, a.client.Do("GET", "/projects/new", nil, asUser("   ")))
	wantBouncedToLogin(t, a.client.Do("GET", "/projects/new", nil, asUser("a\tb")))
	if countUsers(t, a) != 1 {
		t.Fatal("an invalid header name must not create a user")
	}
}

func TestLaterForwardAuthUsersAreNotAdmins(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	a.addUser(t, "admin", true)
	wantStatus(t, a.client.Do("GET", "/projects/new", nil, asUser("carol")), http.StatusOK)
	user, _ := a.store.FindUserByUsername(t.Context(), "carol")
	if user == nil || user.IsAdmin {
		t.Fatalf("a later identity must not be admin: %+v", user)
	}
}

// An existing real session wins over the header: one identity per session.
func TestLiveSessionIsNotReplacedByTheHeader(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	a.addUser(t, "alice", false)
	a.login(t, "alice")
	wantStatus(t, a.client.Do("GET", "/projects/new", nil, asUser("mallory")), http.StatusOK)
	if user, _ := a.store.FindUserByUsername(t.Context(), "mallory"); user != nil {
		t.Fatal("a live session must short-circuit the header")
	}
}

func TestLogoutBehindAGatewayExplainsInsteadOfRedirectingToLogin(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	a.addUser(t, "admin", true)
	wantStatus(t, a.client.Do("GET", "/projects/new", nil, asUser("admin")), http.StatusOK)
	token := testutil.NewestSessionCSRF(t, a.store)
	res := a.client.Do("POST", "/logout", nil, http.Header{"X-Forwarded-User": {"admin"}, "X-Csrf-Token": {token}})
	wantStatus(t, res, http.StatusSeeOther)
	if got := res.Header.Get("Location"); got != "/" {
		t.Fatalf("Location = %q, want /", got)
	}
	if res.Header.Get("Clear-Site-Data") != "" {
		t.Error("the flash exit must not send Clear-Site-Data")
	}
}

func TestLogoutURLComesFromConfigNeverTheRequest(t *testing.T) {
	env := map[string]string{
		"PINGWARD_FORWARD_AUTH_HEADER": "X-Forwarded-User", "PINGWARD_TRUSTED_PROXIES": "127.0.0.1",
		"PINGWARD_FORWARD_AUTH_LOGOUT_URL": "https://sso.example/logout",
	}
	a := newApp(t, env)
	a.addUser(t, "admin", true)
	wantStatus(t, a.client.Do("GET", "/projects/new", nil, asUser("admin")), http.StatusOK)
	token := testutil.NewestSessionCSRF(t, a.store)
	res := a.client.PostForm("/logout?next=https://evil.example", url.Values{"_csrf": {token}})
	if got := res.Header.Get("Location"); got != "https://sso.example/logout" {
		t.Fatalf("Location = %q", got)
	}
}
