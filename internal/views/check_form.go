package views

// CheckFormView feeds the new and edit check forms.
type CheckFormView struct {
	CSRF             string
	IsAdmin          bool
	Heading          string
	Action           string
	Error            string // empty = none
	Name             string
	Description      string
	ScheduleKind     string
	PeriodSecs       string
	CronExpr         string
	GraceSecs        string
	Timezone         string
	ScanIntervalSecs string
	MaxRuntimeSecs   string
	NagIntervalSecs  string
	// Timezones fill the timezone <datalist>: every IANA name, in chrono-tz order.
	Timezones []string
}

func scheduleOption(current, value string) node {
	return el("option", []attr{a("value", value), flagIf(current == value, "selected")}, text(value))
}

func CheckForm(v CheckFormView) string {
	body := group(
		el("h1", nil, text(v.Heading)),
		when(v.Error != "", el("p", []attr{a("class", "flash err")}, text(v.Error))),
		el("div", []attr{a("class", "card")}, el("div", []attr{a("class", "cb")},
			el("form", []attr{a("method", "post"), a("action", v.Action)},
				CSRFField(v.CSRF),
				// Suggestions for the five duration fields below; the server still validates.
				durationDatalist(),
				el("div", []attr{a("class", "field")},
					el("label", []attr{a("for", "name")}, text("Name")),
					void("input", a("id", "name"), a("name", "name"), a("value", v.Name), a("data-testid", "check-name-input"), flag("required"))),
				descriptionField("check-description-input", v.Description),
				el("div", []attr{a("class", "field")},
					el("label", []attr{a("for", "schedule_kind")}, text("Schedule kind")),
					el("select", []attr{a("id", "schedule_kind"), a("name", "schedule_kind")},
						scheduleOption(v.ScheduleKind, "period"), scheduleOption(v.ScheduleKind, "cron"))),
				el("div", []attr{a("class", "sched"), a("data-sched", "period")},
					el("div", []attr{a("class", "field")},
						el("label", []attr{a("for", "period_secs")}, text("Period")),
						void("input", a("id", "period_secs"), a("name", "period_secs"), a("value", v.PeriodSecs), a("list", "dur-list"), a("autocomplete", "off"), a("data-testid", "check-period-input")),
						el("div", []attr{a("class", "help")}, text("Seconds, or a duration like 5m, 1h30m, 2d.")))),
				el("div", []attr{a("class", "sched"), a("data-sched", "cron")},
					el("div", []attr{a("class", "field")},
						el("label", []attr{a("for", "cron_expr")}, text("Cron expression")),
						void("input", a("id", "cron_expr"), a("name", "cron_expr"), a("value", v.CronExpr)),
						el("div", []attr{a("class", "help")}, text(`6-field "sec min hour dom mon dow".`)))),
				el("div", []attr{a("class", "two")},
					el("div", []attr{a("class", "field")},
						el("label", []attr{a("for", "grace_secs")}, text("Grace")),
						void("input", a("id", "grace_secs"), a("name", "grace_secs"), a("value", v.GraceSecs), a("list", "dur-list"), a("autocomplete", "off"), flag("required")),
						el("div", []attr{a("class", "help")}, text("Seconds, or a duration like 5m, 1h30m."))),
					el("div", []attr{a("class", "field")},
						el("label", []attr{a("for", "timezone")}, text("Timezone")),
						// A datalist, not a <select>, for type-to-filter; the handler validates either way.
						void("input", a("id", "timezone"), a("name", "timezone"), a("value", v.Timezone), a("list", "tz-list"), a("autocomplete", "off"), flag("required")),
						el("datalist", []attr{a("id", "tz-list")}, each(v.Timezones, func(tz string) node { return el("option", []attr{a("value", tz)}) })),
						el("div", []attr{a("class", "help")}, text("IANA name. Only cron schedules use it to pick the firing wall clock.")))),
				durationField("scan_interval_secs", "Scan interval", v.ScanIntervalSecs, "Leave blank to inherit from the project. Seconds, or a duration like 5m, 1h30m."),
				durationField("max_runtime_secs", "Max runtime", v.MaxRuntimeSecs, "Leave blank to turn off the runtime budget. Seconds, or a duration like 5m, 1h30m."),
				durationField("nag_interval_secs", "Nag interval", v.NagIntervalSecs, "Leave blank to inherit from the project, or to turn off nagging. Seconds, or a duration like 5m, 1h30m."),
				el("div", []attr{a("class", "btnrow")},
					el("button", []attr{a("class", "btn primary"), a("type", "submit"), a("data-testid", "check-submit")}, text("Save changes")))))),
	)
	return Page(navChrome(v.IsAdmin, v.CSRF), body)
}
