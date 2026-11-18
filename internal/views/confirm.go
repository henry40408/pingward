package views

// ConfirmView feeds the no-script stand-in for app.js's confirm().
type ConfirmView struct {
	IsAdmin bool
	CSRF    string
	Title   string
	Message string
	Button  string
	Action  string
	Cancel  string
}

// Confirm is where a destructive POST without ?confirmed=1 lands; its form
// re-posts the action with the flag.
func Confirm(view ConfirmView) string {
	body := group(
		el("div", []attr{a("class", "phead")}, el("div", nil,
			el("h1", nil, text(view.Title)),
			el("p", []attr{a("class", "crumb tight"), a("data-testid", "confirm-why")}, text("Nothing has happened yet. This page is standing in for the confirmation prompt your browser would normally show, so the action is not carried out until you say so here.")))),
		el("div", []attr{a("class", "card")}, el("div", []attr{a("class", "cb")},
			el("p", []attr{a("class", "crumb tight"), a("data-testid", "confirm-message")}, text(view.Message)),
			el("form", []attr{a("method", "post"), a("action", view.Action)},
				CSRFField(view.CSRF),
				el("div", []attr{a("class", "formactions")},
					el("button", []attr{a("class", "btn danger"), a("type", "submit"), a("data-testid", "confirm-submit")}, text(view.Button)),
					el("a", []attr{a("class", "btn"), a("href", view.Cancel), a("data-testid", "confirm-cancel")}, text("Cancel")))))),
	)
	return Page(Chrome{ShowNav: true, IsAdmin: view.IsAdmin, CSRF: view.CSRF}, body)
}
