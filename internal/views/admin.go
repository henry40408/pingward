package views

import (
	"time"

	"github.com/henry40408/pingward/internal/view"
)

// EnvKind says how an environment setting is shown.
type EnvKind int

const (
	EnvText         EnvKind = iota // the value, in <code>
	EnvUnset                       // "not set"
	EnvSecretSet                   // "configured", never the value
	EnvSecretNotSet                // "not set" for a secret
)

// EnvSetting is one environment variable. Secrets are redacted before they
// reach a view, so no view can print one.
type EnvSetting struct {
	Var         string
	Kind        EnvKind
	Value       string // only read for EnvText
	Default     string
	Description string
}

// EnvGroup is a titled table of settings.
type EnvGroup struct {
	Title string
	Rows  []EnvSetting
}

// AdminUserRow marks the admin's own row so its self-mutation controls render inert.
type AdminUserRow struct {
	ID       int64
	Username string
	IsAdmin  bool
	Disabled bool
	IsSelf   bool
}

type StatusCounts struct{ New, Up, Running, Down, Paused int64 }

type DownCheck struct {
	ID      int64
	Name    string
	Project string
	Owner   string
}

type ChannelFailure struct {
	Name   string
	OK     int64
	Failed int64
}

type FailedNotification struct {
	CreatedAt time.Time
	Event     string
	CheckID   int64
	Error     string
}

type AdminProjectRow struct {
	ID    int64
	Name  string
	Owner string
}

// LoopStamp is when a background loop last ran. At is RFC 3339 ("" = never);
// UTC and Ago are the server-rendered fallbacks that app.js re-ticks.
type LoopStamp struct {
	At, UTC, Ago string
}

// AdminView is the whole /admin page.
type AdminView struct {
	CSRF string
	// AuditPartial is composed as Markup so /admin and /admin/audit emit identical markup.
	AuditPartial Markup
	// Overview.
	UserCount, ProjectCount, Checks, Pings24h int64
	Status                                    StatusCounts
	Down                                      []DownCheck
	NotifOK, NotifErr                         int64
	ChannelFailures                           []ChannelFailure
	RecentFailures                            []FailedNotification
	LastScan, LastPrune                       LoopStamp
	// Settings.
	ScanInterval, NagInterval, PingsRetentionDays, NotificationsRetentionDays, AuditRetentionDays, DisplayTimezone string
	Timezones                                                                                                      []string
	SettingsError, SettingsFlash                                                                                   string
	// Users.
	Users          []AdminUserRow
	UserFlash      string
	ElevationFlash string
	// ElevationApplies is false for a passwordless forward-auth admin.
	ElevationApplies bool
	// ElevationRemaining is the readable duration while unlocked, "" when locked.
	ElevationRemaining string
	// ElevationLocked means the gate applies and is unconfirmed; only then do gated
	// controls carry data-reauth. The server re-checks regardless.
	ElevationLocked    bool
	PasswordResetFlash string
	UserError          string
	Projects           []AdminProjectRow
	Env                []EnvGroup
}

func Admin(v AdminView) string {
	body := group(
		el("div", []attr{a("class", "phead")}, el("div", nil,
			el("h1", nil, text("Admin")),
			el("p", []attr{a("class", "crumb tight")}, text("Site-wide overview, global settings, user management, and every project across all users.")))),
		scaleTiles(v),
		checkHealthCard(v),
		notificationHealthCard(v),
		schedulerCard(v),
		settingsCard(v),
		environmentCard(v),
		when(v.ElevationApplies, elevationCard(v)),
		usersCard(v),
		projectsGroup(v),
		el("div", []attr{a("class", "card")},
			el("div", []attr{a("class", "ch")}, el("h2", nil, text("Audit trail"))),
			el("div", []attr{a("class", "cb"), a("id", "audit-section"), a("data-endpoint", "/admin/audit")}, v.AuditPartial)),
	)
	return Page(navChrome(true, v.CSRF), body)
}

func scaleTiles(v AdminView) node {
	tile := func(n int64, label string) node {
		return el("div", []attr{a("class", "tile total")},
			el("span", []attr{a("class", "edge")}),
			el("div", []attr{a("class", "n")}, text(num(n))),
			el("div", []attr{a("class", "l")}, text(label)))
	}
	// Bare .tiles, as on the dashboard: a tile is already a card.
	return el("div", []attr{a("class", "tiles"), a("data-testid", "admin-scale")},
		tile(v.UserCount, "Users"), tile(v.ProjectCount, "Projects"), tile(v.Checks, "Checks"), tile(v.Pings24h, "Pings (24h)"))
}

