package server_test

import (
	"net/http"
	"net/url"
	"strings"
	"testing"

	"github.com/henry40408/pingward/internal/testutil"
)

// The Rust check-form tests (restyled fields, description round trip, channel
// binding on create, duration suggestions and their acceptance) wait for the
// check forms, which are blocked on the cron/tz/markdown fixtures.

// suggestedDurations is src/view.rs durations(), written out so a broken list
// cannot agree with itself.
var suggestedDurations = []string{"30s", "1m", "5m", "15m", "30m", "1h", "6h", "12h", "1d", "7d"}

// suggestedExpiries is src/view.rs expiries().
var suggestedExpiries = []string{"7d", "30d", "90d", "365d"}

func projectFormValues(name, description string) url.Values {
	return url.Values{"name": {name}, "description": {description}, "scan_interval_secs": {""}, "nag_interval_secs": {""}}
}

func (a *app) postWithToken(path string, form url.Values) *testutil.Response {
	form.Set("_csrf", testutil.NewestSessionCSRF(a.client.T, a.store))
	return a.client.PostForm(path, form)
}

func TestChannelFormIsRestyledAndKeepsFields(t *testing.T) {
	a, projectID := projectWith(t)
	res := a.client.Get("/projects/" + itoa(projectID) + "/channels/new")
	wantStatus(t, res, http.StatusOK)
	wantContains(t, res.Body, `class="field"`, "form restyled")
	wantContains(t, res.Body, `name="webhook_url"`, "webhook field")
}

// Every field name the project form reads.
func TestProjectFormIsRestyledAndKeepsFields(t *testing.T) {
	a := dashboardApp(t)
	res := a.client.Get("/projects/new")
	wantStatus(t, res, http.StatusOK)
	wantContains(t, res.Body, `class="field"`, "form restyled")
	for _, name := range []string{"name", "description", "scan_interval_secs", "nag_interval_secs"} {
		wantContains(t, res.Body, `name="`+name+`"`, "project form field "+name)
	}
}

func (a *app) storedDescription(t *testing.T, projectID int64) string {
	t.Helper()
	project, err := a.store.FindProject(t.Context(), projectID)
	if err != nil || project == nil {
		t.Fatalf("project %d: %v %v", projectID, project, err)
	}
	return project.Description
}

func TestProjectDescriptionRoundTripsAndIsLengthValidated(t *testing.T) {
	a := dashboardApp(t)
	wantStatus(t, a.postWithToken("/projects", projectFormValues("proj", "**bold** desc")), http.StatusSeeOther)
	projects, err := a.store.ListProjectsForUser(t.Context(), a.adminID(t))
	if err != nil || len(projects) != 1 {
		t.Fatalf("projects = %v, %v", projects, err)
	}
	if projects[0].Description != "**bold** desc" {
		t.Fatalf("stored description = %q", projects[0].Description)
	}
	projectID := projects[0].ID
	path := "/projects/" + itoa(projectID)

	edit := a.client.Get(path + "/edit")
	wantStatus(t, edit, http.StatusOK)
	wantContains(t, edit.Body, "**bold** desc", "edit form round-trips the stored description into the textarea")

	// Both sides of the 2000-character limit.
	res := a.postWithToken(path, projectFormValues("proj", strings.Repeat("a", 2001)))
	wantStatus(t, res, http.StatusOK)
	wantContains(t, res.Body, "description must be at most 2000 characters", "2001 characters rejected with the exact message")
	if got := a.storedDescription(t, projectID); got != "**bold** desc" {
		t.Fatalf("the rejected update overwrote the stored description: %.20q", got)
	}

	boundary := strings.Repeat("b", 2000)
	wantStatus(t, a.postWithToken(path, projectFormValues("proj", boundary)), http.StatusSeeOther)
	if got := a.storedDescription(t, projectID); got != boundary {
		t.Fatalf("a 2000-character description is exactly at the limit and must be accepted")
	}
}

