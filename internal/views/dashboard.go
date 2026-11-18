package views

import "github.com/henry40408/pingward/internal/view"

// DashboardView feeds the checks overview.
type DashboardView struct {
	CSRF    string
	IsAdmin bool
	Total   int
	// Up includes running checks: an in-flight run is still up, so they share a tile.
	Up     int
	Late   int
	Down   int
	Groups []ProjectGroup
	Q      string
	Status string
	// ForwardAuthLogout is set for a forward-auth logout with no gateway URL:
	// it tells the visitor to sign out at the proxy.
	ForwardAuthLogout string
}

// CheckRow is one check on the dashboard.
type CheckRow struct {
	ID       int64
	Name     string
	Status   string
	Schedule string // "every 1h · 10m grace" or the cron expression
	Last     string
	Bars     []view.Bar
	// Description is a single-line plain-text summary.
	Description string
	// NoChannel renders a chip so a check nobody is alerted for shows.
	NoChannel bool
}

type ProjectGroup struct {
	ID          int64
	Name        string
	Count       int
	Checks      []CheckRow
	Description string // single-line plain-text summary
}

func Dashboard(v DashboardView) string {
	filtered := v.Q != "" || v.Status != ""
	var list node
	if len(v.Groups) == 0 {
		list = emptyList(filtered)
	} else {
		list = group(each(v.Groups, groupCard), legend())
	}
	body := group(
		when(v.ForwardAuthLogout != "", el("p", []attr{a("class", "flash warn"), a("data-testid", "forward-auth-logout-flash")}, text(v.ForwardAuthLogout))),
		el("div", []attr{a("class", "phead")},
			el("div", nil, el("h1", nil, text("Checks"))),
			el("a", []attr{a("class", "btn-primary"), a("href", "/projects/new")}, text("New project"))),
		filterForm(v, filtered),
		tiles(v),
		list,
	)
	return Page(navChrome(v.IsAdmin, v.CSRF), body)
}

func selectOption(current, value, label string) node {
	return el("option", []attr{a("value", value), flagIf(current == value, "selected")}, text(label))
}

func filterForm(v DashboardView, filtered bool) node {
	return el("form", []attr{a("class", "filters"), a("method", "get"), a("action", "/"), a("role", "search")},
		el("label", []attr{a("class", "fld")}, text("Filter"),
			void("input", a("type", "search"), a("name", "q"), a("value", v.Q), a("placeholder", "project or check name / description"), a("data-testid", "dashboard-filter-input"))),
		el("label", []attr{a("class", "fld")}, text("Status"),
			el("select", []attr{a("name", "status"), a("data-testid", "dashboard-status-filter")},
				selectOption(v.Status, "", "All"),
				selectOption(v.Status, "up", "Up"),
				selectOption(v.Status, "late", "Late"),
				selectOption(v.Status, "down", "Down"))),
		el("button", []attr{a("class", "btn"), a("type", "submit"), a("data-testid", "dashboard-filter-submit")}, text("Filter")),
		when(filtered, el("a", []attr{a("class", "btn"), a("href", "/"), a("data-testid", "dashboard-filter-clear")}, text("Clear"))),
	)
}

func tile(class string, n int, label string) node {
	return el("div", []attr{a("class", "tile "+class)},
		el("span", []attr{a("class", "edge")}),
		el("div", []attr{a("class", "n")}, text(num(n))),
		el("div", []attr{a("class", "l")}, text(label)))
}

func tiles(v DashboardView) node {
	return el("div", []attr{a("class", "tiles")},
		tile("total", v.Total, "Total"), tile("up", v.Up, "Up"),
		tile("late", v.Late, "Late"), tile("down", v.Down, "Down"))
}

func emptyList(filtered bool) node {
	if filtered {
		return el("div", []attr{a("class", "list")},
			el("p", []attr{a("class", "empty"), a("data-testid", "dashboard-no-results")},
				text("No checks match the current filter. "),
				el("a", []attr{a("href", "/")}, text("Clear the filter")),
				text(" to see every check.")))
	}
	return el("div", []attr{a("class", "list")},
		el("p", []attr{a("class", "empty"), a("data-testid", "dashboard-empty")}, text("No projects yet. Create one to start watching a job.")))
}

func groupCard(g ProjectGroup) node {
	return el("div", []attr{a("class", "group")},
		el("div", []attr{a("class", "gh")},
			el("h2", nil, text(g.Name)),
			when(g.Description != "", el("span", []attr{a("class", "gdesc")}, text(g.Description))),
			el("span", []attr{a("class", "count")}, text(num(g.Count)+" checks")),
			el("span", []attr{a("class", "line")}),
			el("a", []attr{a("href", "/projects/"+num(g.ID))}, text("Manage →"))),
		el("div", []attr{a("class", "list")}, each(g.Checks, checkRow)))
}

func heartbeatBars(bars []view.Bar) node {
	return each(bars, func(bar view.Bar) node {
		return el("i", []attr{a("class", bar.Class), a("style", "height:"+num(bar.Height)+"px"), a("title", bar.Title)})
	})
}

func checkRow(c CheckRow) node {
	return el("div", []attr{a("class", "check"), a("data-testid", "dashboard-check-row"), a("data-href", "/checks/"+num(c.ID))},
		el("span", []attr{a("class", "status-dot "+c.Status)}),
		el("div", []attr{a("class", "cmeta")},
			el("div", []attr{a("class", "nm")}, el("a", []attr{a("href", "/checks/"+num(c.ID))}, text(c.Name))),
			el("div", []attr{a("class", "sc")}, text(c.Schedule)),
			when(c.Description != "", el("div", []attr{a("class", "cdesc"), a("data-testid", "check-description-summary")}, text(c.Description)))),
		when(c.NoChannel, noChannelChip()),
		el("div", []attr{a("class", "spark")}, heartbeatBars(c.Bars)),
		el("div", []attr{a("class", "cwhen")}, text(c.Last)),
		el("span", []attr{a("class", "badge "+c.Status)}, text(c.Status)))
}

func legendKey(swatch, label string) node {
	return el("span", []attr{a("class", "k")},
		el("span", []attr{a("class", "mini")}, el("i", []attr{a("class", swatch)})), text(label))
}

func legend() node {
	return el("div", []attr{a("class", "legend")},
		el("span", []attr{a("class", "legendhead")}, text("Heartbeat")),
		legendKey("sw-short", "short = fast run · lots of headroom"),
		legendKey("sw-tall", "tall = long run · nearing timeout"),
		legendKey("sw-late", "amber = used ≥80% of budget"),
		legendKey("sw-down", "red = failed"),
		// A timed run with no ceiling is hollow too (view.Heartbeat).
		legendKey("sw-none", "hollow = success, duration not measurable"),
		legendKey("sw-paused", "flat = paused"))
}
