package server_test

import (
	"net/http"
	"net/url"
	"regexp"
	"strings"
	"testing"

	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/secret"
	"github.com/henry40408/pingward/internal/testutil"
	"github.com/henry40408/pingward/internal/web"
)

var csrfField = regexp.MustCompile(`name="_csrf" value="([^"]*)"`)

func extractCSRF(t *testing.T, html string) string {
	t.Helper()
	match := csrfField.FindStringSubmatch(html)
	if match == nil {
		t.Fatal("a rendered form must embed a _csrf field")
	}
	return match[1]
}

func (a *app) sessionID(t *testing.T) string {
	t.Helper()
	var id string
	if err := a.store.DB.QueryRowContext(t.Context(), "SELECT id FROM sessions LIMIT 1").Scan(&id); err != nil {
		t.Fatal(err)
	}
	return id
}

func withToken(form url.Values, token string) url.Values {
	form.Set("_csrf", token)
	return form
}

// The embedded token alone (no header) authorizes a POST.
func TestFormIncludesCSRFAndFormPostSucceeds(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	token := extractCSRF(t, a.client.Get("/projects/new").Body)
	if token == "" {
		t.Fatal("rendered form must embed a non-empty _csrf token")
	}
	wantStatus(t, a.client.PostForm("/projects", withToken(newProjectForm(), token)), http.StatusSeeOther)
	wantStatus(t, a.client.PostForm("/projects", newProjectForm()), http.StatusForbidden)
}

func TestProtectedPostWithoutTokenIsForbidden(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	res := a.client.PostForm("/projects", newProjectForm())
	wantStatus(t, res, http.StatusForbidden)
	if res.Body != "" {
		t.Errorf("the 403 must be bodyless, got %q", res.Body)
	}
}

func TestProtectedPostWithHeaderTokenSucceeds(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	header := http.Header{"X-Csrf-Token": {testutil.NewestSessionCSRF(t, a.store)}}
	wantStatus(t, a.client.PostRaw("/projects", newProjectForm().Encode(), header), http.StatusSeeOther)
}

func TestProtectedPostWithFormFieldTokenSucceeds(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	form := withToken(newProjectForm(), testutil.NewestSessionCSRF(t, a.store))
	wantStatus(t, a.client.PostForm("/projects", form), http.StatusSeeOther)
}

func TestProtectedPostWithWrongTokenIsForbidden(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	header := http.Header{"X-Csrf-Token": {"not-the-real-token"}}
	wantStatus(t, a.client.PostRaw("/projects", newProjectForm().Encode(), header), http.StatusForbidden)
}

// A wrong header token is refused even when the body carries a right one: the
// header path decides alone.
func TestAWrongHeaderTokenIsNotRescuedByTheFormField(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	form := withToken(newProjectForm(), testutil.NewestSessionCSRF(t, a.store))
	header := http.Header{"X-Csrf-Token": {"wrong"}}
	wantStatus(t, a.client.PostRaw("/projects", form.Encode(), header), http.StatusForbidden)
}

// The guard finds _csrf among repeated channel_ids keys, wherever it sits in
// the body, and restores the body so the handler sees every value. The route
// is a probe: the real channel-binding handler arrives with step 17.
func TestMultiValueFormAuthorizesViaCSRFFieldAndReachesTheHandler(t *testing.T) {
	var seen []string
	probe := web.Route{Method: "POST", Pattern: "/probe", Handler: func(w http.ResponseWriter, r *http.Request) {
		seen = r.PostForm["channel_ids"]
		if len(seen) == 0 {
			_ = r.ParseForm()
			seen = r.PostForm["channel_ids"]
		}
		w.WriteHeader(http.StatusSeeOther)
	}}
	a := newApp(t, nil, probe)
	a.loggedInAdmin(t)
	token := testutil.NewestSessionCSRF(t, a.store)
	body := "channel_ids=1&channel_ids=2&_csrf=" + token
	wantStatus(t, a.client.PostRaw("/probe", body, nil), http.StatusSeeOther)
	if strings.Join(seen, ",") != "1,2" {
		t.Fatalf("the handler must read the restored body, got %v", seen)
	}
	wantStatus(t, a.client.PostRaw("/probe", "channel_ids=1", nil), http.StatusForbidden)
}