// current-password offers the saved credential, new-password a generated one;
// swapping them fails silently in the password manager.
func TestCredentialFieldsDeclareTheirAutocompleteRole(t *testing.T) {
	fresh := newApp(t, nil)
	setup := fresh.client.Get("/setup").Body
	wantContains(t, setup, `name="username" autocomplete="username"`, "/setup username")
	wantContains(t, setup, `type="password" autocomplete="new-password"`, "/setup sets a password, so it must not be tagged current-password")

	a := dashboardApp(t)
	// A logged-out client on the same store: the signed-in jar would bounce to /.
	anonymous := testutil.NewClient(t, newHandler(a.deps))
	login := anonymous.Get("/login").Body
	wantContains(t, login, `name="username" autocomplete="username"`, "/login username")
	wantContains(t, login, `type="password" autocomplete="current-password"`, "/login submits an existing credential")

	// /admin edits other accounts, so its username field opts out of autofill.
	admin := a.client.Get("/admin").Body
	wantContains(t, admin, `name="username" autocomplete="off"`, "/admin username")
	if n := strings.Count(admin, `autocomplete="new-password"`); n != 2 {
		t.Errorf("/admin has %d new-password fields, want the reset field and the add-user field", n)
	}

	account := a.client.Get("/account").Body
	wantContains(t, account, `autocomplete="current-password"`, "/account current password")
	if n := strings.Count(account, `autocomplete="new-password"`); n != 2 {
		t.Errorf("/account has %d new-password fields, want 2", n)
	}
}

// --- duration suggestion lists ---
//
// Duration fields are wired to a <datalist>, and every suggestion must be one
// the handler accepts: the user picks it from the browser's own dropdown.

func inputTag(t *testing.T, body, id string) string {
	t.Helper()
	for _, tag := range strings.Split(body, "<") {
		if strings.HasPrefix(tag, "input") && strings.Contains(tag, `id="`+id+`"`) {
			return tag
		}
	}
	t.Fatalf(`no <input id="%s"> on the page`, id)
	return ""
}

// wantList wants exactly one <datalist id>, with an <option> for each value.
func wantList(t *testing.T, body, id string, want []string) {
	t.Helper()
	if n := strings.Count(body, `<datalist id="`+id+`">`); n != 1 {
		t.Errorf(`%d <datalist id="%s"> elements, want exactly one: a duplicate id leaves the second as dead markup`, n, id)
	}
	for _, value := range want {
		wantContains(t, body, `<option value="`+value+`">`, "list "+id+" suggestion")
	}
}

func wantWired(t *testing.T, body, listID string, fields ...string) {
	t.Helper()
	for _, field := range fields {
		if tag := inputTag(t, body, field); !strings.Contains(tag, `list="`+listID+`"`) {
			t.Errorf("field %s is not wired to the %s suggestions: %s", field, listID, tag)
		}
	}
}

func TestProjectFormDurationFieldsOfferTheSharedSuggestions(t *testing.T) {
	body := dashboardApp(t).client.Get("/projects/new").Body
	wantList(t, body, "dur-list", suggestedDurations)
	wantWired(t, body, "dur-list", "scan_interval_secs", "nag_interval_secs")
}

// The retention fields count days, so 5m would be rejected.
func TestAdminSettingsDurationFieldsOfferTheSharedSuggestions(t *testing.T) {
	body := dashboardApp(t).client.Get("/admin").Body
	wantList(t, body, "dur-list", suggestedDurations)
	wantWired(t, body, "dur-list", "scan_interval", "nag_interval")
	for _, daysField := range []string{"pings_retention_days", "notifications_retention_days", "audit_retention_days"} {
		if tag := inputTag(t, body, daysField); strings.Contains(tag, `list="dur-list"`) {
			t.Errorf("%s counts days, not durations, and must not offer duration suggestions: %s", daysField, tag)
		}
	}
}

func TestAPIKeyExpiryOffersItsOwnSuggestions(t *testing.T) {
	body := dashboardApp(t).client.Get("/account").Body
	wantList(t, body, "expiry-list", suggestedExpiries)
	wantWired(t, body, "expiry-list", "expires_in")
	wantAbsent(t, body, `<datalist id="dur-list">`, "/account has no interval fields, so the shared list has no business there")
}