func plainTable(testID, tableClass string, head node, rows node) node {
	attrs := []attr{attrIf(testID != "", "data-testid", testID), attrIf(tableClass != "", "class", tableClass)}
	return el("div", []attr{a("class", "tscroll")}, el("table", attrs, head, el("tbody", nil, rows)))
}

func checkHealthCard(v AdminView) node {
	badge := func(class string, n int64) node {
		return el("span", []attr{a("class", "badge "+class)}, text(num(n)+" "+class))
	}
	s := v.Status
	down := el("p", []attr{a("class", "note")}, text("No checks are currently down."))
	if len(v.Down) > 0 {
		down = plainTable("health-down", "", headRow("Check", "Project", "Owner"), each(v.Down, func(d DownCheck) node {
			return el("tr", nil,
				el("td", nil, el("a", []attr{a("href", "/admin/checks/"+num(d.ID))}, text(d.Name))),
				el("td", nil, text(d.Project)),
				el("td", nil, text(d.Owner)))
		}))
	}
	return el("div", []attr{a("class", "card")},
		el("div", []attr{a("class", "ch")}, el("h2", nil, text("Check health"))),
		el("div", []attr{a("class", "cb")},
			el("p", []attr{a("class", "pillrow")}, badge("new", s.New), badge("up", s.Up), badge("running", s.Running), badge("down", s.Down), badge("paused", s.Paused)),
			down))
}

func notificationHealthCard(v AdminView) node {
	channels := el("p", []attr{a("class", "note")}, text("No notifications sent in the last 24h."))
	if len(v.ChannelFailures) > 0 {
		channels = plainTable("health-channels", "", headRow("Channel", "Ok", "Error"), each(v.ChannelFailures, func(c ChannelFailure) node {
			return el("tr", nil,
				el("td", nil, text(c.Name)),
				el("td", []attr{a("class", "mono ok-txt")}, text(num(c.OK))),
				el("td", []attr{a("class", "mono fail-txt")}, text(num(c.Failed))))
		}))
	}
	recent := el("p", []attr{a("class", "note")}, text("No recent notification failures."))
	if len(v.RecentFailures) > 0 {
		recent = plainTable("health-recent", "", headRow("When", "Event", "Check", "Error"), each(v.RecentFailures, func(n FailedNotification) node {
			return el("tr", nil,
				el("td", []attr{a("class", "mono")}, localTime(n.CreatedAt)),
				el("td", nil, text(n.Event)),
				el("td", nil, el("a", []attr{a("href", "/admin/checks/"+num(n.CheckID))}, text("#"+num(n.CheckID)))),
				el("td", []attr{a("class", "fail-txt")}, text(n.Error)))
		}))
	}
	return el("div", []attr{a("class", "card")},
		el("div", []attr{a("class", "ch")}, el("h2", nil, text("Notification health (24h)"))),
		el("div", []attr{a("class", "cb")},
			el("p", []attr{a("class", "pillrow")},
				el("span", []attr{a("class", "pill ok")}, text(num(v.NotifOK)+" ok")),
				el("span", []attr{a("class", "pill fail")}, text(num(v.NotifErr)+" error"))),
			channels,
			el("h2", []attr{a("class", "subhead")}, text("Recent failures")),
			recent))
}

func loopStamp(testID, label string, stamp LoopStamp) node {
	never := stamp.At == ""
	dotClass := "hb-dot"
	if never {
		dotClass += " idle"
	}
	timing := group(
		el("div", []attr{a("class", "hb-time localtime"), a("data-ts", stamp.At)}, text(firstNonEmpty(stamp.UTC, stamp.At))),
		el("div", []attr{a("class", "hb-ago"), a("data-ago", stamp.At)}, text(stamp.Ago)))
	if never {
		timing = el("div", []attr{a("class", "hb-time hb-never")}, text("never"))
	}
	return el("div", []attr{a("class", "hb"), a("data-testid", testID)},
		el("span", []attr{a("class", dotClass)}),
		el("div", []attr{a("class", "hb-body")}, el("div", []attr{a("class", "hb-label")}, text(label)), timing))
}

func firstNonEmpty(first, second string) string {
	if first != "" {
		return first
	}
	return second
}

