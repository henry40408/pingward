package views

// AuthView feeds the login and setup pages.
type AuthView struct {
	CSRF  string
	Error string // empty = none
}

func authChrome(view AuthView) Chrome { return Chrome{CSRF: view.CSRF} }

func brand() node {
	return el("a", []attr{a("class", "brand"), a("href", "/")},
		el("span", []attr{a("class", "glyph")}, text("▚")), text("pingward"))
}

func authError(testID, message string) node {
	return when(message != "", el("p", []attr{a("class", "flash err"), a("data-testid", testID)}, text(message)))
}

func Login(view AuthView) string {
	body := el("div", []attr{a("class", "auth")}, el("div", []attr{a("class", "card")}, el("div", []attr{a("class", "cb")},
		brand(),
		el("h1", nil, text("Log in")),
		authError("login-error", view.Error),
		el("form", []attr{a("method", "post"), a("action", "/login")},
			CSRFField(view.CSRF),
			el("div", []attr{a("class", "field")},
				el("label", []attr{a("for", "username")}, text("Username")),
				void("input", a("id", "username"), a("name", "username"), a("autocomplete", "username"), a("data-testid", "username-input"), flag("required"))),
			el("div", []attr{a("class", "field")},
				el("label", []attr{a("for", "password")}, text("Password")),
				void("input", a("id", "password"), a("name", "password"), a("type", "password"), a("autocomplete", "current-password"), a("data-testid", "password-input"), flag("required"))),
			el("button", []attr{a("class", "btn primary block"), a("type", "submit"), a("data-testid", "login-submit")}, text("Log in")),
		))))
	return Page(authChrome(view), body)
}

func Setup(view AuthView) string {
	body := el("div", []attr{a("class", "auth")}, el("div", []attr{a("class", "card")}, el("div", []attr{a("class", "cb")},
		brand(),
		el("h1", nil, text("Create the first admin")),
		authError("setup-error", view.Error),
		el("form", []attr{a("method", "post"), a("action", "/setup")},
			CSRFField(view.CSRF),
			el("div", []attr{a("class", "field")},
				el("label", []attr{a("for", "username")}, text("Username")),
				void("input", a("id", "username"), a("name", "username"), a("autocomplete", "username"), a("data-testid", "username-input"), flag("required"))),
			el("div", []attr{a("class", "field")},
				el("label", []attr{a("for", "password")}, text("Password")),
				void("input", a("id", "password"), a("name", "password"), a("type", "password"), a("autocomplete", "new-password"), a("data-testid", "password-input"), flag("required"))),
			el("div", []attr{a("class", "field")},
				el("label", []attr{a("for", "setup_token")}, text("Setup token")),
				void("input", a("id", "setup_token"), a("name", "setup_token"), a("autocomplete", "off"), a("autocapitalize", "characters"), a("spellcheck", "false"), a("placeholder", "XXXXX-XXXXX"), a("data-testid", "setup-token-input"), flag("required")),
				el("div", []attr{a("class", "help")}, text("Printed in the server log (stderr) when pingward started with no users. Look for “Setup token”."))),
			el("button", []attr{a("class", "btn primary block"), a("type", "submit"), a("data-testid", "setup-submit")}, text("Create admin")),
		))))
	return Page(authChrome(view), body)
}