// The first _csrf wins: a later valid duplicate cannot rescue a bad first one.
func TestOnlyTheFirstCSRFFieldCounts(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	token := testutil.NewestSessionCSRF(t, a.store)
	body := newProjectForm().Encode() + "&_csrf=bad&_csrf=" + token
	wantStatus(t, a.client.PostRaw("/projects", body, nil), http.StatusForbidden)
}

// /ping/* lives outside the web routes, so the guard never sees it.
func TestPingPostNeedsNoCSRF(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	res := a.client.PostRaw("/ping/does-not-exist", "", nil)
	if res.StatusCode == http.StatusForbidden {
		t.Fatal("the ping router must be exempt from the CSRF guard")
	}
}

// /login is not exempt (login CSRF); the anonymous session supplies the token.
func TestLoginPostWithTheAnonymousTokenSucceeds(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "admin", true)
	csrf := testutil.AnonymousCSRF(t, a.client, "/login")
	form := url.Values{"_csrf": {csrf}, "username": {"admin"}, "password": {testPassword}}
	wantStatus(t, a.client.PostForm("/login", form), http.StatusSeeOther)
}

func TestLoginPostWithoutATokenIsForbidden(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "admin", true)
	testutil.AnonymousCSRF(t, a.client, "/login")
	form := url.Values{"username": {"admin"}, "password": {testPassword}}
	wantStatus(t, a.client.PostForm("/login", form), http.StatusForbidden)
}

// A missing hidden input renders a permanently unsubmittable form, which no
// guard test would catch.
func TestLoginAndSetupPagesEmbedAToken(t *testing.T) {
	a := newApp(t, nil)
	if extractCSRF(t, a.client.Get("/setup").Body) == "" {
		t.Fatal("/setup must embed a token")
	}
	a.addUser(t, "admin", true)
	if extractCSRF(t, a.client.Get("/login").Body) == "" {
		t.Fatal("/login must embed a token")
	}
}

// The anonymous session must cost no sessions row, or a crawler hitting /login
// would grow the table without bound.
func TestAnonymousSessionWritesNoRow(t *testing.T) {
	a := newApp(t, nil)
	if testutil.AnonymousCSRF(t, a.client, "/login") == "" {
		t.Fatal("an anonymous visitor still gets a token")
	}
	var rows int
	if err := a.store.DB.QueryRowContext(t.Context(), "SELECT COUNT(*) FROM sessions").Scan(&rows); err != nil {
		t.Fatal(err)
	}
	if rows != 0 {
		t.Fatalf("sessions rows = %d, want 0", rows)
	}
}

func wantBouncedToLogin(t *testing.T, res *testutil.Response) {
	t.Helper()
	wantStatus(t, res, http.StatusSeeOther)
	if got := res.Header.Get("Location"); got != "/login" {
		t.Fatalf("Location = %q, want /login: the cookie must not authenticate", got)
	}
}

// otherSecretServer is a cookie-less server over the same store whose secret
// differs, simulating a rotating restart.
func otherSecretServer(t *testing.T, a *app, secretText string) *testutil.Client {
	t.Helper()
	cfg := testutil.TestConfig(map[string]string{"PINGWARD_SECRET": secretText})
	deps := web.NewDeps(a.store, cfg, a.deps.Logger, a.deps.Now)
	return testutil.NewClient(t, newHandler(deps))
}

func cookieHeader(name, value string) http.Header {
	return http.Header{"Cookie": {name + "=" + value}}
}

// The cookie is <id>.<hmac>: the bare session id, which a database leak would
// expose, authenticates nothing.
func TestUnsignedSessionIDDoesNotAuthenticate(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	id := a.sessionID(t)
	res := a.client.Do("GET", "/projects/new", nil, cookieHeader(auth.SessionCookieName(false), id))
	wantBouncedToLogin(t, res)
}

// Rotating the secret ends every session via the signature alone; rows survive.
func TestRotatedSecretInvalidatesExistingSessions(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	wantStatus(t, a.client.Get("/projects/new"), http.StatusOK)

	signed := secret.SignSession([]byte(testutil.TestSecret), a.sessionID(t))
	rotated := otherSecretServer(t, a, "a-completely-different-secret")
	wantBouncedToLogin(t, rotated.Do("GET", "/projects/new", nil, cookieHeader(auth.SessionCookieName(false), signed)))

	var rows int
	if err := a.store.DB.QueryRowContext(t.Context(), "SELECT COUNT(*) FROM sessions").Scan(&rows); err != nil {
		t.Fatal(err)
	}
	if rows != 1 {
		t.Fatalf("the session row must survive a rotation, rows = %d", rows)
	}
}

