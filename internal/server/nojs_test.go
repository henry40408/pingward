package server_test

import (
	"fmt"
	"net/http"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/apikey"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

// noJSApp is a signed-in admin whose requests carry a valid CSRF header, so a
// refusal below can only be the confirmation gate.
func noJSApp(t *testing.T) *app {
	t.Helper()
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	return a
}

// postToken posts an empty body with the CSRF header only: the confirm gate
// reads its flag from the query because some forms post no body.
func (a *app) postToken(path string) *testutil.Response {
	return a.client.PostRaw(path, "", a.csrfHeader())
}

func (a *app) checkFor(t *testing.T, ownerID int64, uuid string) (projectID, checkID int64) {
	t.Helper()
	projectID, err := a.store.CreateProject(t.Context(), ownerID, store.ProjectSettings{Name: "web"}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return projectID, a.addCheck(t, projectID, "backup", uuid)
}

func (a *app) member(t *testing.T, name string) int64 {
	t.Helper()
	return a.addUser(t, name, false)
}

func (a *app) adminID(t *testing.T) int64 { return a.userByName(t, "admin").ID }

// --- rows reach their page without a click handler ---

func TestDashboardCheckRowsCarryARealLink(t *testing.T) {
	a := noJSApp(t)
	_, checkID := a.checkFor(t, a.adminID(t), "cu")
	wantContains(t, a.dashboardBody(t, ""), fmt.Sprintf(`href="/checks/%d"`, checkID), "dashboard row anchor")
}

func TestProjectCheckRowsCarryARealLink(t *testing.T) {
	a := noJSApp(t)
	projectID, checkID := a.checkFor(t, a.adminID(t), "cu")
	wantContains(t, a.projectBody(t, projectID), fmt.Sprintf(`href="/checks/%d"`, checkID), "project row anchor")
}

func TestAdminProjectRowsCarryARealLink(t *testing.T) {
	a := noJSApp(t)
	projectID, _ := a.checkFor(t, a.adminID(t), "cu")
	res := a.client.Get("/admin")
	wantStatus(t, res, http.StatusOK)
	wantContains(t, res.Body, fmt.Sprintf(`href="/admin/projects/%d"`, projectID), "admin project row anchor")
}

// role="link" gives focus and Enter but still leaves the row dead without JS.
func TestRowsDoNotSimulateALinkWithAria(t *testing.T) {
	a := noJSApp(t)
	a.checkFor(t, a.adminID(t), "cu")
	for _, path := range []string{"/", "/admin"} {
		res := a.client.Get(path)
		wantStatus(t, res, http.StatusOK)
		wantAbsent(t, res.Body, `role="link"`, path+" fakes a link instead of rendering one")
	}
}

// --- fragment endpoints answer a navigation with a page ---

func (a *app) wantRedirect(t *testing.T, path, location string) {
	t.Helper()
	wantLocation(t, a.client.Get(path), location)
}

func TestPingsFragmentRedirectsARealNavigationToTheCheckPage(t *testing.T) {
	a := noJSApp(t)
	_, checkID := a.checkFor(t, a.adminID(t), "cu")
	id := itoa(checkID)
	a.wantRedirect(t, "/checks/"+id+"/pings", "/checks/"+id+"#pings-section")
}

// Dropping the query would break the unscripted "Older" pager.
func TestTheRedirectCarriesTheCursorAndFilter(t *testing.T) {
	a := noJSApp(t)
	_, checkID := a.checkFor(t, a.adminID(t), "cu")
	id := itoa(checkID)
	a.wantRedirect(t, "/checks/"+id+"/pings?pb=42&pk=fail", "/checks/"+id+"?pb=42&pk=fail#pings-section")
}

func TestNotificationsFragmentRedirectsARealNavigation(t *testing.T) {
	a := noJSApp(t)
	_, checkID := a.checkFor(t, a.adminID(t), "cu")
	id := itoa(checkID)
	a.wantRedirect(t, "/checks/"+id+"/notifications", "/checks/"+id+"#notifs-section")
}

func TestAdminCheckFragmentsRedirectWithinTheAdminPrefix(t *testing.T) {
	a := noJSApp(t)
	_, checkID := a.checkFor(t, a.adminID(t), "cu")
	id := itoa(checkID)
	a.wantRedirect(t, "/admin/checks/"+id+"/pings", "/admin/checks/"+id+"#pings-section")
	a.wantRedirect(t, "/admin/checks/"+id+"/notifications", "/admin/checks/"+id+"#notifs-section")
}

func TestAuditFragmentRedirectsARealNavigationToAdmin(t *testing.T) {
	a := noJSApp(t)
	a.checkFor(t, a.adminID(t), "cu")
	a.wantRedirect(t, "/admin/audit?aaction=admin.access", "/admin?aaction=admin.access#audit-section")
}

func TestAFetchCallerStillGetsTheBareFragment(t *testing.T) {
	a := noJSApp(t)
	_, checkID := a.checkFor(t, a.adminID(t), "cu")
	res := a.client.Do(http.MethodGet, "/checks/"+itoa(checkID)+"/pings", nil, http.Header{"X-Requested-With": {"fetch"}})
	wantStatus(t, res, http.StatusOK)
	wantContains(t, res.Body, `data-testid="pings-filters"`, "fetch caller gets the fragment")
	wantAbsent(t, res.Body, "<title>", "fetch caller must not get a whole page")
}

// Ownership is resolved before the redirect, so it cannot leak existence.
func TestTheRedirectNeverAnswersForAnotherUsersCheck(t *testing.T) {
	a := noJSApp(t)
	_, checkID := a.checkFor(t, a.member(t, "other"), "cu2")
	id := itoa(checkID)
	wantStatus(t, a.client.Get("/checks/"+id+"/pings"), http.StatusNotFound)
	wantStatus(t, a.client.Get("/checks/"+id+"/notifications"), http.StatusNotFound)
}

// --- absolute timestamps read as text, not as a machine stamp ---

// The .localtime[data-ts] fallback text must be the readable UTC form, not a
// Go time String (nanoseconds) or raw RFC3339.
func TestAbsoluteTimestampsFallBackToReadableUTC(t *testing.T) {
	a := noJSApp(t)
	if err := a.store.SetSetting(t.Context(), "last_scan_at", time.Now().UTC().Format(time.RFC3339Nano)); err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{"/account", "/admin"} {
		body := a.client.Get(path).Body
		// Split on the class name, not class="localtime": heartbeat tiles are
		// class="hb-time localtime".
		var texts []string
		for _, part := range strings.Split(body, "localtime")[1:] {
			_, rest, ok := strings.Cut(part, ">")
			if !ok {
				continue
			}
			if text, _, ok := strings.Cut(rest, "<"); ok {
				texts = append(texts, text)
			}
		}
		if len(texts) == 0 {
			t.Fatalf("%s renders no localizable timestamps: the assertion would be vacuous", path)
		}
		for _, text := range texts {
			// Index 10 is a space in the readable form, a T in RFC3339 (UTC has
			// a T too, so a plain "no T" check would not work).
			if !strings.HasSuffix(text, " UTC") || strings.Contains(text, ".") || len(text) < 11 || text[10] != ' ' {
				t.Errorf("%s renders an unformatted timestamp fallback: %q", path, text)
			}
		}
	}
}

// --- the scheduler heartbeat states its own age ---

func TestTheSchedulerHeartbeatRendersItsAgeServerSide(t *testing.T) {
	a := noJSApp(t)
	stamp := time.Now().UTC().Add(-90 * time.Minute).Format(time.RFC3339Nano)
	if err := a.store.SetSetting(t.Context(), "last_scan_at", stamp); err != nil {
		t.Fatal(err)
	}
	body := a.client.Get("/admin").Body
	wantContains(t, body, `class="hb-ago" data-ago=`, "heartbeat age element")
	wantContains(t, body, "1h ago", "server-rendered age")
}

func TestAnUnparseableHeartbeatStampRendersNoAge(t *testing.T) {
	a := noJSApp(t)
	if err := a.store.SetSetting(t.Context(), "last_scan_at", "not-a-date"); err != nil {
		t.Fatal(err)
	}
	wantContains(t, a.client.Get("/admin").Body, `class="hb-ago" data-ago="not-a-date"></div>`, "an unparseable stamp leaves the age empty")
}

// --- the light palette exists twice, and must stay identical ---

// paletteAfter returns the "--token: value" pairs in the first block after
// marker (blocks do not nest).
func paletteAfter(t *testing.T, css, marker string) []string {
	t.Helper()
	start := strings.Index(css, marker)
	if start < 0 {
		t.Fatalf("%s is gone from app.css", marker)
	}
	rest := css[start+len(marker):]
	end := strings.Index(rest, "}")
	if end < 0 {
		t.Fatal("unterminated block")
	}
	var tokens []string
	for _, line := range strings.Split(rest[:end], "\n") {
		line = strings.TrimSpace(line)
		line, ok := strings.CutPrefix(line, "--")
		if !ok {
			continue
		}
		key, value, ok := strings.Cut(line, ":")
		if !ok {
			continue
		}
		tokens = append(tokens, strings.TrimSpace(key)+"="+strings.TrimSuffix(strings.TrimSpace(value), ";"))
	}
	return tokens
}

// A selector list cannot span a media query, so the light palette exists for
// [data-theme="light"] and for the scriptless prefers-color-scheme case.
func TestTheTwoLightPalettesAreIdentical(t *testing.T) {
	source, err := os.ReadFile("../assets/static/app.css")
	if err != nil {
		t.Fatal(err)
	}
	css := string(source)
	scripted := paletteAfter(t, css, `:root[data-theme="light"] {`)
	scriptless := paletteAfter(t, css, `:root:not([data-theme]) {`)
	if len(scripted) <= 20 {
		t.Fatalf("parsed only %d tokens: the marker probably moved and this test passes vacuously", len(scripted))
	}
	if strings.Join(scripted, "\n") != strings.Join(scriptless, "\n") {
		t.Errorf("the scripted and scriptless light palettes have drifted\nscripted:   %v\nscriptless: %v", scripted, scriptless)
	}
}

// --- the filter forms submit on their own ---

// A GET submit replaces the whole query string, so each form must re-send the
// other section's filter.
func TestEachFilterFormCarriesTheOtherSectionsFilter(t *testing.T) {
	a := noJSApp(t)
	_, checkID := a.checkFor(t, a.adminID(t), "cu")
	id := itoa(checkID)
	body := a.client.Get("/checks/" + id + "?pk=fail&ne=down").Body

	wantContains(t, body, `<input type="hidden" name="ne" value="down">`, "pings form keeps the notifications filter")
	wantContains(t, body, `<input type="hidden" name="pk" value="fail">`, "notifications form keeps the pings filter")
	// Exactly once each: a form re-sending its own keys as hidden state would
	// submit a stale duplicate next to the live control.
	for _, key := range []string{"pk", "ne"} {
		if n := strings.Count(body, `<input type="hidden" name="`+key+`"`); n != 1 {
			t.Errorf("%s is carried as hidden state %d times, want once (by the other form)", key, n)
		}
	}
	wantContains(t, body, "/checks/"+id+"/pings?ne=down", "the pings Clear link keeps the notifications filter")
	wantContains(t, body, "/checks/"+id+"/notifications?pk=fail", "the notifications Clear link keeps the pings filter")
}

// The forms target the page, not the fragment endpoint.
func TestTheFilterFormsAreRealGETForms(t *testing.T) {
	a := noJSApp(t)
	_, checkID := a.checkFor(t, a.adminID(t), "cu")
	id := itoa(checkID)
	body := a.client.Get("/checks/" + id).Body
	wantContains(t, body, `method="get" action="/checks/`+id+`"`, "a filter form GET action")
	wantAbsent(t, body, `type="button" data-apply`, "a Filter button that submits nothing")
	for _, name := range []string{"pk", "pfrom", "pto", "ne", "ns", "nfrom", "nto"} {
		wantContains(t, body, `name="`+name+`"`, "filter control "+name+" must be named to submit")
	}
}

// --- irreversible actions ask before they run ---
//
// data-confirm is JS-only, so these actions run only with ?confirmed=1 and
// otherwise render a confirmation page. Tests assert both halves, since a gate
// refusing everything would pass either alone.

func wantConfirmationPage(t *testing.T, res *testutil.Response) {
	t.Helper()
	wantStatus(t, res, http.StatusOK)
	wantContains(t, res.Body, `data-testid="confirm-submit"`, "confirmation page")
}

func TestDeletingACheckAsksFirstAndDeletesOnlyWhenConfirmed(t *testing.T) {
	a := noJSApp(t)
	_, checkID := a.checkFor(t, a.adminID(t), "cu")
	path := "/checks/" + itoa(checkID) + "/delete"

	res := a.postToken(path)
	wantConfirmationPage(t, res)
	wantContains(t, res.Body, `action="`+path+`?confirmed=1"`, "the page offers to complete the action")
	a.wantCheckExists(t, checkID, true, "deleted before anyone confirmed")

	wantStatus(t, a.postToken(path+"?confirmed=1"), http.StatusSeeOther)
	a.wantCheckExists(t, checkID, false, "the confirmed delete did not go through")
}

func (a *app) wantCheckExists(t *testing.T, checkID int64, want bool, why string) {
	t.Helper()
	check, err := a.store.FindCheck(t.Context(), checkID)
	if err != nil {
		t.Fatal(err)
	}
	if (check != nil) != want {
		t.Fatalf("check %d exists = %t, want %t: %s", checkID, check != nil, want, why)
	}
}

func (a *app) wantProjectExists(t *testing.T, projectID int64, want bool, why string) {
	t.Helper()
	project, err := a.store.FindProject(t.Context(), projectID)
	if err != nil {
		t.Fatal(err)
	}
	if (project != nil) != want {
		t.Fatalf("project %d exists = %t, want %t: %s", projectID, project != nil, want, why)
	}
}

func (a *app) wantChannelExists(t *testing.T, channelID int64, want bool, why string) {
	t.Helper()
	channel, err := a.store.FindChannel(t.Context(), channelID)
	if err != nil {
		t.Fatal(err)
	}
	if (channel != nil) != want {
		t.Fatalf("channel %d exists = %t, want %t: %s", channelID, channel != nil, want, why)
	}
}

func (a *app) wantUserExists(t *testing.T, userID int64, want bool, why string) {
	t.Helper()
	user, err := a.store.FindUserByID(t.Context(), userID)
	if err != nil {
		t.Fatal(err)
	}
	if (user != nil) != want {
		t.Fatalf("user %d exists = %t, want %t: %s", userID, user != nil, want, why)
	}
}

func (a *app) pingUUID(t *testing.T, checkID int64) string {
	t.Helper()
	check, err := a.store.FindCheck(t.Context(), checkID)
	if err != nil || check == nil {
		t.Fatalf("check %d: %v %v", checkID, check, err)
	}
	return check.PingUUID
}

func TestDeletingAProjectAsksFirst(t *testing.T) {
	a := noJSApp(t)
	projectID, _ := a.checkFor(t, a.adminID(t), "cu")
	path := "/projects/" + itoa(projectID) + "/delete"

	wantConfirmationPage(t, a.postToken(path))
	a.wantProjectExists(t, projectID, true, "deleted before anyone confirmed")
	wantStatus(t, a.postToken(path+"?confirmed=1"), http.StatusSeeOther)
	a.wantProjectExists(t, projectID, false, "the confirmed delete did not go through")
}

// Irreversible too: it breaks every job still pinging the old URL.
func TestRegeneratingAPingURLAsksFirst(t *testing.T) {
	a := noJSApp(t)
	_, checkID := a.checkFor(t, a.adminID(t), "cu")
	path := "/checks/" + itoa(checkID) + "/regenerate"

	wantConfirmationPage(t, a.postToken(path))
	if a.pingUUID(t, checkID) != "cu" {
		t.Fatal("the ping URL changed before anyone confirmed")
	}
	wantStatus(t, a.postToken(path+"?confirmed=1"), http.StatusSeeOther)
	if a.pingUUID(t, checkID) == "cu" {
		t.Error("the confirmed regenerate did not change the ping URL")
	}
}

func TestDeletingAUserAsksFirst(t *testing.T) {
	a := noJSApp(t)
	victim := a.member(t, "victim")

	wantConfirmationPage(t, a.postToken(userPath(victim, "delete")))
	a.wantUserExists(t, victim, true, "deleted before anyone confirmed")
	wantStatus(t, a.postToken(userPath(victim, "delete")+"?confirmed=1"), http.StatusSeeOther)
	a.wantUserExists(t, victim, false, "the confirmed delete did not go through")
}

func (a *app) userFlags(t *testing.T, userID int64) (disabled, isAdmin bool) {
	t.Helper()
	user, err := a.store.FindUserByID(t.Context(), userID)
	if err != nil || user == nil {
		t.Fatalf("user %d: %v %v", userID, user, err)
	}
	return user.Disabled, user.IsAdmin
}

// Mirrors where the template renders data-confirm.
func TestTheUserTogglesAskOnlyInTheDirectionThatTakesAccessAway(t *testing.T) {
	a := noJSApp(t)
	member := a.member(t, "member")

	wantConfirmationPage(t, a.postToken(userPath(member, "disabled")))
	if disabled, _ := a.userFlags(t, member); disabled {
		t.Fatal("disabled without confirming")
	}
	wantStatus(t, a.postToken(userPath(member, "disabled")+"?confirmed=1"), http.StatusSeeOther)
	if disabled, _ := a.userFlags(t, member); !disabled {
		t.Fatal("the confirmed disable did not go through")
	}

	// Re-enabling does not ask.
	wantStatus(t, a.postToken(userPath(member, "disabled")), http.StatusSeeOther)
	if disabled, _ := a.userFlags(t, member); disabled {
		t.Fatal("re-enabling did not go through")
	}

	// Promoting needs elevation, not confirmation; unlock so the redirect below
	// is the promotion, not the elevation bounce.
	wantStatus(t, a.unlock(t, testPassword, a.csrfHeader()), http.StatusSeeOther)
	wantStatus(t, a.postToken(userPath(member, "admin")), http.StatusSeeOther)
	if _, isAdmin := a.userFlags(t, member); !isAdmin {
		t.Fatal("promotion did not go through")
	}

	wantConfirmationPage(t, a.postToken(userPath(member, "admin")))
	if _, isAdmin := a.userFlags(t, member); !isAdmin {
		t.Fatal("demoted without confirming")
	}
}

func TestRevokingAnAPIKeyAsksFirst(t *testing.T) {
	a := noJSApp(t)
	userID := a.adminID(t)
	keyID, err := a.store.InsertAPIKey(t.Context(), store.NewAPIKey{
		UserID: userID, Name: "ci", TokenHash: "hash", Prefix: "pw_abcd",
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	path := "/account/api-keys/" + itoa(keyID) + "/delete"

	wantConfirmationPage(t, a.postToken(path))
	if keys := a.apiKeys(t, userID); len(keys) != 1 {
		t.Fatalf("%d keys after an unconfirmed revoke, want 1", len(keys))
	}
	wantStatus(t, a.postToken(path+"?confirmed=1"), http.StatusSeeOther)
	if keys := a.apiKeys(t, userID); len(keys) != 0 {
		t.Fatalf("%d keys after a confirmed revoke, want 0", len(keys))
	}
}

func (a *app) apiKeys(t *testing.T, userID int64) []models.ApiKey {
	t.Helper()
	keys, err := a.store.ListAPIKeysForUser(t.Context(), userID)
	if err != nil {
		t.Fatal(err)
	}
	return keys
}

func (a *app) sessionCount(t *testing.T, userID int64) int {
	t.Helper()
	sessions, err := a.store.ListSessionsForUser(t.Context(), userID, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return len(sessions)
}

func TestRevokingSessionsAsksFirst(t *testing.T) {
	a := noJSApp(t)
	userID := a.adminID(t)
	handle := apikey.Hash(a.sessionID(t))

	wantConfirmationPage(t, a.postToken("/account/sessions/revoke-others"))
	revoke := "/account/sessions/" + handle + "/revoke"
	wantConfirmationPage(t, a.postToken(revoke))
	if n := a.sessionCount(t, userID); n != 1 {
		t.Fatalf("%d sessions after unconfirmed revokes, want 1", n)
	}

	// Revokes the current session, signing this browser out.
	wantStatus(t, a.postToken(revoke+"?confirmed=1"), http.StatusSeeOther)
	if n := a.sessionCount(t, userID); n != 0 {
		t.Fatalf("%d sessions after the confirmed revoke, want 0", n)
	}
}

// The /admin twins share templates but not handlers, so each needs a gate.
func TestTheAdminTwinsAskFirstToo(t *testing.T) {
	a := noJSApp(t)
	projectID, checkID := a.checkFor(t, a.member(t, "owner"), "cu2")
	checkPath := "/admin/checks/" + itoa(checkID)
	projectPath := "/admin/projects/" + itoa(projectID)

	wantConfirmationPage(t, a.postToken(checkPath+"/regenerate"))
	if a.pingUUID(t, checkID) != "cu2" {
		t.Fatal("the ping URL changed before anyone confirmed")
	}
	wantConfirmationPage(t, a.postToken(checkPath+"/delete"))
	a.wantCheckExists(t, checkID, true, "deleted before anyone confirmed")
	wantConfirmationPage(t, a.postToken(projectPath+"/delete"))
	a.wantProjectExists(t, projectID, true, "deleted before anyone confirmed")

	wantStatus(t, a.postToken(checkPath+"/regenerate?confirmed=1"), http.StatusSeeOther)
	if a.pingUUID(t, checkID) == "cu2" {
		t.Error("the confirmed regenerate did not change the ping URL")
	}
	wantStatus(t, a.postToken(checkPath+"/delete?confirmed=1"), http.StatusSeeOther)
	a.wantCheckExists(t, checkID, false, "the confirmed delete did not go through")
	wantStatus(t, a.postToken(projectPath+"/delete?confirmed=1"), http.StatusSeeOther)
	a.wantProjectExists(t, projectID, false, "the confirmed delete did not go through")
}

func TestDeletingAChannelAsksFirstOnBothRoutes(t *testing.T) {
	a := noJSApp(t)
	projectID, _ := a.checkFor(t, a.adminID(t), "cu3")

	for _, prefix := range []string{"", "/admin"} {
		channelID, err := a.store.CreateChannel(t.Context(), store.NewChannel{
			ProjectID: projectID, Kind: models.ChannelWebhook, Name: "h", ConfigJSON: `{"url":"http://x.test/h"}`,
		}, time.Now())
		if err != nil {
			t.Fatal(err)
		}
		path := prefix + "/channels/" + itoa(channelID) + "/delete"

		wantConfirmationPage(t, a.postToken(path))
		a.wantChannelExists(t, channelID, true, prefix+": deleted before anyone confirmed")
		wantStatus(t, a.postToken(path+"?confirmed=1"), http.StatusSeeOther)
		a.wantChannelExists(t, channelID, false, prefix+": the confirmed delete did not go through")
	}
}

// Authorization precedes the gate: another user's resource is a 404.
func TestTheConfirmationNeverAnswersForAnotherUsersResource(t *testing.T) {
	a := noJSApp(t)
	projectID, checkID := a.checkFor(t, a.member(t, "other"), "cu2")
	wantStatus(t, a.postToken("/checks/"+itoa(checkID)+"/delete"), http.StatusNotFound)
	wantStatus(t, a.postToken("/projects/"+itoa(projectID)+"/delete"), http.StatusNotFound)
}
