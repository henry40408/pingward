package views

import (
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/view"
)

// ProjectFormView feeds the new and edit project forms.
type ProjectFormView struct {
	CSRF             string
	IsAdmin          bool
	Heading          string
	Action           string
	Name             string
	Description      string
	ScanIntervalSecs string
	NagIntervalSecs  string
	Error            string // empty = none
}

// ProjectView feeds the project page. Admin switches the links to /admin.
type ProjectView struct {
	CSRF            string
	IsAdmin         bool
	Admin           bool
	Project         models.Project
	DescriptionHTML string // output of markdown.Render, already escaped
	Checks          []ProjectCheckRow
	Channels        []ProjectChannelRow
	TestResult      *TestResult
}

// ProjectCheckRow carries a precomputed Status because it needs the current time.
type ProjectCheckRow struct {
	ID          int64
	Name        string
	Status      string
	Schedule    string
	Description string // single-line plain-text summary
	NoChannel   bool
}

// ProjectChannelRow is projected so a channel's config_json (delivery secrets)
// never reaches the view.
type ProjectChannelRow struct {
	ID   int64
	Name string
	Kind string
}

type TestResult struct {
	OK      bool
	Message string
}

func noChannelChip() node {
	return el("span", []attr{a("class", "nochan"), a("data-testid", "check-no-channel"), a("title", "No notification channel is bound — nobody is alerted when this check goes down")}, text("no channel"))
}

func durationDatalist() node {
	return el("datalist", []attr{a("id", "dur-list")}, durationOptions())
}

func ProjectForm(v ProjectFormView) string {
	body := group(
		el("h1", nil, text(v.Heading)),
		when(v.Error != "", el("p", []attr{a("class", "flash err")}, text(v.Error))),
		el("div", []attr{a("class", "card")}, el("div", []attr{a("class", "cb")},
			el("form", []attr{a("method", "post"), a("action", v.Action)},
				CSRFField(v.CSRF),
				durationDatalist(),
				el("div", []attr{a("class", "field")},
					el("label", []attr{a("for", "name")}, text("Name")),
					void("input", a("id", "name"), a("name", "name"), a("value", v.Name), a("data-testid", "project-name-input"), flag("required"))),
				descriptionField("project-description-input", v.Description),
				durationField("scan_interval_secs", "Scan interval", v.ScanIntervalSecs, "Leave blank to inherit the default scan interval. Seconds, or a duration like 5m, 1h30m."),
				durationField("nag_interval_secs", "Nag interval", v.NagIntervalSecs, "Leave blank to inherit, or to turn off nagging. Seconds, or a duration like 5m, 1h30m."),
				el("div", []attr{a("class", "btnrow")},
					el("button", []attr{a("class", "btn primary"), a("type", "submit"), a("data-testid", "project-submit")}, text("Save changes")))))),
	)
	return Page(navChrome(v.IsAdmin, v.CSRF), body)
}

func durationOptions() node {
	return each(view.DurationSuggestions(), func(d string) node { return el("option", []attr{a("value", d)}) })
}

func descriptionField(testID, value string) node {
	return el("div", []attr{a("class", "field")},
		el("label", []attr{a("for", "description")}, text("Description")),
		el("textarea", []attr{a("id", "description"), a("name", "description"), a("rows", "4"), a("data-testid", testID)}, text(value)),
		el("div", []attr{a("class", "help")}, text("Optional. Supports **bold**, *italic*, `code`, [links](url) and - lists.")))
}

// durationField is a text input offering the shared duration suggestions.
func durationField(name, label, value, help string) node {
	return el("div", []attr{a("class", "field")},
		el("label", []attr{a("for", name)}, text(label)),
		void("input", a("id", name), a("name", name), a("value", value), a("list", "dur-list"), a("autocomplete", "off")),
		el("div", []attr{a("class", "help")}, text(help)))
}

