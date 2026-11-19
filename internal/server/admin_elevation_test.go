package server_test

import (
	"net/http"
	"net/url"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/apikey"
	"github.com/henry40408/pingward/internal/testutil"
)

// lockedAdminApp is a signed-in admin who has not unlocked, plus a member.
func lockedAdminApp(t *testing.T) (a *app, memberID int64) {
	t.Helper()
	a = newApp(t, nil)
	a.loggedInAdmin(t)
	return a, a.addUser(t, "member", false)
}

func (a *app) wantBouncedToUnlock(t *testing.T, res *testutil.Response) {
	t.Helper()
	wantLocation(t, res, "/admin/unlock")
}

func (a *app) sessionHandles(t *testing.T) []string {
	t.Helper()
	rows, err := a.store.DB.QueryContext(t.Context(), "SELECT id FROM sessions ORDER BY rowid")
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = rows.Close() }()
	var handles []string
	for rows.Next() {
		var id string
		if err := rows.Scan(&id); err != nil {
			t.Fatal(err)
		}
		handles = append(handles, apikey.Hash(id))
	}
	return handles
}

func (a *app) handleIsElevated(handle string) bool {
	_, ok := a.deps.Elevations.RemainingSecs(handle, time.Now())
	return ok
}

func TestCreatingAUserIsRefusedWhileLocked(t *testing.T) {
	a, _ := lockedAdminApp(t)
	a.wantBouncedToUnlock(t, a.post(t, "/admin/users", newUserForm("carol", testPassword)))
	if user, _ := a.store.FindUserByUsername(t.Context(), "carol"); user != nil {
		t.Fatal("a refused creation must not write")
	}
	if len(a.auditActions(t)) != 0 {
		t.Fatalf("a refused creation must not be audited: %v", a.auditActions(t))
	}
}

func TestResettingAPasswordIsRefusedWhileLockedBeforeAnythingElse(t *testing.T) {
	a, memberID := lockedAdminApp(t)
	before := a.userByName(t, "member").PasswordHash
	// A short password and an unknown id would each be refused later; the gate
	// is first, so both bounce.
	a.wantBouncedToUnlock(t, a.post(t, userPath(memberID, "password"), url.Values{"password": {"short"}}))
	a.wantBouncedToUnlock(t, a.post(t, userPath(9999, "password"), url.Values{"password": {"another-long-passphrase"}}))
	a.wantBouncedToUnlock(t, a.post(t, userPath(memberID, "password"), url.Values{"password": {"another-long-passphrase"}}))
	after := a.userByName(t, "member").PasswordHash
	if (before == nil) != (after == nil) || (before != nil && *before != *after) {
		t.Fatal("a refused reset must not change the hash")
	}
}

func TestPromotingToAdminIsRefusedWhileLocked(t *testing.T) {
	a, memberID := lockedAdminApp(t)
	a.wantBouncedToUnlock(t, a.post(t, userPath(memberID, "admin"), url.Values{}))
	if a.userByName(t, "member").IsAdmin {
		t.Fatal("a refused promotion must not write")
	}
}

func TestDemotingDisablingAndDeletingWorkWhileLocked(t *testing.T) {
	a, memberID := lockedAdminApp(t)
	if err := a.store.SetUserAdmin(t.Context(), memberID, true); err != nil {
		t.Fatal(err)
	}
	wantLocation(t, a.post(t, userPath(memberID, "admin")+"?confirmed=1", url.Values{}), "/admin")
	if a.userByName(t, "member").IsAdmin {
		t.Fatal("demotion must not need elevation")
	}
	wantLocation(t, a.post(t, userPath(memberID, "disabled")+"?confirmed=1", url.Values{}), "/admin")
	if !a.userByName(t, "member").Disabled {
		t.Fatal("disabling must not need elevation")
	}
	wantLocation(t, a.post(t, userPath(memberID, "delete")+"?confirmed=1", url.Values{}), "/admin")
	if user, _ := a.store.FindUserByID(t.Context(), memberID); user != nil {
		t.Fatal("deleting must not need elevation")
	}
}

func TestUnlockingThenGrantingWorks(t *testing.T) {
	a, memberID := lockedAdminApp(t)
	a.wantBouncedToUnlock(t, a.post(t, userPath(memberID, "admin"), url.Values{}))
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	wantLocation(t, a.post(t, userPath(memberID, "admin"), url.Values{}), "/admin")
	if !a.userByName(t, "member").IsAdmin {
		t.Fatal("the refused action is not replayed, but a new one must work")
	}
}

