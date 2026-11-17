package server_test

import (
	"fmt"
	"net/http"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
)

func dashboardApp(t *testing.T) *app {
	t.Helper()
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	return a
}

func (a *app) addProject(t *testing.T, name, description string) int64 {
	t.Helper()
	id, err := a.store.CreateProject(t.Context(), a.userByName(t, "admin").ID,
		store.ProjectSettings{Name: name, Description: description}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return id
}

func (a *app) addDescribedCheck(t *testing.T, projectID int64, name, description, uuid string) int64 {
	t.Helper()
	period := int64(3600)
	id, err := a.store.CreateCheck(t.Context(), store.NewCheck{
		ProjectID: projectID, Name: name, Description: description, PingUUID: uuid,
		Kind: models.SchedulePeriod, PeriodSecs: &period, GraceSecs: 300, Timezone: "UTC",
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return id
}

func (a *app) dashboardBody(t *testing.T, query string) string {
	t.Helper()
	res := a.client.Get("/" + query)
	wantStatus(t, res, http.StatusOK)
	return res.Body
}

// dashboardWithCheck is one project holding one never-pinged check.
func dashboardWithCheck(t *testing.T) (a *app, projectID, checkID int64) {
	t.Helper()
	a = dashboardApp(t)
	projectID = a.addProject(t, "web", "")
	return a, projectID, a.addCheck(t, projectID, "backup", "cu")
}

func wantContains(t *testing.T, body, want, why string) {
	t.Helper()
	if !strings.Contains(body, want) {
		t.Errorf("%s: %q missing in %s", why, want, body)
	}
}

func wantAbsent(t *testing.T, body, unwanted, why string) {
	t.Helper()
	if strings.Contains(body, unwanted) {
		t.Errorf("%s: %q present in %s", why, unwanted, body)
	}
}

// wantTile matches the tile's own markup, not a bare "1" somewhere on the page.
func wantTile(t *testing.T, body, label string, n int) {
	t.Helper()
	wantContains(t, body, fmt.Sprintf(`<div class="n">%d</div><div class="l">%s</div>`, n, label), "tile "+label)
}

func TestDashboardRendersTilesAndBadges(t *testing.T) {
	a, _, _ := dashboardWithCheck(t)
	body := a.dashboardBody(t, "")
	wantContains(t, body, `class="tiles"`, "summary tiles")
	wantContains(t, body, `class="badge`, "status badge")
}

func TestDashboardShowsProjectGroupAndCheckRow(t *testing.T) {
	a, projectID, checkID := dashboardWithCheck(t)
	body := a.dashboardBody(t, "")
	wantContains(t, body, "web", "project group name")
	wantContains(t, body, "1 checks", "group check count")
	wantContains(t, body, "/projects/"+itoa(projectID), "manage link")
	wantContains(t, body, "/checks/"+itoa(checkID), "check row link")
	wantContains(t, body, `class="badge new"`, "new badge")
	wantContains(t, body, `class="status-dot new"`, "new status dot")
}

// Both directions, so a template that always (or never) emits the chip fails.
func TestDashboardNoChannelChipReflectsBindingState(t *testing.T) {
	a, projectID, unbound := dashboardWithCheck(t)
	bound := a.addCheck(t, projectID, "has-channel", "cu-bound")
	channelID, err := a.store.CreateChannel(t.Context(), store.NewChannel{
		ProjectID: projectID, Kind: models.ChannelWebhook, Name: "hook", ConfigJSON: `{"url":"http://x"}`,
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if err := a.store.BindChannel(t.Context(), bound, channelID); err != nil {
		t.Fatal(err)
	}
	body := a.dashboardBody(t, "")
	const chip = `data-testid="check-no-channel"`
	if n := strings.Count(body, chip); n != 1 {
		t.Fatalf("%d rows render the chip, want exactly the unbound one", n)
	}
	rows := strings.Split(body, `class="check"`)
	unboundRow := rowLinking(t, rows, unbound)
	boundRow := rowLinking(t, rows, bound)
	if !strings.Contains(unboundRow, chip) {
		t.Error("the unbound check's row must carry the chip")
	}
	if strings.Contains(boundRow, chip) {
		t.Error("the bound check's row must not carry the chip")
	}
}

func rowLinking(t *testing.T, rows []string, checkID int64) string {
	t.Helper()
	for _, row := range rows {
		if strings.Contains(row, "/checks/"+itoa(checkID)) {
			return row
		}
	}
	t.Fatalf("no row links to check %d", checkID)
	return ""
}

func TestDashboardCountsRunningCheckUnderUpAndKeepsTheRowBadge(t *testing.T) {
	a, projectID, checkID := dashboardWithCheck(t)
	started := time.Now()
	a.markPing(t, checkID, store.PingMark{Status: models.StatusNew, LastStartAt: &started})
	// A second, "new" check proves Up counts the running check, not every check.
	a.addCheck(t, projectID, "idle", "idle-uuid")
	body := a.dashboardBody(t, "")
	wantTile(t, body, "Up", 1)
	wantAbsent(t, body, ">Running</div>", "the Running tile must be gone")
	if n := strings.Count(body, `class="badge running"`); n != 1 {
		t.Errorf("%d rows render the running badge, want 1", n)
	}
}

// Its distinctive term ("glacier") sits past the 120-char summary cut-off.
const longDescription = "Copies the primary database to cold storage every night and verifies " +
	"checksums end to end so silent corruption is caught before it spreads, then uploads the " +
	"result to the offsite glacier vault."

// dashboardWithTwoProjects has two projects and four checks; each filter
// dimension (project/check name and description) has a term unique to it.
func dashboardWithTwoProjects(t *testing.T) *app {
	t.Helper()
	a := dashboardApp(t)
	web := a.addProject(t, "web", "Public **frontend** services")
	infra := a.addProject(t, "infra", "Datacenter plumbing")
	a.addDescribedCheck(t, web, "backup", longDescription, "cu-backup")
	a.addDescribedCheck(t, web, "deploy", "", "cu-deploy")
	a.addDescribedCheck(t, infra, "rotate-certs", "Renews TLS certificates", "cu-rotate")
	a.addDescribedCheck(t, infra, "vacuum", "", "cu-vacuum")
	return a
}

func TestDashboardUnfilteredShowsEveryProjectAndNoClearLink(t *testing.T) {
	body := dashboardWithTwoProjects(t).dashboardBody(t, "")
	wantContains(t, body, ">web</h2>", "web group")
	wantContains(t, body, ">infra</h2>", "infra group")
	wantTile(t, body, "Total", 4)
	wantAbsent(t, body, "dashboard-filter-clear", "clear affordance only while a filter is active")
}

func TestDashboardBlankQueryIsTreatedAsUnfiltered(t *testing.T) {
	body := dashboardWithTwoProjects(t).dashboardBody(t, "?q=%20%20")
	wantContains(t, body, ">web</h2>", "web group")
	wantContains(t, body, ">infra</h2>", "infra group")
	wantTile(t, body, "Total", 4)
	wantAbsent(t, body, "dashboard-filter-clear", "whitespace-only q is not an active filter")
}

func TestDashboardFilterByCheckNameDropsOtherProjectsAndNarrowsCounters(t *testing.T) {
	body := dashboardWithTwoProjects(t).dashboardBody(t, "?q=rotate")
	wantContains(t, body, ">infra</h2>", "infra group")
	wantShowsCheck(t, body, "rotate-certs")
	wantAbsent(t, body, ">web</h2>", "a project with no matching check must not render")
	wantHidesCheck(t, body, "vacuum")
	wantTile(t, body, "Total", 1)
	wantTile(t, body, "Up", 0)
	wantContains(t, body, "dashboard-filter-clear", "an active filter must offer a way out")
	wantContains(t, body, `value="rotate"`, "the search box echoes the term")
}

// A project-level hit lists all its checks, not a header above an empty list.
func TestDashboardFilterByProjectDescriptionKeepsAllOfItsChecks(t *testing.T) {
	body := dashboardWithTwoProjects(t).dashboardBody(t, "?q=plumbing")
	wantContains(t, body, ">infra</h2>", "infra group")
	wantShowsCheck(t, body, "rotate-certs")
	wantShowsCheck(t, body, "vacuum")
	wantAbsent(t, body, ">web</h2>", "the non-matching project")
	wantTile(t, body, "Total", 2)
}

func TestDashboardFilterByProjectNameKeepsAllOfItsChecks(t *testing.T) {
	body := dashboardWithTwoProjects(t).dashboardBody(t, "?q=infra")
	wantShowsCheck(t, body, "rotate-certs")
	wantShowsCheck(t, body, "vacuum")
	wantAbsent(t, body, ">web</h2>", "web must not render")
	wantTile(t, body, "Total", 2)
}

// Fails if the filter is pointed at the truncated summary.
func TestDashboardFilterMatchesDescriptionTextBeyondTheVisibleSummary(t *testing.T) {
	if strings.Index(longDescription, "glacier") <= 120 {
		t.Fatal("fixture must place the search term past the summary cut-off")
	}
	body := dashboardWithTwoProjects(t).dashboardBody(t, "?q=glacier")
	wantShowsCheck(t, body, "backup")
	wantAbsent(t, body, "offsite glacier vault", "the matched tail is past the summary cut-off")
	if n := strings.Count(body, "glacier"); n != 1 {
		t.Errorf("the term appears %d times, want only the echoed search box", n)
	}
	wantHidesCheck(t, body, "deploy")
	wantTile(t, body, "Total", 1)
}

func TestDashboardFilterIsCaseInsensitive(t *testing.T) {
	a := dashboardWithTwoProjects(t)
	body := a.dashboardBody(t, "?q=ROTATE-Certs")
	wantShowsCheck(t, body, "rotate-certs")
	wantTile(t, body, "Total", 1)

	// "TLS" is stored uppercase, so this pins the haystack folding too.
	body = a.dashboardBody(t, "?q=tls")
	wantShowsCheck(t, body, "rotate-certs")
	wantTile(t, body, "Total", 1)
}

func TestDashboardNoResultsStateIsDistinctFromTheEmptyState(t *testing.T) {
	body := dashboardWithTwoProjects(t).dashboardBody(t, "?q=nonesuch")
	wantContains(t, body, "dashboard-no-results", "no-results state")
	wantAbsent(t, body, "dashboard-empty", "a user who owns projects must not be told they have none")
	wantContains(t, body, "Clear the filter", "a way back")
	wantTile(t, body, "Total", 0)
}

// dashboardWithMixedStatuses: web is Up, job Running, cron Late, db Down.
func dashboardWithMixedStatuses(t *testing.T) *app {
	t.Helper()
	a := dashboardApp(t)
	projectID := a.addProject(t, "services", "")
	now := time.Now()
	in := func(seconds int64) *time.Time { at := now.Add(time.Duration(seconds) * time.Second); return &at }

	a.markPing(t, a.addCheck(t, projectID, "web", "cu-web"),
		store.PingMark{Status: models.StatusUp, LastPingAt: &now, NextDueAt: in(3600)})
	a.markPing(t, a.addCheck(t, projectID, "job", "cu-job"),
		store.PingMark{Status: models.StatusNew, LastStartAt: &now})
	// Late: due in 100s with 300s grace, so now is inside (expected, due].
	a.markPing(t, a.addCheck(t, projectID, "cron", "cu-cron"),
		store.PingMark{Status: models.StatusUp, LastPingAt: in(-3500), NextDueAt: in(100)})
	a.markPing(t, a.addCheck(t, projectID, "db", "cu-db"),
		store.PingMark{Status: models.StatusDown, LastPingAt: &now})
	return a
}

func TestDashboardMixedStatusesPopulateTheMergedTiles(t *testing.T) {
	body := dashboardWithMixedStatuses(t).dashboardBody(t, "")
	wantTile(t, body, "Total", 4)
	wantTile(t, body, "Up", 2)
	wantTile(t, body, "Late", 1)
	wantTile(t, body, "Down", 1)
	wantAbsent(t, body, ">Running</div>", "there must be no Running tile")
	for _, name := range []string{"web", "job", "cron", "db"} {
		wantShowsCheck(t, body, name)
	}
}

func TestDashboardStatusFilterNarrowsTheListButNotTheTiles(t *testing.T) {
	body := dashboardWithMixedStatuses(t).dashboardBody(t, "?status=down")
	wantShowsCheck(t, body, "db")
	for _, name := range []string{"web", "job", "cron"} {
		wantHidesCheck(t, body, name)
	}
	// Tiles follow q, not the status select, so other buckets stay switchable.
	wantTile(t, body, "Total", 4)
	wantTile(t, body, "Up", 2)
	wantTile(t, body, "Late", 1)
	wantTile(t, body, "Down", 1)
	wantContains(t, body, `value="down" selected`, "the status select re-selects the active option")
	wantContains(t, body, "dashboard-filter-clear", "an active status filter offers a way out")
}

func TestDashboardStatusUpFilterIncludesRunningChecks(t *testing.T) {
	body := dashboardWithMixedStatuses(t).dashboardBody(t, "?status=up")
	wantShowsCheck(t, body, "web")
	wantShowsCheck(t, body, "job") // up and running share a bucket
	wantHidesCheck(t, body, "cron")
	wantHidesCheck(t, body, "db")
}

func TestDashboardStatusAndTextFiltersCombineWithAnd(t *testing.T) {
	body := dashboardWithMixedStatuses(t).dashboardBody(t, "?q=web&status=up")
	wantShowsCheck(t, body, "web")
	wantHidesCheck(t, body, "job") // up, but fails the text filter
	wantHidesCheck(t, body, "db")
}

// Degrades to "no filter" rather than a 400 or an empty page.
func TestDashboardUnknownStatusValueIsIgnored(t *testing.T) {
	body := dashboardWithMixedStatuses(t).dashboardBody(t, "?status=bogus")
	for _, name := range []string{"web", "job", "cron", "db"} {
		wantShowsCheck(t, body, name)
	}
	wantAbsent(t, body, "dashboard-filter-clear", "a bogus status is not an active filter")
	wantContains(t, body, `value="" selected`, "the All option for a bogus status")
	for _, status := range []string{"up", "late", "down"} {
		wantAbsent(t, body, fmt.Sprintf(`value="%s" selected`, status), "no real status option is selected")
	}
}

// Even with q empty, a status filter matching nothing is "no results".
func TestDashboardStatusFilterWithNoMatchesShowsNoResultsNotEmpty(t *testing.T) {
	a := dashboardApp(t)
	a.addCheck(t, a.addProject(t, "svc", ""), "fresh", "cu-fresh")
	body := a.dashboardBody(t, "?status=down")
	wantContains(t, body, "dashboard-no-results", "no-results state")
	wantAbsent(t, body, "dashboard-empty", "a user who owns projects must not be told they have none")
	wantContains(t, body, "dashboard-filter-clear", "a status-only filter still offers clear")
}

func TestDashboardEmptyStateWhenNoProjects(t *testing.T) {
	wantContains(t, dashboardApp(t).dashboardBody(t, ""), "No projects yet", "empty-state message")
}

// renderedOrder lists names in render order; tests assert the whole sequence.
func renderedOrder(body, open, closing string) []string {
	var names []string
	for _, part := range strings.Split(body, open)[1:] {
		if name, _, found := strings.Cut(part, closing); found {
			names = append(names, name)
		}
	}
	return names
}

// projectOrder: <h2> appears nowhere else in the dashboard or its layout.
func projectOrder(body string) []string { return renderedOrder(body, "<h2>", "</h2>") }

// checkOrder reads the nm cell's link, so the search box and project headers
// cannot contribute.
func checkOrder(body string) []string {
	var names []string
	for _, cell := range renderedOrder(body, `class="nm"><a href="`, "</a>") {
		if _, name, found := strings.Cut(cell, ">"); found {
			names = append(names, name)
		}
	}
	return names
}

func wantShowsCheck(t *testing.T, body, name string) {
	t.Helper()
	if !slices.Contains(checkOrder(body), name) {
		t.Errorf("check %q missing: %s", name, body)
	}
}

func wantHidesCheck(t *testing.T, body, name string) {
	t.Helper()
	if slices.Contains(checkOrder(body), name) {
		t.Errorf("check %q must not render: %s", name, body)
	}
}

func wantOrder(t *testing.T, got, want []string, why string) {
	t.Helper()
	if !slices.Equal(got, want) {
		t.Errorf("%s: got %v, want %v", why, got, want)
	}
}

func TestDashboardOrdersProjectsByNameNotCreation(t *testing.T) {
	// web is created before infra, so id order would fail.
	body := dashboardWithTwoProjects(t).dashboardBody(t, "")
	wantOrder(t, projectOrder(body), []string{"infra", "web"}, "projects ordered by name")
}

func TestDashboardProjectOrderByNameIsCaseInsensitive(t *testing.T) {
	a := dashboardApp(t)
	// Byte order would put every uppercase name first.
	for _, name := range []string{"Zulu", "alpha", "Mike"} {
		a.addProject(t, name, "")
	}
	wantOrder(t, projectOrder(a.dashboardBody(t, "")), []string{"alpha", "Mike", "Zulu"}, "case-insensitive project order")
}

// dashboardWithStaggeredActivity has checks whose creation order disagrees
// with their activity order, plus one never pinged.
func dashboardWithStaggeredActivity(t *testing.T) *app {
	t.Helper()
	a := dashboardApp(t)
	projectID := a.addProject(t, "services", "")
	now := time.Now()
	ago := func(seconds int64) *time.Time { at := now.Add(-time.Duration(seconds) * time.Second); return &at }

	a.markPing(t, a.addCheck(t, projectID, "stale", "cu-stale"), store.PingMark{Status: models.StatusUp, LastPingAt: ago(900)})
	a.addCheck(t, projectID, "untouched", "cu-untouched")
	a.markPing(t, a.addCheck(t, projectID, "middle", "cu-middle"), store.PingMark{Status: models.StatusUp, LastPingAt: ago(600)})
	// In flight: only a start, which alone dates it.
	a.markPing(t, a.addCheck(t, projectID, "running", "cu-running"), store.PingMark{Status: models.StatusNew, LastStartAt: ago(60)})
	// Old start, recent finish: guards against sorting on the start alone.
	a.markPing(t, a.addCheck(t, projectID, "finished", "cu-finished"),
		store.PingMark{Status: models.StatusUp, LastPingAt: ago(120), LastStartAt: ago(1200)})
	return a
}

func TestDashboardOrdersChecksByMostRecentActivity(t *testing.T) {
	body := dashboardWithStaggeredActivity(t).dashboardBody(t, "")
	wantOrder(t, checkOrder(body), []string{"running", "finished", "middle", "stale", "untouched"}, "newest activity first")
}

func TestDashboardNeverPingedChecksKeepCreationOrderAtTheBottom(t *testing.T) {
	a := dashboardApp(t)
	projectID := a.addProject(t, "services", "")
	// All sort keys tie, so order must fall back to creation (id).
	for _, name := range []string{"zeta", "alpha", "mu"} {
		a.addCheck(t, projectID, name, "cu-"+name)
	}
	wantOrder(t, checkOrder(a.dashboardBody(t, "")), []string{"zeta", "alpha", "mu"}, "creation order")
}

func TestDashboardCheckOrderSurvivesTheStatusFilter(t *testing.T) {
	body := dashboardWithStaggeredActivity(t).dashboardBody(t, "?status=up")
	wantOrder(t, checkOrder(body), []string{"running", "finished", "middle", "stale"}, "status filter keeps activity order")
}
