package views

import (
	"time"

	"github.com/henry40408/pingward/internal/view"
)

// AccountView feeds the account page.
type AccountView struct {
	CSRF    string
	IsAdmin bool
	// Sessions section.
	Sessions []SessionRow
	// OtherCount hides the "revoke others" control when nothing else is left to revoke.
	OtherCount int
	// HasPassword is false for a passwordless forward-auth account: it hides the
	// password card and the API-key re-auth field.
	HasPassword bool
	// Password section.
	PasswordError string
	PasswordFlash string
	// API keys section.
	Keys []APIKeyRow
	// NewToken is rendered once, right after creation, and never recoverable after.
	NewToken string
	KeyError string
}

// SessionRow is a stored session minus its raw id, plus the handle that names it.
type SessionRow struct {
	Handle     string
	CreatedAt  *time.Time
	LastSeenAt *time.Time
	ExpiresAt  time.Time
	UserAgent  *string
	IP         *string
	Current    bool
	SSO        bool
}

// APIKeyRow keeps an expired key listed so it can still be revoked.
type APIKeyRow struct {
	ID         int64
	Name       string
	Prefix     string
	CreatedAt  time.Time
	LastUsedAt *time.Time
	ExpiresAt  *time.Time
	Expired    bool
}

func localTime(at time.Time) node {
	return el("span", []attr{a("class", "localtime"), a("data-ts", view.RFC3339(at))}, text(view.FmtUTC(at)))
}

func optionalTime(at *time.Time, none string) node {
	if at == nil {
		return text(none)
	}
	return localTime(*at)
}

func optionalText(s *string) node {
	if s == nil {
		return text("—")
	}
	return text(*s)
}

func Account(v AccountView) string {
	body := group(
		el("div", []attr{a("class", "phead")}, el("div", nil,
			el("h1", nil, text("Account")),
			el("p", []attr{a("class", "crumb tight")}, text("Manage the browser sessions signed in as you and the API keys used for programmatic access.")))),
		when(v.NewToken != "", newTokenCard(v.NewToken)),
		when(v.PasswordFlash != "", el("div", []attr{a("class", "card")}, el("div", []attr{a("class", "cb")},
			el("p", []attr{a("class", "flash ok"), a("data-testid", "password-changed-flash")}, text(v.PasswordFlash))))),
		when(v.HasPassword, passwordCard(v)),
		sessionsCard(v),
		when(v.OtherCount > 0, revokeOthersCard(v.CSRF)),
		keysCard(v),
		createKeyCard(v),
	)
	return Page(navChrome(v.IsAdmin, v.CSRF), body)
}

func newTokenCard(token string) node {
	return el("div", []attr{a("class", "card accent")}, el("div", []attr{a("class", "cb")},
		el("p", []attr{a("class", "flash ok"), a("data-testid", "api-key-created")}, text("Key created — copy it now. For security it won't be shown again.")),
		el("div", []attr{a("class", "urlrow")},
			el("code", []attr{a("data-testid", "api-key-token")}, text(token)),
			el("button", []attr{a("class", "copy"), a("type", "button"), a("data-copy", token)}, text("Copy")))))
}

func revokeOthersCard(csrf string) node {
	return el("div", []attr{a("class", "card")}, el("div", []attr{a("class", "cb")},
		el("form", []attr{a("method", "post"), a("action", "/account/sessions/revoke-others"), a("data-confirm", "Revoke every other session? Only this device will remain signed in.")},
			CSRFField(csrf),
			el("button", []attr{a("class", "btn danger"), a("type", "submit"), a("data-testid", "session-revoke-others")}, text("Revoke all other sessions")))))
}

func passwordField(id, name, label, autocomplete, testID string) node {
	return el("div", []attr{a("class", "field")},
		el("label", []attr{a("for", id)}, text(label)),
		void("input", a("id", id), a("name", name), a("type", "password"), a("autocomplete", autocomplete), a("data-testid", testID), flag("required")))
}