func TestTheRefusedActionIsNotReplayedAfterUnlocking(t *testing.T) {
	a, _ := lockedAdminApp(t)
	a.wantBouncedToUnlock(t, a.post(t, "/admin/users", newUserForm("carol", testPassword)))
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	if user, _ := a.store.FindUserByUsername(t.Context(), "carol"); user != nil {
		t.Fatal("unlocking must not perform the earlier refused action")
	}
}

func TestADoomedSubmissionIsRefusedWithoutAskingForAPassword(t *testing.T) {
	a, _ := lockedAdminApp(t)
	for _, form := range []url.Values{newUserForm("carol", "short"), newUserForm("member", testPassword), newUserForm("  ", testPassword)} {
		res := a.post(t, "/admin/users", form)
		wantStatus(t, res, http.StatusOK)
	}
}

func TestElevationDoesNotLeakToAnotherSession(t *testing.T) {
	a, memberID := lockedAdminApp(t)
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	other := testutil.NewClient(t, newHandler(a.deps))
	csrf := testutil.AnonymousCSRF(t, other, "/login")
	res := other.PostForm("/login", url.Values{"_csrf": {csrf}, "username": {"admin"}, "password": {testPassword}})
	wantLocation(t, res, "/")
	token := extractCSRF(t, other.Get("/account").Body)
	res = other.PostForm(userPath(memberID, "admin"), url.Values{"_csrf": {token}})
	a.wantBouncedToUnlock(t, res)
	if handles := a.sessionHandles(t); len(handles) != 2 || a.handleIsElevated(handles[1]) {
		t.Fatal("the second session must stay locked")
	}
}

func TestSigningOutAndBackInStartsLocked(t *testing.T) {
	a, memberID := lockedAdminApp(t)
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	handle := a.sessionHandles(t)[0]
	wantStatus(t, a.post(t, "/logout", url.Values{}), http.StatusSeeOther)
	if a.handleIsElevated(handle) {
		t.Fatal("logging out must drop the elevation")
	}
	a.client.ClearCookies()
	a.login(t, "admin")
	a.wantBouncedToUnlock(t, a.post(t, userPath(memberID, "admin"), url.Values{}))
}

func TestTheUnlockPageExplainsTheRequirementAndTheBounceNoticeIsOneShot(t *testing.T) {
	a, memberID := lockedAdminApp(t)
	a.wantBouncedToUnlock(t, a.post(t, userPath(memberID, "admin"), url.Values{}))
	first := a.client.Get("/admin/unlock")
	wantStatus(t, first, http.StatusOK)
	if !strings.Contains(first.Body, "it grants access, so it needs confirming first") {
		t.Fatal("the bounce notice must show once")
	}
	if second := a.client.Get("/admin/unlock"); strings.Contains(second.Body, "it needs confirming first") {
		t.Fatal("the notice must be consumed")
	}
	// Plain navigation never carries the notice.
	if fresh := lockedFresh(t); strings.Contains(fresh, "it needs confirming first") {
		t.Fatal("navigating directly must not show a bounce notice")
	}
}

func lockedFresh(t *testing.T) string {
	a, _ := lockedAdminApp(t)
	return a.client.Get("/admin/unlock").Body
}

func TestOnlyTheGrantingControlsAreMarkedAndOnlyWhileLocked(t *testing.T) {
	a, _ := lockedAdminApp(t)
	locked := a.client.Get("/admin").Body
	marked := strings.Count(locked, "data-reauth")
	if marked == 0 {
		t.Fatal("a locked page must mark its granting controls")
	}
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	if unlocked := a.client.Get("/admin").Body; strings.Contains(unlocked, "data-reauth") {
		t.Fatal("an unlocked page must not mark any control")
	}
}

func TestRevokingASessionDropsItsElevation(t *testing.T) {
	a, _ := lockedAdminApp(t)
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	handle := a.sessionHandles(t)[0]
	a.secondSession(t, "admin")
	res := a.post(t, "/account/sessions/"+handle+"/revoke?confirmed=1", url.Values{})
	wantStatus(t, res, http.StatusSeeOther)
	if a.handleIsElevated(handle) {
		t.Fatal("revoking a session must drop its elevation")
	}
}

