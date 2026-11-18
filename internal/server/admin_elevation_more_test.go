package server_test

import (
	"net/http"
	"net/url"
	"strings"
	"testing"

	"github.com/henry40408/pingward/internal/ratelimit"
)

func TestTheUnlockPageExplainsTheRequirement(t *testing.T) {
	a, _ := lockedAdminApp(t)
	body := a.client.Get("/admin/unlock").Body
	for _, want := range []string{
		"unlock-input", "unlock-gated", "<strong>granting admin rights</strong>",
		"disabling, demoting, deleting", "not a second factor", "15m", "unlock-cancel",
	} {
		if !strings.Contains(body, want) {
			t.Errorf("the page must contain %q", want)
		}
	}
	if strings.Contains(body, "badge") {
		t.Error("the requirement is named with <strong>, not a status pill")
	}
}

func TestTheWrongPasswordDoesNotUnlockAndKeepsTheForm(t *testing.T) {
	a, memberID := lockedAdminApp(t)
	res := a.unlock(t, "not-the-password", nil)
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "That password is not correct.") || !strings.Contains(res.Body, "unlock-input") {
		t.Fatal("the error and the form must re-render")
	}
	a.wantBouncedToUnlock(t, a.post(t, userPath(memberID, "admin")+"?confirmed=1", url.Values{}))
	if a.userByName(t, "member").IsAdmin {
		t.Fatal("a failed unlock must not have elevated anything")
	}
}

func TestRepeatedWrongUnlocksExhaustTheAccountBudgetOnThePage(t *testing.T) {
	a, _ := lockedAdminApp(t)
	for range ratelimit.AccountMaxAttempts {
		wantStatus(t, a.unlock(t, "not-the-password", nil), http.StatusOK)
	}
	res := a.unlock(t, testPassword, nil)
	if !strings.Contains(res.Body, "Too many attempts") {
		t.Fatalf("the right password must not get past the budget: %s", res.Body)
	}
}

func TestFetchUnlockHasNoBodyOnRefusalAndTheGrantThenWorks(t *testing.T) {
	a, memberID := lockedAdminApp(t)
	wrong := a.unlock(t, "not-the-password", fetchHeader)
	wantStatus(t, wrong, http.StatusForbidden)
	if wrong.Body != "" {
		t.Fatalf("there is no page to render into a dialog: %q", wrong.Body)
	}
	wantStatus(t, a.unlock(t, testPassword, fetchHeader), http.StatusNoContent)
	wantLocation(t, a.post(t, userPath(memberID, "admin")+"?confirmed=1", url.Values{}), "/admin")
	if !a.userByName(t, "member").IsAdmin {
		t.Fatal("the grant must work after a fetch unlock")
	}
}

func TestAdminLinksToTheUnlockPageWhileLocked(t *testing.T) {
	a, _ := lockedAdminApp(t)
	body := a.client.Get("/admin").Body
	if !strings.Contains(body, "elevation-confirm-link") || !strings.Contains(body, "/admin/unlock") {
		t.Fatalf("the locked admin page must link to the unlock page: %s", body)
	}
}

func TestConfirmingDoesNotClaimTheRefusedActionSucceeded(t *testing.T) {
	a, _ := lockedAdminApp(t)
	a.wantBouncedToUnlock(t, a.post(t, "/admin/users", newUserForm("carol", testPassword)))
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	body := a.client.Get("/admin").Body
	if !strings.Contains(body, "elevation-flash") || !strings.Contains(body, "was not performed") {
		t.Fatalf("the confirmation must say the action was not performed: %s", body)
	}
	if strings.Contains(body, "Creating a user, resetting") {
		t.Fatal("the confirmation must not list the gated actions")
	}
	if user, _ := a.store.FindUserByUsername(t.Context(), "carol"); user != nil {
		t.Fatal("nothing was created")
	}
}

func TestADoomedDuplicateIsRefusedWithoutABounce(t *testing.T) {
	a, _ := lockedAdminApp(t)
	res := a.post(t, "/admin/users", newUserForm("admin", testPassword))
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "already exists") {
		t.Fatal("the refusal must say why")
	}
	if strings.Contains(a.client.Get("/admin/unlock").Body, "unlock-bounced") {
		t.Fatal("validation runs before the gate, so there is no bounce")
	}
	if count, _ := a.store.CountUsers(t.Context()); count != 2 {
		t.Fatalf("users = %d, want 2", count)
	}
}

func TestAPasswordlessAdminSeesTheNotApplicablePageAndTheGateIsInert(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	cookie := signedInViaGateway(t, a)
	header := http.Header{"X-Forwarded-User": {"alice"}, "Cookie": {cookie.Name + "=" + cookie.Value}}
	page := a.client.Do("GET", "/admin/unlock", nil, header).Body
	if !strings.Contains(page, "unlock-not-applicable") || strings.Contains(page, "unlock-input") {
		t.Fatalf("the page must say it does not apply: %s", page)
	}
	if strings.Contains(a.client.Do("GET", "/admin", nil, header).Body, "elevation-state") {
		t.Fatal("no elevation state for a passwordless admin")
	}
	memberID := a.addUser(t, "dave", false)
	csrf := extractCSRF(t, page)
	header.Set("Content-Type", "application/x-www-form-urlencoded")
	form := url.Values{"_csrf": {csrf}}
	res := a.client.Do("POST", userPath(memberID, "admin")+"?confirmed=1", strings.NewReader(form.Encode()), header)
	wantLocation(t, res, "/admin")
	if !a.userByName(t, "dave").IsAdmin {
		t.Fatal("the gate is inert, not merely hidden")
	}
}

func TestOnlyTheGrantingControlsCarryTheExactMarkersWhileLocked(t *testing.T) {
	a, memberID := lockedAdminApp(t)
	body := a.client.Get("/admin").Body
	if !strings.Contains(body, `data-reauth="create this user"`) || !strings.Contains(body, `data-reauth="grant admin rights"`) {
		t.Fatalf("granting controls must be marked: %s", body)
	}
	users, _ := a.store.CountUsers(t.Context())
	if got := int64(strings.Count(body, `data-reauth="reset this user`)); got != users {
		t.Fatalf("reset controls = %d, want one per row (%d)", got, users)
	}
	if err := a.store.SetUserAdmin(t.Context(), memberID, true); err != nil {
		t.Fatal(err)
	}
	if strings.Contains(a.client.Get("/admin").Body, `data-reauth="grant admin rights"`) {
		t.Fatal("demoting shares the route and must not be marked")
	}
}
