package server_test

import (
	"net/http"
	"net/url"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/duration"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

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
	wantContains(t, res.Body, "Description must be at most 2000 characters.", "2001 characters rejected with the exact message")
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

func checkFormValues(name, description string) url.Values {
	return url.Values{
		"name": {name}, "description": {description}, "schedule_kind": {"period"}, "period_secs": {"3600"},
		"cron_expr": {""}, "grace_secs": {"300"}, "timezone": {"UTC"},
		"scan_interval_secs": {""}, "max_runtime_secs": {""}, "nag_interval_secs": {""},
	}
}

func (a *app) onlyCheckOf(t *testing.T, projectID int64) models.Check {
	t.Helper()
	checks, err := a.store.ListChecksForProject(t.Context(), projectID)
	if err != nil || len(checks) != 1 {
		t.Fatalf("checks = %v, %v", checks, err)
	}
	return checks[0]
}

func TestCheckFormIsRestyledAndKeepsFields(t *testing.T) {
	a, projectID := projectWith(t)
	res := a.client.Get("/projects/" + itoa(projectID) + "/checks/new")
	wantStatus(t, res, http.StatusOK)
	wantContains(t, res.Body, `class="field"`, "form restyled")
	for _, name := range []string{
		"name", "description", "schedule_kind", "period_secs", "cron_expr", "grace_secs", "timezone",
		"scan_interval_secs", "max_runtime_secs", "nag_interval_secs",
	} {
		wantContains(t, res.Body, `name="`+name+`"`, "check form field "+name)
	}
}

func TestCheckDescriptionRoundTripsAndIsLengthValidated(t *testing.T) {
	a, projectID := projectWith(t)
	wantStatus(t, a.postWithToken("/projects/"+itoa(projectID)+"/checks", checkFormValues("backup", "runs *nightly*")), http.StatusSeeOther)
	check := a.onlyCheckOf(t, projectID)
	if check.Description != "runs *nightly*" {
		t.Fatalf("stored description = %q", check.Description)
	}
	path := "/checks/" + itoa(check.ID)
	edit := a.client.Get(path + "/edit")
	wantStatus(t, edit, http.StatusOK)
	wantContains(t, edit.Body, "runs *nightly*", "edit form round-trips the stored description")

	res := a.postWithToken(path, checkFormValues("backup", strings.Repeat("a", 2001)))
	wantStatus(t, res, http.StatusOK)
	wantContains(t, res.Body, "Description must be at most 2000 characters.", "2001 characters rejected with the exact message")

	boundary := strings.Repeat("b", 2000)
	wantStatus(t, a.postWithToken(path, checkFormValues("backup", boundary)), http.StatusSeeOther)
	if got := a.onlyCheckOf(t, projectID).Description; got != boundary {
		t.Fatal("a 2000-character description is exactly at the limit and must be accepted")
	}
}

func TestCheckCreatedViaWebFormIsBoundToExistingChannels(t *testing.T) {
	a, projectID := projectWith(t)
	var want []int64
	for _, name := range []string{"hook1", "hook2"} {
		id, err := a.store.CreateChannel(t.Context(), store.NewChannel{
			ProjectID: projectID, Kind: models.ChannelWebhook, Name: name, ConfigJSON: `{"url":"http://x"}`,
		}, time.Now())
		if err != nil {
			t.Fatal(err)
		}
		want = append(want, id)
	}
	wantStatus(t, a.postWithToken("/projects/"+itoa(projectID)+"/checks", checkFormValues("backup", "")), http.StatusSeeOther)
	bound, err := a.store.BoundChannelIDs(t.Context(), a.onlyCheckOf(t, projectID).ID)
	if err != nil {
		t.Fatal(err)
	}
	slices.Sort(bound)
	if !slices.Equal(bound, want) {
		t.Fatalf("bound = %v, want %v", bound, want)
	}
}

func TestCheckFormDurationFieldsOfferTheSharedSuggestions(t *testing.T) {
	a, projectID := projectWith(t)
	body := a.client.Get("/projects/" + itoa(projectID) + "/checks/new").Body
	wantList(t, body, "dur-list", suggestedDurations)
	wantWired(t, body, "dur-list", "period_secs", "grace_secs", "scan_interval_secs", "max_runtime_secs", "nag_interval_secs")
}

// Each suggestion must survive all five duration fields.
func TestEverySuggestedDurationIsAcceptedByTheCheckForm(t *testing.T) {
	a, projectID := projectWith(t)
	for _, suggestion := range suggestedDurations {
		form := checkFormValues(suggestion, "")
		for _, field := range []string{"period_secs", "grace_secs", "scan_interval_secs", "max_runtime_secs", "nag_interval_secs"} {
			form.Set(field, suggestion)
		}
		res := a.postWithToken("/projects/"+itoa(projectID)+"/checks", form)
		if res.StatusCode != http.StatusSeeOther {
			t.Fatalf("the check form rejected its own suggestion %q: %d", suggestion, res.StatusCode)
		}
		want, _ := duration.Parse(suggestion)
		stored := a.checkNamed(t, projectID, suggestion)
		for label, got := range map[string]*int64{
			"period": stored.PeriodSecs, "scan": stored.ScanIntervalSecs, "max runtime": stored.MaxRuntimeSecs, "nag": stored.NagIntervalSecs,
		} {
			if got == nil || *got != want {
				t.Errorf("%q: %s = %v, want %d", suggestion, label, got, want)
			}
		}
		if stored.GraceSecs != want {
			t.Errorf("%q: grace = %d, want %d", suggestion, stored.GraceSecs, want)
		}
	}
}

func (a *app) checkNamed(t *testing.T, projectID int64, name string) models.Check {
	t.Helper()
	checks, err := a.store.ListChecksForProject(t.Context(), projectID)
	if err != nil {
		t.Fatal(err)
	}
	for _, check := range checks {
		if check.Name == name {
			return check
		}
	}
	t.Fatalf("no check stored for %q", name)
	return models.Check{}
}