// Rejected even when the session id is identical.
func TestCSRFTokenFromAnotherSecretIsRejected(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	id := a.sessionID(t)
	const other = "a-completely-different-secret"
	rotated := otherSecretServer(t, a, other)
	headers := http.Header{
		"Cookie":       {auth.SessionCookieName(false) + "=" + secret.SignSession([]byte(other), id)},
		"X-Csrf-Token": {secret.DeriveCSRF([]byte(testutil.TestSecret), id)},
	}
	wantStatus(t, rotated.PostRaw("/projects", newProjectForm().Encode(), headers), http.StatusForbidden)
}

// csrfMaxBodyBytes in internal/web: the guard must not buffer more than this.
const csrfBodyCap = 1 << 20

func bodyOfSize(token string, size int) string {
	prefix := "_csrf=" + token + "&name=web&pad="
	return prefix + strings.Repeat("a", size-len(prefix))
}

func TestFormBodyAtTheCapIsAccepted(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	token := testutil.NewestSessionCSRF(t, a.store)
	wantStatus(t, a.client.PostRaw("/projects", bodyOfSize(token, csrfBodyCap), nil), http.StatusSeeOther)
}

// Dropping the cap lets any client make the server buffer an unbounded body.
func TestFormBodyOverTheCapIsRejectedEvenWithAValidToken(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	token := testutil.NewestSessionCSRF(t, a.store)
	res := a.client.PostRaw("/projects", bodyOfSize(token, csrfBodyCap+1), nil)
	wantStatus(t, res, http.StatusForbidden)
	events := a.logs.EventsNamed(t, "csrf.rejected")
	if len(events) != 1 || events[0]["reason"] != "body_unreadable" {
		t.Fatalf("want one body_unreadable rejection, got %v", events)
	}
}

// With the header the body is never read, so it is not capped by the guard.
func TestHeaderTokenPathDoesNotBufferTheBody(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	header := http.Header{"X-Csrf-Token": {testutil.NewestSessionCSRF(t, a.store)}}
	body := bodyOfSize("x", csrfBodyCap+1024)
	res := a.client.PostRaw("/probe-missing", body, header)
	if res.StatusCode == http.StatusForbidden {
		t.Fatal("a valid header token must pass the guard without reading the body")
	}
}

// Secure cookies: the __Host- prefix needs Secure, Path=/ and no Domain, and
// the plain name must not be used (browsers refuse __Host- over HTTP, and the
// reverse would drop the prefix protection).
func TestSecureModeUsesHostPrefixedCookiesWithSecure(t *testing.T) {
	a := newApp(t, map[string]string{"PINGWARD_COOKIE_SECURE": "true"})
	res := a.client.Get("/setup")
	var session *http.Cookie
	for _, cookie := range res.Cookies() {
		if cookie.Name == "pingward_session" {
			t.Fatal("the unprefixed session cookie must not be set in secure mode")
		}
		if cookie.Name == "__Host-pingward_session" {
			session = cookie
		}
	}
	if session == nil {
		t.Fatal("secure mode must set __Host-pingward_session")
	}
	if !session.Secure || !session.HttpOnly || session.Path != "/" || session.Domain != "" {
		t.Fatalf("__Host- cookie needs Secure, HttpOnly, Path=/, no Domain: %v", session)
	}
	if session.SameSite != http.SameSiteLaxMode {
		t.Fatalf("SameSite = %v, want Lax", session.SameSite)
	}
}

func TestPlainModeCookieIsNotSecureAndNotPrefixed(t *testing.T) {
	a := newApp(t, nil)
	cookies := a.client.Get("/setup").Cookies()
	if len(cookies) != 1 {
		t.Fatalf("want exactly the session cookie, got %v", cookies)
	}
	if cookies[0].Name != "pingward_session" || cookies[0].Secure {
		t.Fatalf("plain mode must set an unprefixed, non-Secure cookie, got %v", cookies[0])
	}
}