func passwordCard(v AccountView) node {
	return el("div", []attr{a("class", "card")},
		el("div", []attr{a("class", "ch")}, el("h2", nil, text("Change password"))),
		el("div", []attr{a("class", "cb")},
			when(v.PasswordError != "", el("p", []attr{a("class", "flash err"), a("data-testid", "password-error")}, text(v.PasswordError))),
			el("p", []attr{a("class", "crumb tight")}, text("Changing it signs out every other session listed below. API keys keep working — revoke those separately.")),
			el("form", []attr{a("method", "post"), a("action", "/account/password")},
				CSRFField(v.CSRF),
				passwordField("current_password", "current_password", "Current password", "current-password", "current-password-input"),
				el("div", []attr{a("class", "two")},
					passwordField("new_password", "new_password", "New password", "new-password", "new-password-input"),
					passwordField("confirm_password", "confirm_password", "Confirm new password", "new-password", "confirm-password-input")),
				el("div", []attr{a("class", "formactions")},
					el("button", []attr{a("class", "btn primary"), a("type", "submit"), a("data-testid", "password-submit")}, text("Change password"))))))
}

func tableCard(heading string, empty node, table node) node {
	content := table
	if empty != "" {
		content = empty
	}
	return el("div", []attr{a("class", "card")}, el("div", []attr{a("class", "ch")}, el("h2", nil, text(heading))), content)
}

func headRow(titles ...string) node {
	cells := make([]node, 0, len(titles))
	for _, title := range titles {
		cells = append(cells, el("th", nil, text(title)))
	}
	return el("thead", nil, el("tr", nil, cells...))
}

func scrollTable(head node, rows node) node {
	return el("div", []attr{a("class", "cb")}, el("div", []attr{a("class", "tscroll")},
		el("table", nil, head, el("tbody", nil, rows))))
}

func sessionsCard(v AccountView) node {
	if len(v.Sessions) == 0 {
		return tableCard("Your sessions", el("p", []attr{a("class", "empty"), a("data-testid", "sessions-empty")}, text("No active sessions.")), "")
	}
	rows := each(v.Sessions, func(s SessionRow) node { return sessionRow(s, v.CSRF) })
	return tableCard("Your sessions", "", scrollTable(headRow("Started", "Last seen", "IP", "Browser", "Idle expiry", ""), rows))
}

func sessionRow(s SessionRow, csrf string) node {
	mono := []attr{a("class", "mono")}
	return el("tr", []attr{a("data-testid", "session-row-"+s.Handle)},
		el("td", mono, optionalTime(s.CreatedAt, "—")),
		el("td", mono, optionalTime(s.LastSeenAt, "never")),
		el("td", mono, optionalText(s.IP)),
		el("td", nil,
			optionalText(s.UserAgent),
			when(s.Current, group(text(" "), el("span", []attr{a("class", "pill ok"), a("data-testid", "session-current")}, text("this device")))),
			when(s.SSO, group(text(" "), el("span", []attr{a("class", "pill log"), a("data-testid", "session-sso")}, text("SSO"))))),
		el("td", mono, localTime(s.ExpiresAt)),
		el("td", nil, el("form", []attr{a("class", "inline"), a("method", "post"), a("action", "/account/sessions/"+s.Handle+"/revoke"), a("data-confirm", "Revoke this session? If it is your current session you will be signed out.")},
			CSRFField(csrf),
			el("button", []attr{a("class", "btn danger"), a("type", "submit"), a("data-testid", "session-revoke")}, text("revoke")))))
}

