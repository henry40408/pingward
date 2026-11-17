package views

// HiddenField carries the sibling section's filter, since a GET submit
// replaces the whole query.
type HiddenField struct {
	Name  string
	Value string
}

type PingRow struct {
	Time      string // UTC fallback shown when JS is off
	ISO       string // RFC 3339 UTC; localised to the viewer's zone client-side
	PillClass string // ok|fail|start|log
	KindLabel string
	Exit      string
	Duration  string
	Source    string
	Body      string
}

type NotificationRow struct {
	CreatedAt      string // UTC fallback shown when JS is off
	ISO            string // RFC 3339 UTC; localised client-side
	Event          string // down|up|reminder
	EventPillClass string // mirrors the ping-kind pills
	Status         string
	Channel        string
	Error          string
}

// HistoryFilter is what both history fragments share. Base is "" or "/admin".
type HistoryFilter struct {
	Base    string
	CheckID int64
	// FromUTC and ToUTC are Z-form RFC 3339 ("" = unset), localised client-side.
	FromUTC string
	ToUTC   string
	// Filtered controls the "Clear" affordance.
	Filtered bool
	// Carry re-sends the other section's filter so a scriptless submit keeps it.
	Carry []HiddenField
	// Clear is the clear link; it keeps the other section's filter.
	Clear string
	Newer string // empty = no newer page
	Older string // empty = no older page
}

type CheckPingsView struct {
	HistoryFilter
	Rows []PingRow
	// Kind is "" for all, canonicalised from the query.
	Kind string
}

type CheckNotifsView struct {
	HistoryFilter
	Rows []NotificationRow
	// Event is "" for all: up|down|reminder. Status is "" for all: ok|error.
	Event  string
	Status string
}

func carryInputs(carry []HiddenField) node {
	return each(carry, func(f HiddenField) node {
		return void("input", a("type", "hidden"), a("name", f.Name), a("value", f.Value))
	})
}

func pagerLink(href, testID, label string) node {
	if href == "" {
		return el("span", []attr{a("class", "btn disabled"), a("aria-disabled", "true"), a("data-testid", testID)}, text(label))
	}
	return el("a", []attr{a("class", "btn"), a("href", href), a("data-testid", testID)}, text(label))
}

func pager(testIDPrefix, newer, older, pagerID string) node {
	return el("div", []attr{a("class", "pager"), a("data-testid", pagerID)},
		pagerLink(newer, testIDPrefix+"-newer", "← Newer"),
		pagerLink(older, testIDPrefix+"-older", "Older →"))
}

func dateField(label, name, testID, utc string) node {
	return el("label", []attr{a("class", "fld")}, text(label+" "),
		void("input", a("type", "datetime-local"), a("name", name), a("data-testid", testID), a("data-utc", utc)))
}

func historyForm(f HistoryFilter, testID string, fields ...node) node {
	return el("form", []attr{a("class", "filters"), a("method", "get"), a("action", f.Base+"/checks/"+num(f.CheckID)), a("data-testid", testID), flag("data-nosubmit")},
		fields...)
}

func CheckPings(v CheckPingsView) Markup {
	form := historyForm(v.HistoryFilter, "pings-filters",
		carryInputs(v.Carry),
		dateField("From", "pfrom", "pings-from", v.FromUTC),
		dateField("To", "pto", "pings-to", v.ToUTC),
		el("label", []attr{a("class", "fld")}, text("Kind"),
			el("select", []attr{a("name", "pk"), a("data-testid", "pings-kind")},
				selectOption(v.Kind, "", "All"), selectOption(v.Kind, "success", "success"),
				selectOption(v.Kind, "fail", "fail"), selectOption(v.Kind, "start", "start"),
				selectOption(v.Kind, "log", "log"))),
		el("button", []attr{a("class", "btn"), a("type", "submit"), flag("data-apply"), a("data-testid", "pings-apply")}, text("Filter")),
		when(v.Filtered, el("a", []attr{a("class", "btn ghost"), a("href", v.Clear), a("data-testid", "pings-clear")}, text("Clear"))),
	)
	if len(v.Rows) == 0 {
		empty := "No pings yet."
		if v.Filtered {
			empty = "No pings match the filter."
		}
		return group(form, el("p", []attr{a("class", "empty"), a("data-testid", "pings-empty")}, text(empty)))
	}
	return group(form,
		el("div", []attr{a("class", "tscroll")}, el("table", nil,
			el("thead", nil, el("tr", nil, el("th", nil, text("When")), el("th", nil, text("Kind")), el("th", nil, text("Exit")), el("th", nil, text("Duration")), el("th", nil, text("Source")))),
			el("tbody", nil, each(v.Rows, pingRow)))),
		pager("pings", v.Newer, v.Older, "pings-pager"))
}

