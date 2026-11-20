package web

import (
	"cmp"
	"github.com/henry40408/pingward/internal/markdown"
	"net/http"
	"slices"
	"strings"
	"time"

	"github.com/henry40408/pingward/internal/duration"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/view"
	"github.com/henry40408/pingward/internal/views"
)

// heartbeatBars is more than fit: app.css clips from the left, newest pinned
// right.
const heartbeatBars = 120

// heartbeatWindow is two rows per bar plus headroom, so run durations can pair
// each finish with its start.
const heartbeatWindow = 300

const dashboardBars = 6
const dashboardPingWindow = 40
const dashboardSummaryChars = 120

// statusFilter mirrors the summary tiles: Up folds in Running, and Paused/New
// have no entry.
type statusFilter string

func parseStatusFilter(raw string) statusFilter {
	switch raw {
	case "up", "late", "down":
		return statusFilter(raw)
	}
	return ""
}

func (f statusFilter) matches(status view.DisplayStatus) bool {
	switch f {
	case "up":
		return status == view.StatusUp || status == view.StatusRunning
	case "late":
		return status == view.StatusLate
	case "down":
		return status == view.StatusDown
	}
	return true
}

// containsFold: the needle must already be lowercased. In Go, not SQL: LIKE
// case-sensitivity differs between SQLite and Postgres.
func containsFold(haystack, needle string) bool {
	return strings.Contains(strings.ToLower(haystack), needle)
}

func lastActivityAt(check models.Check) *time.Time {
	switch {
	case check.LastStartAt == nil:
		return check.LastPingAt
	case check.LastPingAt == nil || check.LastStartAt.After(*check.LastPingAt):
		return check.LastStartAt
	}
	return check.LastPingAt
}

// compareActivity orders newest first; a never-pinged check sorts last.
func compareActivity(a, b models.Check) int {
	left, right := lastActivityAt(a), lastActivityAt(b)
	switch {
	case left == nil && right == nil:
		return cmp.Compare(a.ID, b.ID)
	case left == nil:
		return 1
	case right == nil:
		return -1
	case !left.Equal(*right):
		return right.Compare(*left)
	}
	return cmp.Compare(a.ID, b.ID)
}

func sortProjectsByName(projects []models.Project) {
	slices.SortFunc(projects, func(a, b models.Project) int {
		return cmp.Or(
			cmp.Compare(strings.ToLower(a.Name), strings.ToLower(b.Name)),
			cmp.Compare(a.ID, b.ID))
	})
}

// scheduleLabel is formatted with duration.Format, so it matches what the form accepts.
func scheduleLabel(check models.Check) string {
	grace := duration.Format(check.GraceSecs)
	switch {
	case check.ScheduleKind == models.SchedulePeriod && check.PeriodSecs != nil:
		return "every " + duration.Format(*check.PeriodSecs) + " · " + grace + " grace"
	case check.ScheduleKind == models.ScheduleCron && check.CronExpr != nil:
		return *check.CronExpr + " · " + grace + " grace"
	}
	return grace + " grace"
}

type projectChecks struct {
	project models.Project
	checks  []models.Check
}

// filterProjectChecks drops projects with no matching check; a project-level
// hit keeps all its checks.
func filterProjectChecks(projects []models.Project, byProject map[int64][]models.Check, needle string) []projectChecks {
	var kept []projectChecks
	for _, project := range projects {
		checks := byProject[project.ID]
		slices.SortFunc(checks, compareActivity)
		projectHit := needle == "" || containsFold(project.Name, needle) || containsFold(project.Description, needle)
		if !projectHit {
			checks = slices.DeleteFunc(slices.Clone(checks), func(c models.Check) bool {
				return !containsFold(c.Name, needle) && !containsFold(c.Description, needle)
			})
			if len(checks) == 0 {
				continue
			}
		}
		kept = append(kept, projectChecks{project, checks})
	}
	return kept
}

type statusTally struct{ total, up, late, down int }

func (t *statusTally) count(status view.DisplayStatus) {
	t.total++
	switch status {
	case view.StatusUp, view.StatusRunning:
		t.up++
	case view.StatusLate:
		t.late++
	case view.StatusDown:
		t.down++
	}
}