func keysCard(v AccountView) node {
	intro := el("div", []attr{a("class", "cb")}, el("p", []attr{a("class", "crumb tight")},
		text("Authenticate programmatic requests with "), el("code", nil, text("Authorization: Bearer <token>")), text(". "),
		el("a", []attr{a("href", "/api/docs"), a("data-testid", "api-docs-link")}, text("API reference")), text(" · "),
		el("a", []attr{a("href", "/api/openapi.json"), a("data-testid", "api-openapi-link")}, text("OpenAPI spec")), text(".")))
	list := scrollTable(headRow("Name", "Prefix", "Created", "Last used", "Expires", ""),
		each(v.Keys, func(k APIKeyRow) node { return keyRow(k, v.CSRF) }))
	if len(v.Keys) == 0 {
		list = el("p", []attr{a("class", "empty"), a("data-testid", "api-keys-empty")}, text("No API keys yet — create one below."))
	}
	return el("div", []attr{a("class", "card")}, el("div", []attr{a("class", "ch")}, el("h2", nil, text("Your keys"))), intro, list)
}

func keyRow(k APIKeyRow, csrf string) node {
	mono := []attr{a("class", "mono")}
	expires := text("never")
	if k.ExpiresAt != nil {
		expires = group(localTime(*k.ExpiresAt),
			when(k.Expired, group(text(" "), el("span", []attr{a("class", "pill fail"), a("data-testid", "api-key-expired-"+num(k.ID))}, text("expired")))))
	}
	return el("tr", []attr{a("data-testid", "api-key-row-"+num(k.ID))},
		el("td", nil, text(k.Name)),
		el("td", mono, text(k.Prefix+"…")),
		el("td", mono, localTime(k.CreatedAt)),
		el("td", mono, optionalTime(k.LastUsedAt, "never")),
		el("td", mono, expires),
		el("td", nil, el("form", []attr{a("class", "inline"), a("method", "post"), a("action", "/account/api-keys/"+num(k.ID)+"/delete"), a("data-confirm", "Revoke this API key?")},
			CSRFField(csrf),
			el("button", []attr{a("class", "btn danger"), a("type", "submit"), a("data-testid", "api-key-delete")}, text("revoke")))))
}

func createKeyCard(v AccountView) node {
	suggestions := each(view.ExpirySuggestions(), func(e string) node { return el("option", []attr{a("value", e)}) })
	passwordInput := el("div", []attr{a("class", "field")},
		el("label", []attr{a("for", "key_current_password")}, text("Your password")),
		void("input", a("id", "key_current_password"), a("name", "current_password"), a("type", "password"), a("autocomplete", "current-password"), a("data-testid", "api-key-password-input"), flag("required")),
		el("p", []attr{a("class", "crumb tight")}, text("A key keeps working after you sign out or change your password, so it is asked for again here.")))
	return el("div", []attr{a("class", "card flush")},
		el("div", []attr{a("class", "ch")}, el("h2", nil, text("Create API key"))),
		el("div", []attr{a("class", "cb")},
			when(v.KeyError != "", el("p", []attr{a("class", "flash err"), a("data-testid", "api-key-error")}, text(v.KeyError))),
			el("form", []attr{a("method", "post"), a("action", "/account/api-keys")},
				CSRFField(v.CSRF),
				el("div", []attr{a("class", "two")},
					el("div", []attr{a("class", "field")},
						el("label", []attr{a("for", "name")}, text("Name")),
						void("input", a("id", "name"), a("name", "name"), a("data-testid", "api-key-name-input"), a("placeholder", "e.g. CI deploy"), flag("required"))),
					el("div", []attr{a("class", "field")},
						el("label", []attr{a("for", "expires_in")}, text("Expires in (optional)")),
						// Blank means never, so the field is not required.
						void("input", a("id", "expires_in"), a("name", "expires_in"), a("list", "expiry-list"), a("autocomplete", "off"), a("data-testid", "api-key-expires-input"), a("placeholder", "30d — blank = never")),
						el("datalist", []attr{a("id", "expiry-list")}, suggestions))),
				when(v.HasPassword, passwordInput),
				el("div", []attr{a("class", "formactions")},
					el("button", []attr{a("class", "btn primary"), a("type", "submit"), a("data-testid", "api-key-submit")}, text("Create key"))))))
}
