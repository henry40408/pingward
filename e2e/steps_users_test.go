package e2e

import (
	"errors"
	"fmt"
	"net/url"
	"slices"
	"strings"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

const deleteUserConfirmation = "Delete this user? This cannot be undone."

// userRow is a user's <tr>. Row-local test ids stay unambiguous even when a
// username matches another row's pill text ("member").
func (w *world) userRow(username string) (*harness.Element, error) {
	return w.browser.TestID("user-row-" + username)
}

// rowTestID waits for the descendant of an element that carries the
// data-testid.
func rowTestID(row *harness.Element, id string) (*harness.Element, error) {
	var found *harness.Element
	err := harness.Eventually("a descendant with testid `"+id+"`", func() (bool, error) {
		element, err := row.FindOptional(`[data-testid="` + id + `"]`)
		found = element
		return element != nil, err
	})
	return found, err
}

// submitSelfRowAction posts a row action the page renders as an inert <span>,
// to prove the server-side self-guard. The base path comes off the row's
// password-reset form, which is always live.
func (w *world) submitSelfRowAction(row *harness.Element, action string) error {
	form, err := row.FindOptional(`form[action$="/password"]`)
	if err != nil {
		return err
	}
	if form == nil {
		return errors.New("the row has no password-reset form to read")
	}
	reset, _, err := form.Attr("action")
	if err != nil {
		return err
	}
	field, err := w.browser.Optional(`input[name="_csrf"]`)
	if err != nil {
		return err
	}
	if field == nil {
		return errors.New("the page renders no CSRF field")
	}
	csrf, err := field.Value()
	if err != nil {
		return err
	}
	base := strings.TrimSuffix(reset, "/password")
	// Assert the 303: a csrf_guard 403 would also leave state unchanged.
	status, err := w.postFormAsUser(base+"/"+action, url.Values{"_csrf": {csrf}})
	if err != nil {
		return err
	}
	if status != 303 {
		return fmt.Errorf("the self-targeted %s answered %d", action, status)
	}
	return w.goTo("/admin")
}

// submitRowAction drives a row's mutating control and waits for the document
// to be replaced: the redirect returns to /admin, so a URL check would pass on
// the stale DOM.
func (w *world) submitRowAction(row, control *harness.Element, action string) error {
	tag, err := control.TagName()
	if err != nil {
		return err
	}
	if tag == "SPAN" {
		return w.submitSelfRowAction(row, action)
	}
	// Only some controls confirm (delete, revoke admin, disable), so accept
	// whichever prompt comes.
	w.browser.SetDialogPolicy(harness.AcceptDialogs)
	defer w.browser.SetDialogPolicy(harness.DismissDialogs)
	return w.browser.SubmitElementWhenReady(control)
}

func (w *world) rowAction(username, controlID, action string) error {
	row, err := w.userRow(username)
	if err != nil {
		return err
	}
	control, err := rowTestID(row, controlID)
	if err != nil {
		return err
	}
	if err := w.submitRowAction(row, control, action); err != nil {
		return err
	}
	return w.expectPath("/admin")
}

// addUser submits "Add user" and waits for the new row.
func (w *world) addUser(username, password string, admin bool) error {
	b := w.browser
	if err := b.Fill("user-username-input", username); err != nil {
		return err
	}
	if err := b.Fill("user-password-input", password); err != nil {
		return err
	}
	if admin {
		if err := b.Check("user-admin-checkbox"); err != nil {
			return err
		}
	}
	if err := b.Submit("user-submit"); err != nil {
		return err
	}
	if err := w.expectPath("/admin"); err != nil {
		return err
	}
	return b.ExpectVisible("user-row-" + username)
}

func (w *world) resetPassword(username, password string) error {
	// A refusal re-renders /admin, so the document is replaced either way.
	row, err := w.userRow(username)
	if err != nil {
		return err
	}
	field, err := rowTestID(row, "user-reset-input")
	if err != nil {
		return err
	}
	if err := field.Fill(password); err != nil {
		return err
	}
	submit, err := rowTestID(row, "user-reset-submit")
	if err != nil {
		return err
	}
	return w.browser.SubmitElementWhenReady(submit)
}

func (w *world) dismissDeleteConfirmation(username string) error {
	row, err := w.userRow(username)
	if err != nil {
		return err
	}
	control, err := rowTestID(row, "user-delete")
	if err != nil {
		return err
	}
	w.browser.SetDialogPolicy(harness.DismissDialogs)
	seen := len(w.browser.Dialogs())
	if err := control.ClickWhenReady(); err != nil {
		return err
	}
	// The message is read first: a missing dialog would leave the row too.
	if err := harness.Eventually("a confirm() prompt", func() (bool, error) {
		return len(w.browser.Dialogs()) > seen, nil
	}); err != nil {
		return err
	}
	dialogs := w.browser.Dialogs()
	if message := dialogs[len(dialogs)-1]; message != deleteUserConfirmation {
		return fmt.Errorf("the confirmation read %q", message)
	}
	return nil
}

var selfRowControls = map[string]string{
	"demote":  "user-toggle-admin",
	"disable": "user-toggle-disabled",
	"delete":  "user-delete",
}

func (w *world) ownRowControlInert(action string) error {
	controlID, known := selfRowControls[action]
	if !known {
		return fmt.Errorf("no self-row control is called `%s`", action)
	}
	// The Background signs in as "admin".
	row, err := w.userRow("admin")
	if err != nil {
		return err
	}
	control, err := rowTestID(row, controlID)
	if err != nil {
		return err
	}
	tag, err := control.TagName()
	if err != nil {
		return err
	}
	if tag != "SPAN" {
		return fmt.Errorf("the %s control renders as <%s>", action, tag)
	}
	classes, err := control.Classes()
	if err != nil {
		return err
	}
	if !slices.Contains(classes, "disabled") {
		return fmt.Errorf("the %s control's classes are %q", action, strings.Join(classes, " "))
	}
	return nil
}

func (w *world) ownRowResetUsable() error {
	row, err := w.userRow("admin")
	if err != nil {
		return err
	}
	control, err := rowTestID(row, "user-reset-submit")
	if err != nil {
		return err
	}
	tag, err := control.TagName()
	if err != nil {
		return err
	}
	if tag != "BUTTON" {
		return fmt.Errorf("the reset control renders as <%s>", tag)
	}
	enabled, err := control.Enabled()
	if err != nil {
		return err
	}
	if !enabled {
		return errors.New("the reset control is disabled")
	}
	return nil
}

func (w *world) userListedWithRole(username, role string) error {
	row, err := w.userRow(username)
	if err != nil {
		return err
	}
	pill, err := rowTestID(row, "user-role")
	if err != nil {
		return err
	}
	rendered, err := pill.NormalizedText()
	if err != nil {
		return err
	}
	if rendered != role {
		return fmt.Errorf("%s is listed as %q", username, rendered)
	}
	return nil
}

// disabledPill is the row's "disabled" pill, nil when it carries none. The row
// must exist, or a deleted user would pass as re-enabled.
func (w *world) disabledPill(username string) (*harness.Element, error) {
	row, err := w.userRow(username)
	if err != nil {
		return nil, err
	}
	return row.FindOptional(`[data-testid="user-disabled"]`)
}

func (w *world) tryGrantAdmin(username string) error {
	// Locked: app.js opens the re-auth dialog instead of navigating.
	row, err := w.userRow(username)
	if err != nil {
		return err
	}
	control, err := rowTestID(row, "user-toggle-admin")
	if err != nil {
		return err
	}
	return control.ClickWhenReady()
}

// registerUsersSteps covers the /admin user table.
func registerUsersSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	sc.Step(`^I am on the users page$`, func() error { return w.goTo("/admin") })
	sc.Step(`^I add a user `+quoted+` with password `+quoted+`$`, func(username, password string) error {
		return w.addUser(unquote(username), unquote(password), false)
	})
	sc.Step(`^a member `+quoted+` with password `+quoted+` exists$`, func(username, password string) error {
		return w.addUser(unquote(username), unquote(password), false)
	})
	sc.Step(`^I add an admin user `+quoted+` with password `+quoted+`$`, func(username, password string) error {
		return w.addUser(unquote(username), unquote(password), true)
	})
	sc.Step(`^an admin user `+quoted+` with password `+quoted+` exists$`, func(username, password string) error {
		return w.addUser(unquote(username), unquote(password), true)
	})
	sc.Step(`^I toggle admin on `+quoted+`$`, func(username string) error {
		return w.rowAction(unquote(username), "user-toggle-admin", "admin")
	})
	// One toggle; direction comes from the row's current state.
	toggleDisabled := func(username string) error {
		return w.rowAction(unquote(username), "user-toggle-disabled", "disabled")
	}
	sc.Step(`^I disable `+quoted+`$`, toggleDisabled)
	sc.Step(`^I enable `+quoted+`$`, toggleDisabled)
	sc.Step(`^I delete the user `+quoted+`$`, func(username string) error {
		return w.rowAction(unquote(username), "user-delete", "delete")
	})
	sc.Step(`^I (?:try to )?reset `+quoted+`'s password to `+quoted+`$`, func(username, password string) error {
		return w.resetPassword(unquote(username), unquote(password))
	})
	sc.Step(`^I attempt to delete `+quoted+` but dismiss the confirmation$`, func(username string) error {
		return w.dismissDeleteConfirmation(unquote(username))
	})
	sc.Step(`^the (\S+) control on my own row is inert$`, w.ownRowControlInert)
	sc.Step(`^the password reset control on my own row is usable$`, w.ownRowResetUsable)
	sc.Step(`^the user `+quoted+` is listed with role `+quoted+`$`, func(username, role string) error {
		return w.userListedWithRole(unquote(username), unquote(role))
	})
	sc.Step(`^the user `+quoted+` is marked disabled$`, func(username string) error {
		pill, err := w.disabledPill(unquote(username))
		if err != nil {
			return err
		}
		if pill == nil {
			return fmt.Errorf("%s carries no disabled pill", unquote(username))
		}
		return nil
	})
	sc.Step(`^the user `+quoted+` is not marked disabled$`, func(username string) error {
		pill, err := w.disabledPill(unquote(username))
		if err != nil {
			return err
		}
		if pill != nil {
			return fmt.Errorf("%s is still marked disabled", unquote(username))
		}
		return nil
	})
	sc.Step(`^the user `+quoted+` is not listed$`, func(username string) error {
		return b().ExpectAbsent("user-row-" + unquote(username))
	})
	// Expects a refusal (re-render, no new row), unlike addUser.
	sc.Step(`^I try to add a user `+quoted+` with password `+quoted+`$`, func(username, password string) error {
		if err := b().Fill("user-username-input", unquote(username)); err != nil {
			return err
		}
		if err := b().Fill("user-password-input", unquote(password)); err != nil {
			return err
		}
		return b().Submit("user-submit")
	})
	sc.Step(`^the user form shows the error `+quoted+`$`, func(message string) error {
		return b().ExpectExactText("user-error", unquote(message))
	})
	sc.Step(`^I try to grant admin to `+quoted+`$`, func(username string) error {
		return w.tryGrantAdmin(unquote(username))
	})
}
