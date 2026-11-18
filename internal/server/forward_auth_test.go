package server_test

import (
	"net/http"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/secret"
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
	if projects, _ := a.store.ListProjectsForUser(t.Context(), user.ID); len(projects) != 1 {
		t.Fatalf("the forward-auth user's own form must create the project, got %d", len(projects))
	}
}

// On an empty instance an untrusted peer must not become the first admin: the
// anonymous cookie it gets addresses no account or session row.
func TestForwardAuthHeaderFromAnUntrustedPeerMintsNothing(t *testing.T) {
	env := map[string]string{"PINGWARD_FORWARD_AUTH_HEADER": "X-Forwarded-User", "PINGWARD_TRUSTED_PROXIES": "203.0.113.9"}
	a := newApp(t, env)
	wantStatus(t, a.client.Do("GET", "/projects/new", nil, asUser("mallory")), http.StatusSeeOther)
	if countUsers(t, a) != 0 || sessionRows(t, a) != 0 {
		t.Fatalf("an untrusted peer must not populate users (%d) or sessions (%d)", countUsers(t, a), sessionRows(t, a))
	}
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
	wantBouncedToLogin(t, a.client.Do("GET", "/projects/new", nil, asUser(strings.Repeat("a", 65))))
	if countUsers(t, a) != 1 || sessionRows(t, a) != 0 {
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

// The gateway re-authenticates on the next request, so a bounce to /login would
// silently sign the user back in; the dashboard warns instead.
func TestLogoutBehindAGatewayExplainsInsteadOfRedirectingToLogin(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	a.addUser(t, "admin", true)
	wantStatus(t, a.client.Do("GET", "/projects/new", nil, asUser("admin")), http.StatusOK)
	token := testutil.NewestSessionCSRF(t, a.store)
	res := a.client.Do("POST", "/logout", nil, http.Header{"X-Forwarded-User": {"admin"}, "X-Csrf-Token": {token}})
	wantLocation(t, res, "/")
	if sessionRows(t, a) != 0 {
		t.Error("the local session is deleted")
	}
	if res.Header.Get("Clear-Site-Data") != "" {
		t.Error("the flash exit must not send Clear-Site-Data, or the warning can never render")
	}
	flash := flashCookieSet(res)
	if flash == nil {
		t.Fatal("the warning flash cookie is set")
	}
	if payload, ok := secret.VerifyFlash([]byte(testutil.TestSecret), flash.Value); !ok || payload != "forward_auth_logout" {
		t.Fatalf("the flash must carry a forward_auth_logout payload signed by this server: %q", flash.Value)
	}

	dashboard := a.client.Do("GET", "/", nil, asUser("admin"))
	wantStatus(t, dashboard, http.StatusOK)
	if sessionRows(t, a) != 1 {
		t.Error("a fresh session is minted")
	}
	if cleared := flashCookieSet(dashboard); cleared == nil || cleared.Value != "" {
		t.Errorf("the dashboard clears the flash cookie: %v", cleared)
	}
	for _, want := range []string{`data-testid="forward-auth-logout-flash"`, "log out at your proxy or SSO provider"} {
		if !strings.Contains(dashboard.Body, want) {
			t.Errorf("the dashboard must render %q", want)
		}
	}
}

func flashCookieSet(res *testutil.Response) *http.Cookie {
	for _, cookie := range res.Cookies() {
		if cookie.Name == "pingward_flash" {
			return cookie
		}
	}
	return nil
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
	wantLocation(t, res, "https://sso.example/logout")
	if sessionRows(t, a) != 0 {
		t.Error("the local session must be deleted whatever the redirect target")
	}
	// "cookies" would reach the whole registrable domain and clear the
	// gateway's own cookies on a sibling subdomain.
	if got := res.Header.Get("Clear-Site-Data"); got != `"cache"` {
		t.Errorf("Clear-Site-Data = %q, want \"cache\"", got)
	}
}

// Whoever the gateway lets in first on an empty instance administers it; later
// arrivals are ordinary users.
func TestTheFirstForwardAuthUserIsAdminAndTheSecondIsNot(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	isAdmin := func(name string) bool {
		a.client.ClearCookies()
		wantStatus(t, a.client.Do("GET", "/projects/new", nil, asUser(name)), http.StatusOK)
		return a.userByName(t, name).IsAdmin
	}
	if !isAdmin("alice") || isAdmin("bob") {
		t.Fatal("alice is the first and so the admin; bob is not")
	}
	if !isAdmin("alice") {
		t.Fatal("signing in again must not change the role")
	}
}

// The rule gates new accounts only: someone provisioned under an older, looser
// rule must not be locked out of the gateway.
func TestAnExistingUserWithANowInvalidNameStillSignsIn(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	for _, name := range []string{strings.Repeat("l", 80), "ta\tb"} {
		if _, err := a.store.CreateUser(t.Context(), name, nil, false, time.Now()); err != nil {
			t.Fatal(err)
		}
	}
	for _, name := range []string{strings.Repeat("l", 80), "ta\tb"} {
		a.client.ClearCookies()
		wantStatus(t, a.client.Do("GET", "/projects/new", nil, asUser(name)), http.StatusOK)
	}
	if countUsers(t, a) != 2 {
		t.Fatal("nothing new may be minted")
	}
}