// secondSession signs username in on a fresh client and unlocks it when the
// user is an admin.
func (a *app) secondSession(t *testing.T, username string) *testutil.Client {
	t.Helper()
	client := testutil.NewClient(t, newHandler(a.deps))
	csrf := testutil.AnonymousCSRF(t, client, "/login")
	res := client.PostForm("/login", url.Values{"_csrf": {csrf}, "username": {username}, "password": {testPassword}})
	wantLocation(t, res, "/")
	return client
}

func (a *app) unlockClient(t *testing.T, client *testutil.Client) {
	t.Helper()
	token := extractCSRF(t, client.Get("/account").Body)
	wantStatus(t, client.PostForm("/admin/unlock", url.Values{"_csrf": {token}, "password": {testPassword}}), http.StatusSeeOther)
}

func TestRevokingOtherSessionsDropsTheirElevationsOnly(t *testing.T) {
	a, _ := lockedAdminApp(t)
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	other := a.secondSession(t, "admin")
	a.unlockClient(t, other)
	handles := a.sessionHandles(t)
	wantStatus(t, a.post(t, "/account/sessions/revoke-others?confirmed=1", url.Values{}), http.StatusSeeOther)
	if !a.handleIsElevated(handles[0]) || a.handleIsElevated(handles[1]) {
		t.Fatal("only the current session keeps its elevation")
	}
}

func TestChangingOnesPasswordDropsTheOtherSessionsElevations(t *testing.T) {
	a, _ := lockedAdminApp(t)
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	other := a.secondSession(t, "admin")
	a.unlockClient(t, other)
	handles := a.sessionHandles(t)
	res := a.post(t, "/account/password", passwordForm(testPassword, "another-long-passphrase", "another-long-passphrase"))
	wantStatus(t, res, http.StatusSeeOther)
	if !a.handleIsElevated(handles[0]) || a.handleIsElevated(handles[1]) {
		t.Fatal("only the current session keeps its elevation")
	}
}

func TestResettingAnotherUsersPasswordDropsTheirElevations(t *testing.T) {
	a, memberID := lockedAdminApp(t)
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	wantLocation(t, a.post(t, userPath(memberID, "admin"), url.Values{}), "/admin")
	peer := a.secondSession(t, "member")
	a.unlockClient(t, peer)
	peerHandle := a.sessionHandles(t)[1]
	wantLocation(t, a.post(t, userPath(memberID, "password"), url.Values{"password": {"another-long-passphrase"}}), "/admin")
	if a.handleIsElevated(peerHandle) {
		t.Fatal("a reset must drop the target's elevations")
	}
}

func TestResettingOnesOwnPasswordKeepsOnlyTheCurrentElevation(t *testing.T) {
	a, _ := lockedAdminApp(t)
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	other := a.secondSession(t, "admin")
	a.unlockClient(t, other)
	handles := a.sessionHandles(t)
	self := a.userByName(t, "admin").ID
	wantLocation(t, a.post(t, userPath(self, "password"), url.Values{"password": {"another-long-passphrase"}}), "/admin")
	if !a.handleIsElevated(handles[0]) || a.handleIsElevated(handles[1]) {
		t.Fatal("a self-reset spares only the current session")
	}
}

func TestDisablingOrDeletingAUserDropsTheirElevations(t *testing.T) {
	for _, verb := range []string{"disabled", "delete"} {
		a, memberID := lockedAdminApp(t)
		wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
		wantLocation(t, a.post(t, userPath(memberID, "admin"), url.Values{}), "/admin")
		peer := a.secondSession(t, "member")
		a.unlockClient(t, peer)
		peerHandle := a.sessionHandles(t)[1]
		// Demote first: an admin target is the only one with an elevation.
		wantLocation(t, a.post(t, userPath(memberID, verb)+"?confirmed=1", url.Values{}), "/admin")
		if a.handleIsElevated(peerHandle) {
			t.Fatalf("%s must drop the target's elevations", verb)
		}
	}
}

func TestEveryGrantingActionIsAuditedOnlyWhenItRuns(t *testing.T) {
	a, memberID := lockedAdminApp(t)
	a.wantBouncedToUnlock(t, a.post(t, userPath(memberID, "admin"), url.Values{}))
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	wantLocation(t, a.post(t, userPath(memberID, "admin"), url.Values{}), "/admin")
	if got := a.auditActions(t); !slices.Equal(got, []string{"user.set_admin"}) {
		t.Fatalf("actions = %v", got)
	}
}
