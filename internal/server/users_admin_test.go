package server_test

import (
	"net/http"
	"net/url"
	"slices"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

// adminApp is a signed-in, unlocked admin ("admin") plus a member ("member").
func adminApp(t *testing.T) (a *app, memberID int64) {
	t.Helper()
	a = newApp(t, nil)
	a.loggedInAdmin(t)
	memberID = a.addUser(t, "member", false)
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	return a, memberID
}

func (a *app) auditLog(t *testing.T) []models.AuditLog {
	t.Helper()
	entries, err := a.store.ListAudit(t.Context(), 100)
	if err != nil {
		t.Fatal(err)
	}
	return entries
}

// auditActions is oldest first.
func (a *app) auditActions(t *testing.T) []string {
	t.Helper()
	var actions []string
	for _, entry := range a.auditLog(t) {
		actions = append(actions, entry.Action)
	}
	slices.Reverse(actions)
	return actions
}

func (a *app) lastAudit(t *testing.T, action string) models.AuditLog {
	t.Helper()
	for _, entry := range a.auditLog(t) {
		if entry.Action == action {
			return entry
		}
	}
	t.Fatalf("no %q audit entry in %v", action, a.auditActions(t))
	return models.AuditLog{}
}

func (a *app) userByName(t *testing.T, name string) *models.User {
	t.Helper()
	user, err := a.store.FindUserByUsername(t.Context(), name)
	if err != nil || user == nil {
		t.Fatalf("user %q: %v %v", name, user, err)
	}
	return user
}

func userPath(id int64, verb string) string {
	return "/admin/users/" + strconv.FormatInt(id, 10) + "/" + verb
}

func newUserForm(username, password string) url.Values {
	return url.Values{"username": {username}, "password": {password}}
}

func wantLocation(t *testing.T, res *testutil.Response, location string) {
	t.Helper()
	wantStatus(t, res, http.StatusSeeOther)
	if got := res.Header.Get("Location"); got != location {
		t.Fatalf("Location = %q, want %q", got, location)
	}
}

func TestCreatingAUserIsAudited(t *testing.T) {
	a, _ := adminApp(t)
	wantLocation(t, a.post(t, "/admin/users", newUserForm("carol", testPassword)), "/admin")
	carol := a.userByName(t, "carol")
	entry := a.lastAudit(t, "user.create")
	if entry.TargetID == nil || *entry.TargetID != carol.ID || entry.Detail == nil || *entry.Detail != "member" {
		t.Fatalf("audit = %+v", entry)
	}
	form := newUserForm("dave", testPassword)
	form.Set("is_admin", "on")
	wantLocation(t, a.post(t, "/admin/users", form), "/admin")
	if !a.userByName(t, "dave").IsAdmin {
		t.Fatal("the admin box must create an admin")
	}
	if detail := a.lastAudit(t, "user.create").Detail; detail == nil || *detail != "admin" {
		t.Fatalf("detail = %v", detail)
	}
}

func TestCreatingAUserStoresTheTrimmedNameAndRefusesBadNames(t *testing.T) {
	a, _ := adminApp(t)
	wantLocation(t, a.post(t, "/admin/users", newUserForm("  erin  ", testPassword)), "/admin")
	a.userByName(t, "erin")
	for _, form := range []url.Values{newUserForm("   ", testPassword), newUserForm("frank", "short")} {
		res := a.post(t, "/admin/users", form)
		wantStatus(t, res, http.StatusOK)
	}
	if user, _ := a.store.FindUserByUsername(t.Context(), "frank"); user != nil {
		t.Fatal("a short password must not create the user")
	}
}

func TestCreatingAUserWithATakenUsernameIsRefusedWithAMessage(t *testing.T) {
	a, memberID := adminApp(t)
	res := a.post(t, "/admin/users", newUserForm("member", testPassword))
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, `A user named &quot;member&quot; already exists.`) {
		t.Fatalf("message missing: %s", res.Body)
	}
	if a.userByName(t, "member").ID != memberID {
		t.Fatal("the existing account must be left alone")
	}
	if slices.Contains(a.auditActions(t), "user.create") {
		t.Fatal("a refused duplicate must not be audited")
	}
}

func TestAUsernameDifferingOnlyInCaseIsAccepted(t *testing.T) {
	a, _ := adminApp(t)
	wantLocation(t, a.post(t, "/admin/users", newUserForm("Member", testPassword)), "/admin")
	a.userByName(t, "Member")
}

func TestDeletingAUserIsAuditedAndCascadesItsSessions(t *testing.T) {
	a, memberID := adminApp(t)
	if err := a.store.CreateSession(t.Context(), store.NewSession{ID: "member-session", UserID: memberID, ExpiresAt: time.Now().Add(time.Hour)}, time.Now()); err != nil {
		t.Fatal(err)
	}
	confirm := a.post(t, userPath(memberID, "delete"), url.Values{})
	wantStatus(t, confirm, http.StatusOK)
	if !strings.Contains(confirm.Body, "Delete this user?") {
		t.Fatal("the confirmation page must come first")
	}
	wantLocation(t, a.post(t, userPath(memberID, "delete")+"?confirmed=1", url.Values{}), "/admin")
	if user, _ := a.store.FindUserByID(t.Context(), memberID); user != nil {
		t.Fatal("the user must be gone")
	}
	sessions, err := a.store.ListSessionsForUser(t.Context(), memberID, time.Now())
	if err != nil || len(sessions) != 0 {
		t.Fatalf("sessions = %v, %v", sessions, err)
	}
	entry := a.lastAudit(t, "user.delete")
	if entry.TargetID == nil || *entry.TargetID != memberID {
		t.Fatalf("audit = %+v", entry)
	}
}