func schedulerCard(v AdminView) node {
	return el("div", []attr{a("class", "card")},
		el("div", []attr{a("class", "ch")}, el("h2", nil, text("Scheduler"))),
		el("div", []attr{a("class", "cb")}, el("div", []attr{a("class", "heartbeats")},
			loopStamp("sched-scan", "Last scan", v.LastScan),
			loopStamp("sched-prune", "Last prune", v.LastPrune))))
}

func settingField(id, label, value, help string, extra ...attr) node {
	attrs := append([]attr{a("id", id), a("name", id), a("value", value)}, extra...)
	return el("div", []attr{a("class", "field")},
		el("label", []attr{a("for", id)}, text(label)),
		void("input", attrs...),
		el("div", []attr{a("class", "help")}, text(help)))
}

func settingsCard(v AdminView) node {
	durations := each(view.DurationSuggestions(), func(d string) node { return el("option", []attr{a("value", d)}) })
	durationAttrs := []attr{a("list", "dur-list"), a("autocomplete", "off")}
	const durationHelp = "Seconds, or a duration like 5m, 1h30m."
	timezone := el("div", []attr{a("class", "field")},
		el("label", []attr{a("for", "display_timezone")}, text("Notification timezone")),
		void("input", a("id", "display_timezone"), a("name", "display_timezone"), a("value", v.DisplayTimezone), a("list", "tz-list"), a("autocomplete", "off"), a("data-testid", "display-timezone-input")),
		el("datalist", []attr{a("id", "tz-list")}, each(v.Timezones, func(tz string) node { return el("option", []attr{a("value", tz)}) })),
		el("div", []attr{a("class", "help")}, text("Timestamps in delivered notifications render in this zone. Blank uses each check's own timezone (itself UTC unless set). Pages in this UI always follow your browser instead.")))
	return el("div", []attr{a("class", "card")},
		el("div", []attr{a("class", "ch")}, el("h2", nil, text("Settings"))),
		el("div", []attr{a("class", "cb")},
			when(v.SettingsError != "", el("p", []attr{a("class", "flash err")}, text(v.SettingsError))),
			when(v.SettingsFlash != "", el("p", []attr{a("class", "flash ok"), a("data-testid", "settings-flash")}, text(v.SettingsFlash))),
			el("form", []attr{a("method", "post"), a("action", "/admin/settings")},
				CSRFField(v.CSRF),
				// Not for the retention fields below: those are plain day counts.
				el("datalist", []attr{a("id", "dur-list")}, durations),
				el("div", []attr{a("class", "two")},
					settingField("scan_interval", "Global scan interval", v.ScanInterval, durationHelp, durationAttrs...),
					settingField("nag_interval", "Global nag interval", v.NagInterval, "Leave blank to turn off nagging. "+durationHelp, durationAttrs...)),
				el("div", []attr{a("class", "two")},
					settingField("pings_retention_days", "Pings retention (days)", v.PingsRetentionDays, "Leave blank to keep forever."),
					settingField("notifications_retention_days", "Notifications retention (days)", v.NotificationsRetentionDays, "Leave blank to keep forever.")),
				el("div", []attr{a("class", "two")},
					settingField("audit_retention_days", "Audit trail retention (days)", v.AuditRetentionDays,
						"Blank keeps every entry forever — nothing else trims this table, so it grows for the life of the instance. Shortening it deletes history from the audit trail below, and the change is itself recorded there.",
						a("data-testid", "audit-retention-input")),
					timezone),
				el("div", []attr{a("class", "btnrow")}, el("button", []attr{a("class", "btn primary"), a("type", "submit")}, text("Save changes"))))))
}

func environmentCard(v AdminView) node {
	groups := each(v.Env, func(g EnvGroup) node {
		return group(
			el("h2", []attr{a("class", "subhead")}, text(g.Title)),
			plainTable("", "envtable", headRow("Variable", "Current", "Default", "Description"), each(g.Rows, envRow)))
	})
	return el("div", []attr{a("class", "card")},
		el("div", []attr{a("class", "ch")}, el("h2", nil, text("Environment"))),
		el("div", []attr{a("class", "cb")},
			el("p", []attr{a("class", "crumb tight")}, text("Configured via environment variables — changing these requires restarting the server.")),
			groups))
}

func envValue(row EnvSetting) node {
	switch row.Kind {
	case EnvText:
		return el("code", nil, text(row.Value))
	case EnvSecretSet:
		return el("span", []attr{a("class", "pill ok")}, text("configured"))
	default:
		return el("span", []attr{a("class", "note")}, text("not set"))
	}
}

