package views

import (
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/view"
)

type ChannelBox struct {
	ID    int64
	Name  string
	Kind  string
	Bound bool
}

// CheckView feeds the check page. Admin switches the links to /admin.
type CheckView struct {
	CSRF            string
	IsAdmin         bool
	Admin           bool
	Check           models.Check
	DescriptionHTML string // output of markdown.Render, already escaped
	ProjectName     string
	Status          string
	Since           string
	NextDue         view.NextDue
	Schedule        string
	PingURL         string
	// PingURLHidden withholds the URL pending an audited reveal.
	PingURLHidden bool
	Bars          []view.Bar
	ChannelBoxes  []ChannelBox
	// The same fragments the history endpoints serve.
	PingsPartial  Markup
	NotifsPartial Markup
	Flash         string // empty = none
}

func Check(v CheckView) string {
	base := ""
	if v.Admin {
		base = "/admin"
	}
	id := num(v.Check.ID)
	body := group(
		when(v.Flash != "", el("p", []attr{a("class", "flash ok"), a("data-testid", "check-flash")}, text(v.Flash))),
		el("div", []attr{a("class", "crumb")},
			el("a", []attr{a("href", "/")}, text("Checks")), el("span", []attr{a("class", "sep")}, text("/")),
			el("a", []attr{a("href", base+"/projects/"+num(v.Check.ProjectID))}, text(v.ProjectName)),
			el("span", []attr{a("class", "sep")}, text("/")), text(v.Check.Name)),
		checkHeader(v),
		el("div", []attr{a("class", "crumb tight")}, text(v.Schedule)),
		// markdown.Render escapes first, then re-adds a whitelist of tags.
		when(v.DescriptionHTML != "", el("div", []attr{a("class", "desc"), a("data-testid", "check-description")}, raw(v.DescriptionHTML))),
		checkActions(v, base),
		pingCard(v, base),
		notifyChannelsCard(v, base),
		el("div", []attr{a("class", "card"), a("id", "pings-card")},
			el("div", []attr{a("class", "ch")},
				el("h2", nil, text("Recent pings")),
				el("button", []attr{a("class", "btn live-toggle"), a("type", "button"), a("id", "pings-live"), a("data-testid", "pings-live"), a("aria-pressed", "false"), a("data-endpoint", base+"/checks/"+id+"/events")}, text("LIVE"))),
			el("div", []attr{a("class", "cb"), a("id", "pings-section"), a("data-endpoint", base+"/checks/"+id+"/pings")}, v.PingsPartial)),
		el("div", []attr{a("class", "card flush")},
			el("div", []attr{a("class", "ch")}, el("h2", nil, text("Recent notifications"))),
			el("div", []attr{a("class", "cb"), a("id", "notifs-section"), a("data-endpoint", base+"/checks/"+id+"/notifications")}, v.NotifsPartial)),
	)
	return Page(navChrome(v.IsAdmin, v.CSRF), body)
}

func checkHeader(v CheckView) node {
	return el("div", []attr{a("class", "chead")},
		el("span", []attr{a("class", "status-dot "+v.Status)}),
		el("h1", nil, text(v.Check.Name)),
		el("span", []attr{a("class", "badge "+v.Status), a("data-testid", "check-status")}, text(v.Status)),
		el("span", []attr{a("class", "cmeta")},
			el("span", []attr{a("class", "since "+v.Status)}, text(v.Since)),
			el("span", []attr{a("class", "due"), a("data-testid", "check-next-due"), attrIf(v.NextDue.ISO != "", "title", v.NextDue.ISO)}, text(v.NextDue.Label))))
}

func postForm(csrf, action, class string, extra []attr, button node) node {
	attrs := append([]attr{a("class", class), a("method", "post"), a("action", action)}, extra...)
	return el("form", attrs, CSRFField(csrf), button)
}

func checkActions(v CheckView, base string) node {
	id := num(v.Check.ID)
	act := func(name string) string { return base + "/checks/" + id + "/" + name }
	btn := func(class, testID, label string) node {
		return el("button", []attr{a("class", class), a("data-testid", testID)}, text(label))
	}
	var pauseResume node
	if v.Check.Status == models.StatusPaused {
		pauseResume = postForm(v.CSRF, act("resume"), "inline", nil, btn("btn", "resume-button", "Resume"))
	} else {
		pauseResume = postForm(v.CSRF, act("pause"), "inline", nil, btn("btn", "pause-button", "Pause"))
	}
	return el("div", []attr{a("class", "actions")},
		el("a", []attr{a("class", "btn"), a("href", act("edit"))}, text("Edit")),
		pauseResume,
		when(v.Status == "down" && !v.Check.Acknowledged,
			postForm(v.CSRF, act("ack"), "inline", nil, btn("btn primary", "ack-button", "Acknowledge"))),
		postForm(v.CSRF, act("regenerate"), "inline",
			[]attr{a("data-confirm", "Regenerate the ping URL? The current URL stops working immediately and any job using it must be updated.")},
			btn("btn danger", "regenerate-button", "Regenerate URL")),
		postForm(v.CSRF, act("delete"), "inline", []attr{a("data-confirm", "Delete this check?")},
			btn("btn danger", "delete-check-button", "Delete")),
	)
}