func TestDeletingANonexistentUserWritesNoAudit(t *testing.T) {
	a, _ := adminApp(t)
	wantLocation(t, a.post(t, userPath(9999, "delete")+"?confirmed=1", url.Values{}), "/admin")
	if slices.Contains(a.auditActions(t), "user.delete") {
		t.Fatal("nothing was deleted, so nothing may be audited")
	}
}

func TestResettingThePasswordOfANonexistentUserWritesNoAudit(t *testing.T) {
	a, _ := adminApp(t)
	wantLocation(t, a.post(t, userPath(9999, "password"), url.Values{"password": {"another-long-passphrase"}}), "/admin")
	if slices.Contains(a.auditActions(t), "user.password_reset") {
		t.Fatal("no such user, so no audit entry")
	}
}

func TestPromoteAndDemoteAdmin(t *testing.T) {
	a, memberID := adminApp(t)
	wantLocation(t, a.post(t, userPath(memberID, "admin"), url.Values{}), "/admin")
	if !a.userByName(t, "member").IsAdmin {
		t.Fatal("promotion must set the flag")
	}
	if detail := a.lastAudit(t, "user.set_admin").Detail; detail == nil || *detail != "promote" {
		t.Fatalf("detail = %v", detail)
	}
	confirm := a.post(t, userPath(memberID, "admin"), url.Values{})
	wantStatus(t, confirm, http.StatusOK)
	if !strings.Contains(confirm.Body, "Revoke this user") {
		t.Fatal("demotion must ask first")
	}
	wantLocation(t, a.post(t, userPath(memberID, "admin")+"?confirmed=1", url.Values{}), "/admin")
	if a.userByName(t, "member").IsAdmin {
		t.Fatal("demotion must clear the flag")
	}
}

func TestAnAdminCannotChangeTheirOwnAccessEvenWithASecondAdmin(t *testing.T) {
	a, memberID := adminApp(t)
	wantLocation(t, a.post(t, userPath(memberID, "admin"), url.Values{}), "/admin") // second admin
	self := a.userByName(t, "admin").ID
	for _, verb := range []string{"admin", "disabled", "delete"} {
		res := a.post(t, userPath(self, verb)+"?confirmed=1", url.Values{})
		wantLocation(t, res, "/admin")
		page := a.client.Get("/admin")
		if !strings.Contains(page.Body, "you cannot remove your own access") {
			t.Fatalf("%s: the refusal flash must show on /admin", verb)
		}
	}
	user := a.userByName(t, "admin")
	if !user.IsAdmin || user.Disabled {
		t.Fatalf("self-targeting changed the account: %+v", user)
	}
}

func TestAdminResetsPasswordRevokesSessionsAndTheTargetCanLogIn(t *testing.T) {
	a, memberID := adminApp(t)
	if err := a.store.CreateSession(t.Context(), store.NewSession{ID: "stolen", UserID: memberID, ExpiresAt: time.Now().Add(time.Hour)}, time.Now()); err != nil {
		t.Fatal(err)
	}
	const fresh = "a-brand-new-passphrase"
	wantLocation(t, a.post(t, userPath(memberID, "password"), url.Values{"password": {fresh}}), "/admin")
	entry := a.lastAudit(t, "user.password_reset")
	if entry.Detail == nil || *entry.Detail != "sessions_revoked=1" {
		t.Fatalf("audit = %+v", entry)
	}
	a.client.ClearCookies()
	csrf := testutil.AnonymousCSRF(t, a.client, "/login")
	res := a.client.PostForm("/login", url.Values{"_csrf": {csrf}, "username": {"member"}, "password": {fresh}})
	wantLocation(t, res, "/")
}

func TestResetRefusesAShortPasswordWithAPage(t *testing.T) {
	a, memberID := adminApp(t)
	res := a.post(t, userPath(memberID, "password"), url.Values{"password": {"short"}})
	wantStatus(t, res, http.StatusOK)
	if slices.Contains(a.auditActions(t), "user.password_reset") {
		t.Fatal("a refused reset must not be audited")
	}
}

func TestDisableAndEnableAMember(t *testing.T) {
	a, memberID := adminApp(t)
	confirm := a.post(t, userPath(memberID, "disabled"), url.Values{})
	wantStatus(t, confirm, http.StatusOK)
	if !strings.Contains(confirm.Body, "Disable this user?") {
		t.Fatal("disabling must ask first")
	}
	wantLocation(t, a.post(t, userPath(memberID, "disabled")+"?confirmed=1", url.Values{}), "/admin")
	if !a.userByName(t, "member").Disabled {
		t.Fatal("the account must be disabled")
	}
	if detail := a.lastAudit(t, "user.set_disabled").Detail; detail == nil || !strings.HasPrefix(*detail, "disable sessions_revoked=") {
		t.Fatalf("detail = %v", detail)
	}
	// Re-enabling needs no ceremony.
	wantLocation(t, a.post(t, userPath(memberID, "disabled"), url.Values{}), "/admin")
	if a.userByName(t, "member").Disabled {
		t.Fatal("the account must be enabled again")
	}
	if detail := a.lastAudit(t, "user.set_disabled").Detail; detail == nil || *detail != "enable" {
		t.Fatalf("detail = %v", detail)
	}
}

func TestAPlantedUnsignedFlashDoesNotRender(t *testing.T) {
	a, _ := adminApp(t)
	for _, planted := range []string{"pingward_flash=users_blocked", "pingward_flash=password_reset_keys:1:1", "pingward_flash=settings.forged"} {
		res := a.client.Do("GET", "/admin", nil, http.Header{"Cookie": {planted}})
		if strings.Contains(res.Body, "was refused") || strings.Contains(res.Body, "API key") || strings.Contains(res.Body, "Settings saved") {
			t.Fatalf("%s rendered a flash", planted)
		}
	}
}
