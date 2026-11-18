package e2e

import (
	"errors"
	"fmt"
	"strings"

	"github.com/cucumber/godog"
)

// signIn submits the /login form; callers assert where it lands. It signs out
// first if needed, since /login bounces an authenticated visitor to /.
func (w *world) signIn(username, password string) error {
	if err := w.goTo("/login"); err != nil {
		return err
	}
	current, err := w.path()
	if err != nil {
		return err
	}
	if !strings.HasSuffix(current, "/login") {
		if err := w.browser.Submit("logout-button"); err != nil {
			return err
		}
		if err := w.goTo("/login"); err != nil {
			return err
		}
	}
	if err := w.browser.Fill("username-input", username); err != nil {
		return err
	}
	if err := w.browser.Fill("password-input", password); err != nil {
		return err
	}
	return w.browser.Submit("login-submit")
}

// registerAuthSteps covers setup, sign-in and sign-out.
func registerAuthSteps(sc *godog.ScenarioContext, w *world) {
	sc.Step(`^I visit `+quoted+`$`, func(path string) error { return w.goTo(unquote(path)) })
	sc.Step(`^I am on the setup page$`, func() error {
		if err := w.expectPath("/setup"); err != nil {
			return err
		}
		return w.browser.ExpectVisible("setup-submit")
	})
	sc.Step(`^I create the admin `+quoted+` with password `+quoted+`$`, func(username, password string) error {
		for _, field := range [][2]string{
			{"username-input", unquote(username)},
			{"password-input", unquote(password)},
			{"setup-token-input", w.server.SetupToken},
		} {
			if err := w.browser.Fill(field[0], field[1]); err != nil {
				return err
			}
		}
		return w.browser.Submit("setup-submit")
	})
	sc.Step(`^I land on the dashboard signed in$`, func() error {
		if err := w.expectPath("/"); err != nil {
			return err
		}
		return w.browser.ExpectVisible("logout-button")
	})
	sc.Step(`^an admin `+quoted+` with password `+quoted+` exists$`, func(username, password string) error {
		return w.api().BootstrapAdmin(unquote(username), unquote(password))
	})
	sc.Step(`^I sign in as `+quoted+` with password `+quoted+`$`, func(username, password string) error {
		return w.signIn(unquote(username), unquote(password))
	})
	sc.Step(`^I am signed in as `+quoted+` with password `+quoted+`$`, func(username, password string) error {
		if err := w.signIn(unquote(username), unquote(password)); err != nil {
			return err
		}
		return w.expectPath("/")
	})
	sc.Step(`^the login page shows the error `+quoted+`$`, func(message string) error {
		if err := w.expectPath("/login"); err != nil {
			return err
		}
		return w.browser.ExpectText("login-error", unquote(message))
	})
	sc.Step(`^I sign out$`, func() error { return w.browser.Submit("logout-button") })
	sc.Step(`^I am on the login page$`, func() error {
		if err := w.expectPath("/login"); err != nil {
			return err
		}
		return w.browser.ExpectVisible("login-submit")
	})
}

func navTestID(label string) (string, error) {
	if label == "Admin" {
		return "nav-admin", nil
	}
	return "", fmt.Errorf("no nav link is labelled `%s`", label)
}

// registerAuthzSteps covers authorization boundaries: the admin nav,
// cross-user reads and the CSRF guard.
func registerAuthzSteps(sc *godog.ScenarioContext, w *world) {
	// Via the admin's "Add user" form, so the admin must be signed in.
	sc.Step(`^a non-admin user `+quoted+` with password `+quoted+` exists$`, func(username, password string) error {
		username = unquote(username)
		if err := w.goTo("/admin"); err != nil {
			return err
		}
		if err := w.browser.Fill("user-username-input", username); err != nil {
			return err
		}
		if err := w.browser.Fill("user-password-input", unquote(password)); err != nil {
			return err
		}
		if err := w.browser.Submit("user-submit"); err != nil {
			return err
		}
		if err := w.expectPath("/admin"); err != nil {
			return err
		}
		return w.browser.ExpectExactTextSomewhere(username)
	})
	sc.Step(`^the `+quoted+` nav link is visible$`, func(label string) error {
		id, err := navTestID(unquote(label))
		if err != nil {
			return err
		}
		return w.browser.ExpectVisible(id)
	})
	sc.Step(`^the `+quoted+` nav link is not visible$`, func(label string) error {
		id, err := navTestID(unquote(label))
		if err != nil {
			return err
		}
		return w.browser.ExpectAbsent(id)
	})
	sc.Step(`^I navigate to `+quoted+`$`, func(path string) error {
		path = unquote(path)
		if err := w.goTo(path); err != nil {
			return err
		}
		var err error
		w.status, err = w.fetchStatus("GET", path)
		return err
	})
	sc.Step(`^the response status is (\d+)$`, func(status int) error {
		if w.status == 0 {
			return errors.New("no step recorded a response status")
		}
		if w.status != status {
			return fmt.Errorf("the response was %d, not %d", w.status, status)
		}
		return nil
	})
	// A page fetch with the session cookie, missing only the token; the
	// scenario asserts a live session first so the 403 is attributable.
	sc.Step(`^I POST to `+quoted+` without a CSRF token$`, func(path string) error {
		var err error
		w.status, err = w.fetchStatus("POST", unquote(path))
		return err
	})
	sc.Step(`^I remember the current project$`, func() error {
		// "I create a project named" does not await the redirect.
		if err := w.expectPathMatching(`/projects/\d+$`); err != nil {
			return err
		}
		var err error
		w.projectURL, err = w.path()
		return err
	})
	// Positive control, so the later 404 is the ownership guard, not the route.
	sc.Step(`^the owner can read the remembered project$`, func() error {
		project, err := w.requireProject()
		if err != nil {
			return err
		}
		if err := w.goTo(project); err != nil {
			return err
		}
		status, err := w.fetchStatus("GET", project)
		if err != nil {
			return err
		}
		if status != 200 {
			return fmt.Errorf("the owner's own project answered %d", status)
		}
		return nil
	})
	// owned_project answers 404 to a non-owner.
	sc.Step(`^I revisit it as `+quoted+` with password `+quoted+`$`, func(username, password string) error {
		if err := w.browser.Submit("logout-button"); err != nil {
			return err
		}
		if err := w.signIn(unquote(username), unquote(password)); err != nil {
			return err
		}
		if err := w.expectPath("/"); err != nil {
			return err
		}
		project, err := w.requireProject()
		if err != nil {
			return err
		}
		if err := w.goTo(project); err != nil {
			return err
		}
		w.status, err = w.fetchStatus("GET", project)
		return err
	})
}
