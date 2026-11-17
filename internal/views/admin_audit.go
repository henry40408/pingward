package views

// AuditRow shows four columns (when, actor, action, target); the rest of an
// audit entry sits in an expandable row.
type AuditRow struct {
	Time, ISO, Actor, Action, Target string
	MethodPath, Detail, TargetOwner  string
	// Expandable is false when every expanded field is unset, so no caret
	// points at an empty box.
	Expandable bool
}

// AdminAuditView is the audit-trail fragment of /admin and /admin/audit.
type AdminAuditView struct {
	Rows []AuditRow
	// Actors and Actions list everything present in the trail, for the two filter selects.
	Actors, Actions []string
	// Selected filters ("" = all), echoed back into the controls.
	FilterActor, FilterAction, FilterFrom, FilterTo string
	// Filtered switches the empty state's wording and shows the Clear link.
	Filtered bool
	Newer    string
	Older    string
}

func AdminAudit(v AdminAuditView) Markup {
	filters := el("form", []attr{a("class", "filters"), a("method", "get"), a("action", "/admin"), a("data-testid", "audit-filters"), flag("data-nosubmit")},
		dateField("From", "afrom", "audit-from", v.FilterFrom),
		dateField("To", "ato", "audit-to", v.FilterTo),
		el("label", []attr{a("class", "fld")}, text("Actor"), filterSelect("aactor", "audit-actor", v.FilterActor, v.Actors)),
		el("label", []attr{a("class", "fld")}, text("Action"), filterSelect("aaction", "audit-action", v.FilterAction, v.Actions)),
		el("button", []attr{a("class", "btn"), a("type", "submit"), flag("data-apply"), a("data-testid", "audit-apply")}, text("Filter")),
		when(v.Filtered, el("a", []attr{a("class", "btn ghost"), a("href", "/admin/audit"), a("data-testid", "audit-clear")}, text("Clear"))))
	if len(v.Rows) == 0 {
		message := "No audit entries yet."
		if v.Filtered {
			message = "No audit entries match the filter."
		}
		return group(filters, el("p", []attr{a("class", "empty"), a("data-testid", "audit-empty")}, text(message)))
	}
	return group(filters,
		el("div", []attr{a("class", "tscroll")}, el("table", nil,
			headRow("When", "Actor", "Action", "Target"),
			el("tbody", nil, each(v.Rows, auditRow)))),
		pager("audit", v.Newer, v.Older, "audit-pager"))
}

func filterSelect(name, testID, selected string, values []string) node {
	option := func(value, label string) node {
		return el("option", []attr{a("value", value), flagIf(selected == value, "selected")}, text(label))
	}
	return el("select", []attr{a("name", name), a("data-testid", testID)},
		option("", "All"),
		each(values, func(value string) node { return option(value, value) }))
}

func auditRow(r AuditRow) node {
	caretClass := "caret"
	if !r.Expandable {
		caretClass += " blank"
	}
	rowClass := attrIf(r.Expandable, "class", "toggle")
	summary := el("tr", []attr{rowClass, a("data-testid", "audit-row")},
		el("td", []attr{a("class", "mono")},
			el("span", []attr{a("class", caretClass)}, text("▸")),
			el("span", []attr{a("class", "localtime"), a("data-ts", r.ISO)}, text(r.Time))),
		el("td", nil, text(r.Actor)),
		el("td", []attr{a("class", "mono")}, text(r.Action)),
		el("td", []attr{a("class", "mono")}, text(r.Target)))
	if !r.Expandable {
		return summary
	}
	// .out is white-space: pre-wrap, so the newlines are the display.
	detail := r.MethodPath + "\ntarget owner: " + r.TargetOwner
	if r.Detail != "" {
		detail += "\n" + r.Detail
	}
	return group(summary, el("tr", []attr{a("class", "exp")}, el("td", []attr{a("colspan", "4")},
		el("div", []attr{a("class", "out log")}, el("span", []attr{a("class", "tag")}, text("request · detail")), text(detail)))))
}