func envRow(row EnvSetting) node {
	return el("tr", []attr{a("data-testid", "env-row-"+row.Var)},
		el("td", []attr{a("class", "mono")}, text(row.Var)),
		el("td", []attr{attrIf(row.Var == "PINGWARD_SMTP_PASSWORD", "data-testid", "env-smtp-password")}, envValue(row)),
		el("td", []attr{a("class", "mono")}, text(row.Default)),
		el("td", nil, text(row.Description)))
}

func elevationCard(v AdminView) node {
	state := group(
		el("p", []attr{a("class", "crumb tight"), a("data-testid", "elevation-state")}, text("Creating a user, resetting a password and granting admin each hand out access that outlasts this browser session, so they ask you to confirm your password first. Removing access — disabling, demoting, deleting — never does.")),
		el("p", nil, el("a", []attr{a("class", "btn"), a("href", "/admin/unlock"), a("data-testid", "elevation-confirm-link")}, text("Confirm now"))))
	if v.ElevationRemaining != "" {
		state = el("p", []attr{a("class", "note"), a("data-testid", "elevation-state")},
			text("Confirmed — creating a user, resetting a password and granting admin stay available for another "+v.ElevationRemaining+"."))
	}
	return el("div", []attr{a("class", "card")},
		el("div", []attr{a("class", "ch")}, el("h2", nil, text("Admin actions"))),
		el("div", []attr{a("class", "cb")},
			when(v.ElevationFlash != "", el("p", []attr{a("class", "flash ok"), a("data-testid", "elevation-flash")}, text(v.ElevationFlash))),
			state))
}

func usersCard(v AdminView) node {
	return el("div", []attr{a("class", "card")},
		el("div", []attr{a("class", "ch")}, el("h2", nil, text("All users"))),
		el("div", []attr{a("class", "cb")},
			when(v.UserFlash != "", el("p", []attr{a("class", "flash err"), a("data-testid", "users-flash")}, text(v.UserFlash))),
			when(v.PasswordResetFlash != "", el("p", []attr{a("class", "flash warn"), a("data-testid", "password-reset-flash")}, text(v.PasswordResetFlash))),
			// .tscroll keeps the Add-user form below from scrolling with the table.
			plainTable("", "", headRow("Username", "Admin", ""), each(v.Users, func(u AdminUserRow) node { return userRow(u, v) })),
			el("h2", []attr{a("class", "subhead")}, text("Add user")),
			when(v.UserError != "", el("p", []attr{a("class", "flash err"), a("data-testid", "user-error")}, text(v.UserError))),
			addUserForm(v)))
}

func userRow(u AdminUserRow, v AdminView) node {
	toggleAdminLabel, toggleDisabledLabel := "Make admin", "Disable"
	if u.IsAdmin {
		toggleAdminLabel = "Revoke admin"
	}
	if u.Disabled {
		toggleDisabledLabel = "Enable"
	}
	rolePill := el("span", []attr{a("class", "pill start"), a("data-testid", "user-role")}, text("member"))
	if u.IsAdmin {
		rolePill = el("span", []attr{a("class", "pill ok"), a("data-testid", "user-role")}, text("admin"))
	}
	return el("tr", []attr{a("data-testid", "user-row-"+u.Username)},
		el("td", nil, text(u.Username)),
		el("td", nil, rolePill,
			when(u.Disabled, group(text(" "), el("span", []attr{a("class", "pill fail"), a("data-testid", "user-disabled")}, text("disabled"))))),
		el("td", nil,
			el("form", []attr{a("class", "inline"), a("method", "post"), a("action", "/admin/users/"+num(u.ID)+"/password"), attrIf(v.ElevationLocked, "data-reauth", "reset this user's password")},
				CSRFField(v.CSRF),
				void("input", a("type", "password"), a("name", "password"), a("placeholder", "new password"), a("autocomplete", "new-password"), a("data-testid", "user-reset-input"), flag("required")),
				el("button", []attr{a("class", "btn"), a("type", "submit"), a("data-testid", "user-reset-submit")}, text("Reset"))),
			userControls(u, v, toggleAdminLabel, toggleDisabledLabel)))
}