func Project(v ProjectView) string {
	base := ""
	if v.Admin {
		base = "/admin"
	}
	id := num(v.Project.ID)
	flashClass := "flash err"
	if v.TestResult != nil && v.TestResult.OK {
		flashClass = "flash ok"
	}
	body := group(
		el("div", []attr{a("class", "phead")}, el("div", nil, el("h1", nil, text(v.Project.Name)))),
		// markdown.Render escapes first, then re-adds a whitelist of tags.
		when(v.DescriptionHTML != "", el("div", []attr{a("class", "desc"), a("data-testid", "project-description")}, raw(v.DescriptionHTML))),
		el("div", []attr{a("class", "actions")},
			el("a", []attr{a("class", "btn"), a("href", base+"/projects/"+id+"/edit")}, text("Edit")),
			el("a", []attr{a("class", "btn"), a("href", base+"/projects/"+id+"/checks/new"), a("data-testid", "new-check-link")}, text("New check")),
			el("a", []attr{a("class", "btn"), a("href", base+"/projects/"+id+"/channels/new")}, text("New channel")),
			el("form", []attr{a("class", "inline"), a("method", "post"), a("action", base+"/projects/"+id+"/delete"), a("data-confirm", "Delete project?")},
				CSRFField(v.CSRF),
				el("button", []attr{a("class", "btn danger"), a("type", "submit"), a("data-testid", "delete-project-button")}, text("Delete project")))),
		when(v.TestResult != nil, el("p", []attr{a("class", flashClass)}, text(testMessage(v.TestResult)))),
		checksGroup(v, base),
		channelsCard(v, base),
	)
	return Page(navChrome(v.IsAdmin, v.CSRF), body)
}

func testMessage(result *TestResult) string {
	if result == nil {
		return ""
	}
	return result.Message
}

func checksGroup(v ProjectView, base string) node {
	return el("div", []attr{a("class", "group")},
		el("div", []attr{a("class", "gh")}, el("h2", nil, text("Checks")), el("span", []attr{a("class", "line")})),
		el("div", []attr{a("class", "list")},
			when(len(v.Checks) == 0, el("p", []attr{a("class", "empty"), a("data-testid", "checks-empty")}, text("No checks yet."))),
			each(v.Checks, func(c ProjectCheckRow) node { return projectCheckRow(c, base) })))
}

func projectCheckRow(c ProjectCheckRow, base string) node {
	href := base + "/checks/" + num(c.ID)
	return el("div", []attr{a("class", "check"), a("data-href", href)},
		el("span", []attr{a("class", "status-dot "+c.Status)}),
		el("div", []attr{a("class", "cmeta")},
			el("div", []attr{a("class", "nm")}, el("a", []attr{a("href", href)}, text(c.Name))),
			el("div", []attr{a("class", "sc")}, text(c.Schedule)),
			when(c.Description != "", el("div", []attr{a("class", "cdesc"), a("data-testid", "check-description-summary")}, text(c.Description)))),
		when(c.NoChannel, noChannelChip()),
		el("a", []attr{a("class", "rowedit"), a("href", href+"/edit")}, text("edit")),
		el("span", []attr{a("class", "badge "+c.Status)}, text(c.Status)))
}

func channelsCard(v ProjectView, base string) node {
	return el("div", []attr{a("class", "card flush")},
		el("div", []attr{a("class", "ch")}, el("h2", nil, text("Channels"))),
		el("div", []attr{a("class", "cb")},
			when(len(v.Channels) == 0, el("p", []attr{a("class", "flash warn"), a("data-testid", "project-channels-empty")},
				text("No channels yet — when a check in this project goes down, nobody is notified. "),
				el("a", []attr{a("href", base+"/projects/"+num(v.Project.ID)+"/channels/new")}, text("Add a channel")),
				text("."))),
			each(v.Channels, func(ch ProjectChannelRow) node { return channelRow(ch, base, v.CSRF) })))
}

func channelRow(ch ProjectChannelRow, base, csrf string) node {
	id := num(ch.ID)
	return el("div", []attr{a("class", "chk readonly")},
		el("span", []attr{a("class", "nm")}, text(ch.Name)),
		el("span", []attr{a("class", "kind")}, text(ch.Kind)),
		el("a", []attr{a("class", "btn ml-8"), a("href", base+"/channels/"+id+"/edit"), a("data-testid", "channel-edit-"+id)}, text("edit")),
		el("form", []attr{a("class", "inline"), a("method", "post"), a("action", base+"/channels/"+id+"/test")},
			CSRFField(csrf), el("button", []attr{a("class", "btn"), a("type", "submit")}, text("Send test"))),
		el("form", []attr{a("class", "inline"), a("method", "post"), a("action", base+"/channels/"+id+"/delete"), a("data-confirm", "Delete channel?")},
			CSRFField(csrf), el("button", []attr{a("class", "btn danger"), a("type", "submit")}, text("delete"))))
}
