package server_test

import (
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/apikey"
	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/ratelimit"
	"github.com/henry40408/pingward/internal/testutil"
)

var fetchHeader = http.Header{"X-Requested-With": {"fetch"}}

func (a *app) unlock(t *testing.T, password string, headers http.Header) *testutil.Response {
	t.Helper()
	form := withToken(url.Values{"password": {password}}, accountToken(t, a))
	return a.client.PostRaw("/admin/unlock", form.Encode(), headers)
}

func (a *app) isElevated(t *testing.T) bool {
	t.Helper()
	_, ok := a.deps.Elevations.RemainingSecs(apikey.Hash(a.sessionID(t)), time.Now())
	return ok
}

func TestUnlockPageRendersForAnAdmin(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	wantStatus(t, a.client.Get("/admin/unlock"), http.StatusOK)
}

func TestCorrectPasswordUnlocksAndRedirectsToAdmin(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	res := a.unlock(t, testPassword, nil)
	wantStatus(t, res, http.StatusSeeOther)
	if got := res.Header.Get("Location"); got != "/admin" {
		t.Fatalf("Location = %q", got)
	}
	if !a.isElevated(t) {
		t.Fatal("the session must be elevated")
	}
}

func TestWrongPasswordRendersTheFormAndDoesNotElevate(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	res := a.unlock(t, "nope", nil)
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "That password is not correct.") {
		t.Error("the refusal must be shown")
	}
	if a.isElevated(t) {
		t.Fatal("a wrong password must not elevate")
	}
	events := a.logs.EventsNamed(t, "reauth.failed")
	if len(events) != 1 || events[0]["surface"] != "admin_unlock" || events[0]["reason"] != "bad_current_password" {
		t.Fatalf("events = %v", events)
	}
}

func TestFetchUnlockAnswersWithStatusOnly(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	wantStatus(t, a.unlock(t, "nope", fetchHeader), http.StatusForbidden)
	wantStatus(t, a.unlock(t, testPassword, fetchHeader), http.StatusNoContent)
	if !a.isElevated(t) {
		t.Fatal("the session must be elevated")
	}
}

func TestFetchUnlockIsThrottledWithRetryAfter(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	for range ratelimit.AccountMaxAttempts {
		wantStatus(t, a.unlock(t, "nope", fetchHeader), http.StatusForbidden)
	}
	res := a.unlock(t, testPassword, fetchHeader)
	wantThrottled(t, res, ratelimit.AccountWindow)
	if a.isElevated(t) {
		t.Fatal("a throttled unlock must not elevate, even with the right password")
	}
}

func TestOnlyAnAdminMayUnlock(t *testing.T) {
	a, _ := memberApp(t)
	res := a.unlock(t, testPassword, nil)
	if res.StatusCode == http.StatusSeeOther && res.Header.Get("Location") == "/admin" {
		t.Fatal("a member must not be sent on to /admin")
	}
	if a.isElevated(t) {
		t.Fatal("a member must not be elevated")
	}
}

func TestPasswordlessAdminUnlocksWithoutAPassword(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	cookie := signedInViaGateway(t, a)
	header := http.Header{"X-Forwarded-User": {"alice"}, "Cookie": {cookie.Name + "=" + cookie.Value}}
	page := a.client.Do("GET", "/admin/unlock", nil, header)
	wantStatus(t, page, http.StatusOK)
	form := url.Values{"_csrf": {extractCSRF(t, page.Body)}}
	header.Set("Content-Type", "application/x-www-form-urlencoded")
	res := a.client.Do("POST", "/admin/unlock", strings.NewReader(form.Encode()), header)
	wantStatus(t, res, http.StatusSeeOther)
	if _, ok := a.deps.Elevations.RemainingSecs(apikey.Hash(a.sessionID(t)), time.Now()); !ok {
		t.Fatal("a passwordless admin has nothing to re-authenticate against")
	}
}

func TestElevationEndsWithItsTTL(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	a.unlock(t, testPassword, nil)
	handle := apikey.Hash(a.sessionID(t))
	if _, ok := a.deps.Elevations.RemainingSecs(handle, time.Now().Add(15*time.Minute+time.Second)); ok {
		t.Fatal("the grant must lapse after its TTL")
	}
}

func TestSetupRefusesAPasswordUnderTheFloor(t *testing.T) {
	a := newApp(t, nil)
	a.deps.SetupToken.Arm(testutil.TestSetupToken)
	csrf := testutil.AnonymousCSRF(t, a.client, "/setup")
	res := a.client.PostForm("/setup", url.Values{
		"_csrf": {csrf}, "setup_token": {testutil.TestSetupToken}, "username": {"admin"}, "password": {"short"},
	})
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "setup-error") || !strings.Contains(res.Body, "at least "+strconv.Itoa(auth.MinPasswordChars)+" characters") {
		t.Fatalf("the message must say what the requirement is: %s", res.Body)
	}
	if countUsers(t, a) != 0 {
		t.Fatal("a rejected /setup must not create the admin")
	}
}

func TestSetupStillReportsAMissingUsernameAsAPair(t *testing.T) {
	a := newApp(t, nil)
	a.deps.SetupToken.Arm(testutil.TestSetupToken)
	csrf := testutil.AnonymousCSRF(t, a.client, "/setup")
	res := a.client.PostForm("/setup", url.Values{
		"_csrf": {csrf}, "setup_token": {testutil.TestSetupToken}, "username": {""}, "password": {strings.Repeat("a", 20)},
	})
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "username and password are required") || countUsers(t, a) != 0 {
		t.Fatal("want the combined message and no user")
	}
}

func TestSetupAcceptsAPasswordAtTheFloor(t *testing.T) {
	a := newApp(t, nil)
	a.deps.SetupToken.Arm(testutil.TestSetupToken)
	csrf := testutil.AnonymousCSRF(t, a.client, "/setup")
	res := a.client.PostForm("/setup", url.Values{
		"_csrf": {csrf}, "setup_token": {testutil.TestSetupToken}, "username": {"admin"}, "password": {strings.Repeat("a", auth.MinPasswordChars)},
	})
	wantStatus(t, res, http.StatusSeeOther)
	if countUsers(t, a) != 1 {
		t.Fatal("the admin must be created")
	}
}

func TestSessionLifecycleIsLoggedWithoutLeakingTheRawID(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "alice", false)
	a.login(t, "alice")
	raw := a.sessionID(t)
	wantStatus(t, a.client.PostForm("/logout", url.Values{"_csrf": {testutil.NewestSessionCSRF(t, a.store)}}), http.StatusSeeOther)
	created := a.logs.EventsNamed(t, "session.created")
	destroyed := a.logs.EventsNamed(t, "session.destroyed")
	if len(created) != 1 || created[0]["sso"] != false {
		t.Fatalf("session.created = %v", created)
	}
	if len(destroyed) != 1 || destroyed[0]["reason"] != "logout" {
		t.Fatalf("session.destroyed = %v", destroyed)
	}
	log := a.logs.String()
	if !strings.Contains(log, auth.SessionLogHandle(raw)) {
		t.Error("the log handle must appear")
	}
	if strings.Contains(log, raw) {
		t.Error("the raw session id must never be logged")
	}
}