func (d *Deps) dashboard(w http.ResponseWriter, r *http.Request) {
	userCount, err := d.Store.CountUsers(r.Context())
	if err != nil {
		internalError(w)
		return
	}
	if userCount == 0 {
		redirect(w, r, "/setup")
		return
	}
	user, ok := d.requireUser(w, r)
	if !ok {
		return
	}
	// Consumed here so the removal Set-Cookie rides back on this response.
	logoutNote, _ := d.takeFlash(w, r, "forward_auth_logout")
	query := trimRustSpace(r.URL.Query().Get("q"))
	filter := parseStatusFilter(r.URL.Query().Get("status"))
	page, err := d.dashboardView(r, user, query, filter)
	if err != nil {
		internalError(w)
		return
	}
	page.ForwardAuthLogout = logoutNote
	writeHTML(w, http.StatusOK, views.Dashboard(page))
}

func (d *Deps) dashboardView(r *http.Request, user *models.User, query string, filter statusFilter) (views.DashboardView, error) {
	ctx := r.Context()
	now := d.Now()
	projects, err := d.Store.ListProjectsForUser(ctx, user.ID)
	if err != nil {
		return views.DashboardView{}, err
	}
	sortProjectsByName(projects)
	projectIDs := make([]int64, len(projects))
	for i, project := range projects {
		projectIDs[i] = project.ID
	}
	byProject, err := d.Store.ListChecksForProjects(ctx, projectIDs)
	if err != nil {
		return views.DashboardView{}, err
	}
	// Filtered before the batched ping fetch, so a narrow filter narrows the query.
	matched := filterProjectChecks(projects, byProject, strings.ToLower(query))
	var checkIDs []int64
	for _, entry := range matched {
		for _, check := range entry.checks {
			checkIDs = append(checkIDs, check.ID)
		}
	}
	pings, err := d.Store.ListRecentPingSummariesForChecks(ctx, checkIDs, dashboardPingWindow)
	if err != nil {
		return views.DashboardView{}, err
	}
	withChannels, err := d.Store.ChecksWithChannels(ctx, checkIDs)
	if err != nil {
		return views.DashboardView{}, err
	}
	var tally statusTally
	var groups []views.ProjectGroup
	for _, entry := range matched {
		var rows []views.CheckRow
		for _, check := range entry.checks {
			status := view.Status(check, now)
			// Tiles count the q-filtered set regardless of the status filter.
			tally.count(status)
			if !filter.matches(status) {
				continue
			}
			rows = append(rows, dashboardCheckRow(check, status, pings[check.ID], !withChannels[check.ID], now))
		}
		if filter != "" && len(rows) == 0 {
			continue
		}
		groups = append(groups, views.ProjectGroup{
			ID: entry.project.ID, Name: entry.project.Name, Count: len(rows), Checks: rows,
			Description: markdown.TruncatePlain(entry.project.Description, dashboardSummaryChars),
		})
	}
	return views.DashboardView{
		CSRF: d.csrfToken(r), IsAdmin: user.IsAdmin,
		Total: tally.total, Up: tally.up, Late: tally.late, Down: tally.down,
		Groups: groups, Q: query, Status: string(filter),
	}, nil
}

func dashboardCheckRow(check models.Check, status view.DisplayStatus, pings []models.PingSummary, noChannel bool, now time.Time) views.CheckRow {
	last := "—"
	if check.LastPingAt != nil {
		last = view.FmtRelative(*check.LastPingAt, now)
	}
	return views.CheckRow{
		ID: check.ID, Name: check.Name, Status: string(status), Schedule: scheduleLabel(check), Last: last,
		Bars: view.Heartbeat(pings, view.HeartbeatOptions{
			MaxRuntimeSecs: check.MaxRuntimeSecs, Paused: check.Status == models.StatusPaused, Count: dashboardBars,
		}),
		Description: markdown.TruncatePlain(check.Description, dashboardSummaryChars),
		NoChannel:   noChannel,
	}
}
