package server_test

import (
	"net/http"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/apikey"
	"github.com/henry40408/pingward/internal/secret"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

const resetPassword = "a-brand-new-passphrase"

func (a *app) addAPIKey(t *testing.T, userID int64, name string, expiresAt *time.Time) {
	t.Helper()
	key := apikey.Generate()
	_, err := a.store.InsertAPIKey(t.Context(), store.NewAPIKey{UserID: userID, Name: name, TokenHash: key.Hash, Prefix: key.Prefix, ExpiresAt: expiresAt}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
}

func flashPayloadOf(t *testing.T, res *testutil.Response) (string, bool) {
	t.Helper()
	cookie := flashCookieSet(res)
	if cookie == nil {
		return "", false
	}
	value, err := url.QueryUnescape(cookie.Value)
	if err != nil {
		t.Fatal(err)
	}
	payload, ok := secret.VerifyFlash([]byte(testutil.TestSecret), value)
	if !ok {
		t.Fatalf("the flash cookie is not signed by this server: %q", cookie.Value)
	}
	return payload, true
}

func (a *app) resetMemberPassword(t *testing.T, memberID int64) *testutil.Response {
	t.Helper()
	res := a.post(t, userPath(memberID, "password"), url.Values{"password": {resetPassword}})
	wantLocation(t, res, "/admin")
	return res
}

func TestPasswordResetFlashesAWarningWhenTheTargetHasAPIKeys(t *testing.T) {
	a, memberID := adminApp(t)
	a.addAPIKey(t, memberID, "ci", nil)
	res := a.resetMemberPassword(t, memberID)
	payload, ok := flashPayloadOf(t, res)
	if !ok || !strings.HasPrefix(payload, "password_reset_keys:") {
		t.Fatalf("payload = %q, %v", payload, ok)
	}
	page := a.client.Get("/admin").Body
	if !strings.Contains(page, `data-testid="password-reset-flash"`) || !strings.Contains(page, "1 API key") {
		t.Fatalf("the warning must render: %s", page)
	}
	if strings.Contains(a.client.Get("/admin").Body, `data-testid="password-reset-flash"`) {
		t.Fatal("the flash must render once")
	}
}

func TestPasswordResetHasNoWarningWhenTheTargetHasNoAPIKeys(t *testing.T) {
	a, memberID := adminApp(t)
	res := a.resetMemberPassword(t, memberID)
	if flashCookieSet(res) != nil {
		t.Fatal("no keys, so no flash cookie")
	}
	if strings.Contains(a.client.Get("/admin").Body, `data-testid="password-reset-flash"`) {
		t.Fatal("no warning expected")
	}
}

func TestPasswordResetFlashCountsOnlyLiveAPIKeys(t *testing.T) {
	a, memberID := adminApp(t)
	expired := time.Now().Add(-time.Hour)
	a.addAPIKey(t, memberID, "expired", &expired)
	a.addAPIKey(t, memberID, "live", nil)
	a.resetMemberPassword(t, memberID)
	page := a.client.Get("/admin").Body
	if !strings.Contains(page, `data-testid="password-reset-flash"`) || !strings.Contains(page, "1 API key that continues") {
		t.Fatalf("only the live key counts: %s", page)
	}
}

func TestPasswordResetHasNoWarningWhenOnlyTheKeyIsExpired(t *testing.T) {
	a, memberID := adminApp(t)
	expired := time.Now().Add(-time.Hour)
	a.addAPIKey(t, memberID, "expired", &expired)
	res := a.resetMemberPassword(t, memberID)
	if flashCookieSet(res) != nil || strings.Contains(a.client.Get("/admin").Body, `data-testid="password-reset-flash"`) {
		t.Fatal("an expired key is no residual access")
	}
}

func TestPasswordResetHasNoWarningWhenTheTargetIsDisabled(t *testing.T) {
	a, memberID := adminApp(t)
	a.addAPIKey(t, memberID, "ci", nil)
	if err := a.store.SetUserDisabled(t.Context(), memberID, true); err != nil {
		t.Fatal(err)
	}
	res := a.resetMemberPassword(t, memberID)
	if flashCookieSet(res) != nil || strings.Contains(a.client.Get("/admin").Body, `data-testid="password-reset-flash"`) {
		t.Fatal("a disabled user's keys are refused, so there is nothing to warn about")
	}
}

func TestAnAdminResettingTheirOwnPasswordKeepsOnlyTheCurrentSession(t *testing.T) {
	a, _ := adminApp(t)
	adminID := a.userByName(t, "admin").ID
	other := a.secondSession(t, "admin")
	wantStatus(t, other.Get("/account"), http.StatusOK)
	if sessions, _ := a.store.ListSessionsForUser(t.Context(), adminID, time.Now()); len(sessions) != 2 {
		t.Fatalf("sessions = %d, want 2", len(sessions))
	}
	wantLocation(t, a.post(t, userPath(adminID, "password"), url.Values{"password": {resetPassword}}), "/admin")
	wantStatus(t, a.client.Get("/account"), http.StatusOK)
	wantLocation(t, other.Get("/account"), "/login")
	if sessions, _ := a.store.ListSessionsForUser(t.Context(), adminID, time.Now()); len(sessions) != 1 {
		t.Fatalf("sessions = %d, want 1", len(sessions))
	}
	relogged := a.secondSessionWith(t, "admin", resetPassword)
	wantStatus(t, relogged.Get("/account"), http.StatusOK)
}

func (a *app) secondSessionWith(t *testing.T, username, password string) *testutil.Client {
	t.Helper()
	client := testutil.NewClient(t, newHandler(a.deps))
	csrf := testutil.AnonymousCSRF(t, client, "/login")
	wantLocation(t, client.PostForm("/login", url.Values{"_csrf": {csrf}, "username": {username}, "password": {password}}), "/")
	return client
}

func TestDisablingAMemberRevokesSessionsAndEnablingDoesNotResurrectThem(t *testing.T) {
	a, memberID := adminApp(t)
	a.secondSession(t, "member")
	if sessions, _ := a.store.ListSessionsForUser(t.Context(), memberID, time.Now()); len(sessions) != 1 {
		t.Fatalf("sessions = %d, want 1", len(sessions))
	}
	wantLocation(t, a.post(t, userPath(memberID, "disabled")+"?confirmed=1", url.Values{}), "/admin")
	wantLocation(t, a.post(t, userPath(memberID, "disabled")+"?confirmed=1", url.Values{}), "/admin")
	if a.userByName(t, "member").Disabled {
		t.Fatal("the second toggle enables again")
	}
	if sessions, _ := a.store.ListSessionsForUser(t.Context(), memberID, time.Now()); len(sessions) != 0 {
		t.Fatalf("sessions = %d, want 0", len(sessions))
	}
}

func TestAPlantedUnsignedFlashDoesNotRenderAnyOfTheAdminNotices(t *testing.T) {
	a, _ := adminApp(t)
	cases := map[string][]string{
		"password_reset_keys:1:99": {`data-testid="password-reset-flash"`, "99 API keys"},
		"users_blocked":            {`data-testid="users-flash"`},
	}
	for planted, forbidden := range cases {
		res := a.client.Do("GET", "/admin", nil, http.Header{"Cookie": {"pingward_flash=" + planted}})
		for _, marker := range forbidden {
			if strings.Contains(res.Body, marker) {
				t.Fatalf("an unsigned %q rendered %q", planted, marker)
			}
		}
	}
}

func TestARefusedDuplicateLeavesTheExistingAccountAlone(t *testing.T) {
	a, _ := adminApp(t)
	admin := a.userByName(t, "admin")
	before := *admin.PasswordHash
	count, _ := a.store.CountUsers(t.Context())
	res := a.post(t, "/admin/users", newUserForm("admin", "a-completely-new-phrase"))
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, `data-testid="user-error"`) {
		t.Fatal("the refusal must render in the user-error slot")
	}
	after := a.userByName(t, "admin")
	if *after.PasswordHash != before || !after.IsAdmin {
		t.Fatal("the existing account must be untouched")
	}
	if now, _ := a.store.CountUsers(t.Context()); now != count {
		t.Fatalf("users %d -> %d", count, now)
	}
}

func TestOverlongOrControlCharacterUsernamesAreRefusedAndNameIsStoredTrimmed(t *testing.T) {
	a, _ := adminApp(t)
	wantLocation(t, a.post(t, "/admin/users", newUserForm("  dave  ", testPassword)), "/admin")
	if user, _ := a.store.FindUserByUsername(t.Context(), "  dave  "); user != nil {
		t.Fatal("the untrimmed name must not be stored")
	}
	count, _ := a.store.CountUsers(t.Context())
	for _, bad := range []string{strings.Repeat("a", 65), "bad\x07name"} {
		res := a.post(t, "/admin/users", newUserForm(bad, testPassword))
		wantStatus(t, res, http.StatusOK)
		if !strings.Contains(res.Body, `data-testid="user-error"`) {
			t.Fatalf("%q: no user-error", bad)
		}
	}
	if now, _ := a.store.CountUsers(t.Context()); now != count {
		t.Fatalf("users %d -> %d", count, now)
	}
}

func TestSelfDemotionIsRefusedWithASignedOneShotFlashEvenWithASecondAdmin(t *testing.T) {
	a, _ := adminApp(t)
	a.addUser(t, "gale", true)
	adminID := a.userByName(t, "admin").ID
	res := a.post(t, userPath(adminID, "admin")+"?confirmed=1", url.Values{})
	wantLocation(t, res, "/admin")
	if payload, ok := flashPayloadOf(t, res); !ok || payload != "users_blocked" {
		t.Fatalf("payload = %q, %v", payload, ok)
	}
	if !a.userByName(t, "admin").IsAdmin {
		t.Fatal("self-demotion must be refused even though a second enabled admin exists")
	}
	if !strings.Contains(a.client.Get("/admin").Body, `data-testid="users-flash"`) {
		t.Fatal("the flash must render once")
	}
	if strings.Contains(a.client.Get("/admin").Body, `data-testid="users-flash"`) {
		t.Fatal("the flash must be one-shot")
	}
}

func TestAnOnlyAdminCannotDemoteOrDisableThemselves(t *testing.T) {
	a, _ := adminApp(t)
	adminID := a.userByName(t, "admin").ID
	for _, verb := range []string{"admin", "disabled"} {
		wantLocation(t, a.post(t, userPath(adminID, verb)+"?confirmed=1", url.Values{}), "/admin")
	}
	if user := a.userByName(t, "admin"); !user.IsAdmin || user.Disabled {
		t.Fatalf("account changed: %+v", user)
	}
}
