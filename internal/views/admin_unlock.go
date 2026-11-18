package views

// AdminUnlockView feeds the "confirm it's you" page.
type AdminUnlockView struct {
	IsAdmin bool
	CSRF    string
	// Applies is false for a passwordless forward-auth admin: the gate does not apply.
	Applies bool
	// Remaining is the readable duration when already confirmed, "" otherwise.
	Remaining string
	// TTL is the readable confirmation lifetime, taken from the elevation TTL so the copy cannot drift.
	TTL string
	// BouncedFlash is set when a refused action sent the admin here.
	BouncedFlash string
	Error        string
}

func AdminUnlock(v AdminUnlockView) string {
	var content node
	switch {
	case !v.Applies:
		content = unlockNotApplicable()
	case v.Remaining != "":
		content = unlockAlreadyConfirmed(v.Remaining)
	default:
		content = unlockPasswordForm(v)
	}
	body := group(
		el("div", []attr{a("class", "phead")}, el("div", nil,
			el("h1", nil, text("Confirm it's you")),
			el("p", []attr{a("class", "crumb tight")}, text("Some admin actions ask for your password a second time, even though you're already signed in.")))),
		when(v.BouncedFlash != "", el("p", []attr{a("class", "flash err"), a("data-testid", "unlock-bounced")}, text(v.BouncedFlash))),
		when(v.Error != "", el("p", []attr{a("class", "flash err"), a("data-testid", "unlock-error")}, text(v.Error))),
		content)
	return Page(navChrome(v.IsAdmin, v.CSRF), body)
}

func unlockNotApplicable() node {
	return el("div", []attr{a("class", "card")}, el("div", []attr{a("class", "cb")},
		el("p", []attr{a("class", "note"), a("data-testid", "unlock-not-applicable")}, text("Your account signs in through your reverse proxy or SSO provider, so there is no password here to confirm. Every admin action is already available to you.")),
		el("p", nil, el("a", []attr{a("class", "btn"), a("href", "/admin"), a("data-testid", "unlock-back")}, text("Back to admin")))))
}

func unlockAlreadyConfirmed(left string) node {
	return el("div", []attr{a("class", "card")}, el("div", []attr{a("class", "cb")},
		el("p", []attr{a("class", "note"), a("data-testid", "unlock-state")}, text("Already confirmed "), raw("&mdash;"), text(" these actions stay available for another "+left+".")),
		el("p", nil, el("a", []attr{a("class", "btn primary"), a("href", "/admin"), a("data-testid", "unlock-back")}, text("Back to admin")))))
}

func unlockPasswordForm(v AdminUnlockView) node {
	why := el("div", []attr{a("class", "card")},
		el("div", []attr{a("class", "ch")}, el("h2", nil, text("Why you're seeing this"))),
		el("div", []attr{a("class", "cb")},
			el("p", []attr{a("class", "crumb tight"), a("data-testid", "unlock-gated")},
				text("A session cookie proves who opened this browser, not who is at it now. "),
				el("strong", nil, text("Creating a user")), text(", "),
				el("strong", nil, text("resetting another user's password")), text(" and "),
				el("strong", nil, text("granting admin rights")),
				text(" each hand out access that keeps working after you sign out, so they ask for a second confirmation before they run.")),
			el("p", []attr{a("class", "crumb tight")},
				text("Actions that "), el("em", nil, text("remove")), text(" access "), raw("&mdash;"),
				text(" disabling, demoting, deleting "), raw("&mdash;"), text(" never ask, so you can always shut an account down in a hurry.")),
			el("p", []attr{a("class", "crumb tight")}, text("This is your same password again, not a second factor. Confirming lasts "+v.TTL+", and ends when you sign out."))))
	form := el("div", []attr{a("class", "card")},
		el("div", []attr{a("class", "ch")}, el("h2", nil, text("Confirm your password"))),
		el("div", []attr{a("class", "cb")}, el("form", []attr{a("method", "post"), a("action", "/admin/unlock")},
			CSRFField(v.CSRF),
			passwordField("password", "password", "Password", "current-password", "unlock-input"),
			el("div", []attr{a("class", "formactions")},
				el("button", []attr{a("class", "btn primary"), a("type", "submit"), a("data-testid", "unlock-submit")}, text("Confirm")),
				el("a", []attr{a("class", "btn"), a("href", "/admin"), a("data-testid", "unlock-cancel")}, text("Cancel"))))))
	return group(why, form)
}