func pingRow(p PingRow) node {
	hasBody := p.Body != ""
	caret := "caret"
	if !hasBody {
		caret += " blank"
	}
	row := el("tr", []attr{attrIf(hasBody, "class", "toggle"), a("data-testid", "ping-row")},
		el("td", []attr{a("class", "mono")},
			el("span", []attr{a("class", caret)}, text("▸")),
			el("span", []attr{a("class", "localtime"), a("data-ts", p.ISO)}, text(p.Time))),
		el("td", nil, el("span", []attr{a("class", "pill "+p.PillClass)}, text(p.KindLabel))),
		el("td", []attr{a("class", "mono")}, text(p.Exit)),
		el("td", []attr{a("class", "mono")}, text(p.Duration)),
		el("td", []attr{a("class", "mono")}, text(p.Source)))
	if !hasBody {
		return row
	}
	// .out is white-space: pre-wrap: the body must follow the tag with no whitespace between them.
	return group(row, el("tr", []attr{a("class", "exp")}, el("td", []attr{a("colspan", "5")},
		el("div", []attr{a("class", "out "+p.PillClass)},
			el("span", []attr{a("class", "tag")}, text("captured output · POST body")), text(p.Body)))))
}

func CheckNotifs(v CheckNotifsView) Markup {
	form := historyForm(v.HistoryFilter, "notifs-filters",
		carryInputs(v.Carry),
		dateField("From", "nfrom", "notifs-from", v.FromUTC),
		dateField("To", "nto", "notifs-to", v.ToUTC),
		el("label", []attr{a("class", "fld")}, text("Event"),
			el("select", []attr{a("name", "ne"), a("data-testid", "notifs-event")},
				selectOption(v.Event, "", "All"), selectOption(v.Event, "down", "down"),
				selectOption(v.Event, "up", "up"), selectOption(v.Event, "reminder", "reminder"))),
		el("label", []attr{a("class", "fld")}, text("Result"),
			el("select", []attr{a("name", "ns"), a("data-testid", "notifs-status")},
				selectOption(v.Status, "", "All"), selectOption(v.Status, "ok", "sent"), selectOption(v.Status, "error", "error"))),
		el("button", []attr{a("class", "btn"), a("type", "submit"), flag("data-apply"), a("data-testid", "notifs-apply")}, text("Filter")),
		when(v.Filtered, el("a", []attr{a("class", "btn ghost"), a("href", v.Clear), a("data-testid", "notifs-clear")}, text("Clear"))),
	)
	if len(v.Rows) == 0 {
		empty := "No notifications yet."
		if v.Filtered {
			empty = "No notifications match the filter."
		}
		return group(form, el("p", []attr{a("class", "empty"), a("data-testid", "notifications-empty")}, text(empty)))
	}
	return group(form,
		el("div", []attr{a("class", "tscroll")}, el("table", nil,
			el("thead", nil, el("tr", nil, el("th", nil, text("When")), el("th", nil, text("Event")), el("th", nil, text("Channel")), el("th", nil, text("Result")))),
			el("tbody", nil, each(v.Rows, notificationRow)))),
		pager("notif", v.Newer, v.Older, "notif-pager"))
}

func notificationRow(n NotificationRow) node {
	result := el("td", []attr{a("class", "mono fail-txt")}, text(n.Error))
	if n.Status == "ok" {
		result = el("td", []attr{a("class", "mono ok-txt")}, text("sent"))
	}
	return el("tr", nil,
		el("td", []attr{a("class", "mono")}, el("span", []attr{a("class", "localtime"), a("data-ts", n.ISO)}, text(n.CreatedAt))),
		el("td", nil, el("span", []attr{a("class", "pill "+n.EventPillClass)}, text(n.Event))),
		el("td", nil, text(n.Channel)),
		result)
}