func userControls(u AdminUserRow, v AdminView, toggleAdminLabel, toggleDisabledLabel string) node {
	if u.IsSelf {
		inert := func(class, title, testID, label string) node {
			return el("span", []attr{a("class", class), a("title", title), a("data-testid", testID)}, text(label))
		}
		return group(
			inert("btn disabled", "You cannot change your own admin status", "user-toggle-admin", toggleAdminLabel),
			inert("btn disabled", "You cannot disable your own account", "user-toggle-disabled", toggleDisabledLabel),
			inert("btn danger disabled", "You cannot delete your own account", "user-delete", "Delete"))
	}
	// Granting admin is gated on elevation; revoking is not, so it confirms instead.
	adminForm := []attr{a("class", "inline"), a("method", "post"), a("action", "/admin/users/"+num(u.ID)+"/admin"),
		attrIf(u.IsAdmin, "data-confirm", "Revoke admin rights from this user?"),
		attrIf(!u.IsAdmin && v.ElevationLocked, "data-reauth", "grant admin rights")}
	disabledForm := []attr{a("class", "inline"), a("method", "post"), a("action", "/admin/users/"+num(u.ID)+"/disabled"),
		attrIf(!u.Disabled, "data-confirm", "Disable this user? They will not be able to sign in.")}
	deleteForm := []attr{a("class", "inline"), a("method", "post"), a("action", "/admin/users/"+num(u.ID)+"/delete"),
		a("data-confirm", "Delete this user? This cannot be undone.")}
	button := func(class, testID, label string) node {
		return el("button", []attr{a("class", class), a("type", "submit"), a("data-testid", testID)}, text(label))
	}
	return group(
		el("form", adminForm, CSRFField(v.CSRF), button("btn", "user-toggle-admin", toggleAdminLabel)),
		el("form", disabledForm, CSRFField(v.CSRF), button("btn", "user-toggle-disabled", toggleDisabledLabel)),
		el("form", deleteForm, CSRFField(v.CSRF), button("btn danger", "user-delete", "Delete")))
}

func addUserForm(v AdminView) node {
	return el("form", []attr{a("method", "post"), a("action", "/admin/users"), attrIf(v.ElevationLocked, "data-reauth", "create this user")},
		CSRFField(v.CSRF),
		el("div", []attr{a("class", "two")},
			el("div", []attr{a("class", "field")},
				el("label", []attr{a("for", "username")}, text("Username")),
				void("input", a("id", "username"), a("name", "username"), a("autocomplete", "off"), a("data-testid", "user-username-input"), flag("required"))),
			el("div", []attr{a("class", "field")},
				el("label", []attr{a("for", "password")}, text("Password")),
				void("input", a("id", "password"), a("name", "password"), a("type", "password"), a("autocomplete", "new-password"), a("data-testid", "user-password-input"), flag("required")))),
		el("div", []attr{a("class", "two")}, el("div", []attr{a("class", "field")},
			el("label", nil, text("Role")),
			el("label", []attr{a("class", "chk")},
				void("input", a("type", "checkbox"), a("name", "is_admin"), a("value", "1"), a("data-testid", "user-admin-checkbox")),
				el("span", []attr{a("class", "nm")}, text("Admin"))))),
		el("div", []attr{a("class", "formactions")},
			el("button", []attr{a("class", "btn primary"), a("type", "submit"), a("data-testid", "user-submit")}, text("Create user"))))
}

func projectsGroup(v AdminView) node {
	rows := each(v.Projects, func(p AdminProjectRow) node {
		href := "/admin/projects/" + num(p.ID)
		return el("div", []attr{a("class", "check"), a("data-href", href)},
			el("div", []attr{a("class", "cmeta")},
				el("div", []attr{a("class", "nm")}, el("a", []attr{a("href", href)}, text(p.Name))),
				el("div", []attr{a("class", "sc")}, text("owner: "+p.Owner))),
			el("a", []attr{a("class", "rowedit"), a("href", href)}, text("Manage →")))
	})
	return el("div", []attr{a("class", "group")},
		el("div", []attr{a("class", "gh")},
			el("h2", nil, text("All projects")),
			el("span", []attr{a("class", "count")}, text(num(len(v.Projects))+" total")),
			el("span", []attr{a("class", "line")})),
		el("p", []attr{a("class", "note mb-12")}, text("Every project across all users. Looking is not recorded; changes you make, and revealing a check's ping URL, are — see the audit trail below.")),
		el("div", []attr{a("class", "list")},
			when(len(v.Projects) == 0, el("p", []attr{a("class", "empty")}, text("No projects yet."))),
			rows))
}