func pingCard(v CheckView, base string) node {
	urlPart := pingURLHelp(v.PingURL)
	if v.PingURLHidden {
		urlPart = withheldURL(v, base)
	}
	return el("div", []attr{a("class", "card")}, el("div", []attr{a("class", "cb")},
		urlPart,
		// No run count: only the browser knows how many bars are visible.
		el("div", []attr{a("class", "beat"), a("title", "Most recent runs, newest on the right")}, heartbeatBars(v.Bars)),
		// "taller = longer" holds only for solid bars; hollow ones are full height.
		el("div", []attr{a("class", "beatcap")},
			el("span", nil, text("older")),
			el("span", []attr{a("class", "key")}, text("solid = timed run, taller = longer · hollow = duration not measurable · amber ≥80% budget · red = failed")),
			el("span", nil, text("now")))))
}

// withheldURL is the cross-user admin view: the URL is a credential, so it
// (and the usage help repeating it) stays hidden until revealed.
func withheldURL(v CheckView, base string) node {
	return group(
		el("div", []attr{a("class", "urlrow withheld"), a("data-testid", "ping-url-withheld")},
			el("span", []attr{a("class", "note")}, text("Ping URL hidden. It is a credential — anyone holding it can mark this check up or down.")),
			el("form", []attr{a("class", "inline"), a("method", "post"), a("action", base+"/checks/"+num(v.Check.ID)+"/ping-url")},
				CSRFField(v.CSRF),
				el("button", []attr{a("class", "btn"), a("type", "submit"), a("data-testid", "reveal-ping-url")}, text("Reveal ping URL")))),
		el("p", []attr{a("class", "note"), a("data-testid", "ping-url-reveal-note")}, text("Revealing it is recorded in the audit trail, with your username and this check.")))
}

func signalRow(pillClass, pillText, url, suffix string, effect node) node {
	return el("tr", nil,
		el("td", nil, el("span", []attr{a("class", "pill "+pillClass)}, text(pillText))),
		el("td", []attr{a("class", "mono")}, text(url+suffix)),
		el("td", nil, effect))
}

func pingURLHelp(url string) node {
	return group(
		el("div", []attr{a("class", "urlrow")},
			el("code", []attr{a("data-testid", "ping-url")}, text(url)),
			el("button", []attr{a("class", "copy"), a("type", "button"), a("data-copy", url)}, text("Copy"))),
		el("details", []attr{a("class", "pinghelp"), a("data-testid", "ping-help")},
			el("summary", nil, text("How do I ping this check?")),
			el("div", []attr{a("class", "pinghelp-body")},
				el("p", nil, text("Have your job call the URL above when it runs — a plain "), el("code", nil, text("GET")),
					text(" or "), el("code", nil, text("POST")), text(" reports "), el("strong", nil, text("success")),
					text(". Add a suffix for other signals:")),
				el("div", []attr{a("class", "tscroll")}, el("table", nil,
					el("thead", nil, el("tr", nil, el("th", nil, text("Signal")), el("th", nil, text("URL")), el("th", nil, text("Effect")))),
					el("tbody", nil,
						signalRow("ok", "success", url, "", text("marks the check up")),
						signalRow("fail", "fail", url, "/fail", text("marks the check down")),
						signalRow("start", "start", url, "/start", text("opens a run so an overrun can be caught")),
						signalRow("log", "log", url, "/log", text("records a note, status unchanged")),
						el("tr", nil,
							el("td", []attr{a("class", "mono")}, text("exit code")),
							el("td", []attr{a("class", "mono")}, text(url+"/<code>")),
							el("td", nil, el("code", nil, text("0")), text(" = up, anything else = down")))))),
				// .out is white-space: pre-wrap: the newlines below are the display.
				el("div", []attr{a("class", "out log pinghelp-code")},
					el("span", []attr{a("class", "tag")}, text("shell examples")),
					text("# report success when the job finishes\ncurl -fsS "+url+
						"\n\n# wrap a job: start it, run, then report the exit code\ncurl -fsS "+url+
						"/start\nmybackup.sh\ncurl -fsS "+url+
						"/$?\n\n# report failure and attach the output (shown in Recent pings)\nmybackup.sh 2>&1 | curl -fsS --data-binary @- "+url+"/fail")))))
}

func notifyChannelsCard(v CheckView, base string) node {
	id := num(v.Check.ID)
	var content node
	if len(v.ChannelBoxes) == 0 {
		content = el("p", []attr{a("class", "note"), a("data-testid", "check-channels-empty")},
			text("No channels yet. "),
			el("a", []attr{a("href", base+"/projects/"+num(v.Check.ProjectID)+"/channels/new")}, text("Add one")),
			text(" to get notified when this check goes down."))
	} else {
		content = el("form", []attr{a("method", "post"), a("action", base+"/checks/"+id+"/channels")},
			CSRFField(v.CSRF),
			each(v.ChannelBoxes, channelBox),
			el("div", []attr{a("class", "formactions")}, el("button", []attr{a("class", "btn primary"), a("type", "submit")}, text("Save channels"))))
	}
	return el("div", []attr{a("class", "card")},
		el("div", []attr{a("class", "ch")}, el("h2", nil, text("Notify channels"))),
		el("div", []attr{a("class", "cb")}, content))
}

func channelBox(cb ChannelBox) node {
	return el("label", []attr{a("class", "chk")},
		void("input", a("type", "checkbox"), a("name", "channel_ids"), a("value", num(cb.ID)), flagIf(cb.Bound, "checked")),
		el("span", []attr{a("class", "nm")}, text(cb.Name)),
		el("span", []attr{a("class", "kind")}, text(cb.Kind)),
		el("span", []attr{a("class", "onoff"), a("data-testid", "channel-state-"+num(cb.ID))},
			el("span", []attr{a("class", "on")}, text("ON")), el("span", []attr{a("class", "off")}, text("OFF"))))
}
