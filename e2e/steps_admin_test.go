package e2e

import (
	"errors"
	"fmt"
	"regexp"
	"strconv"
	"strings"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

// idFromPath reads the numeric id a URL path ends in.
func idFromPath(path, pattern string) (int64, error) {
	captures := regexp.MustCompile(pattern).FindStringSubmatch(path)
	if captures == nil {
		return 0, fmt.Errorf("`%s` does not match %s", path, pattern)
	}
	return strconv.ParseInt(captures[1], 10, 64)
}

func (w *world) createProjectAsMember(name string) error {
	if err := w.goTo("/projects/new"); err != nil {
		return err
	}
	if err := w.browser.Fill("project-name-input", name); err != nil {
		return err
	}
	if err := w.browser.Submit("project-submit"); err != nil {
		return err
	}
	if err := w.expectPathMatching(`/projects/\d+$`); err != nil {
		return err
	}
	return w.rememberID(`/projects/(\d+)$`, &w.projectID)
}

func (w *world) rememberID(pattern string, target *int64) error {
	path, err := w.path()
	if err != nil {
		return err
	}
	*target, err = idFromPath(path, pattern)
	return err
}

// createCheckAsMember runs from the project page createProjectAsMember leaves
// the browser on.
func (w *world) createCheckAsMember(name string, period int64) error {
	b := w.browser
	if err := b.Submit("new-check-link"); err != nil {
		return err
	}
	if err := b.Fill("check-name-input", name); err != nil {
		return err
	}
	if err := b.Fill("check-period-input", strconv.FormatInt(period, 10)); err != nil {
		return err
	}
	if err := b.Submit("check-submit"); err != nil {
		return err
	}
	if err := w.expectPathMatching(`/checks/\d+$`); err != nil {
		return err
	}
	return w.rememberID(`/checks/(\d+)$`, &w.checkID)
}

// memberOwnsProjectAndCheck seeds data owned by a non-admin so /admin/*
// exercises real cross-user access. signIn signs the admin out first.
func (w *world) memberOwnsProjectAndCheck(username, password, project, check string, period int64) error {
	if err := w.signIn(username, password); err != nil {
		return err
	}
	if err := w.expectPath("/"); err != nil {
		return err
	}
	if err := w.createProjectAsMember(project); err != nil {
		return err
	}
	return w.createCheckAsMember(check, period)
}

type subheadMeasure struct {
	Heading  *float64 `json:"heading"`
	Subheads []struct {
		Text string  `json:"text"`
		Size float64 `json:"size"`
	} `json:"subheads"`
}

// subheadsAreSubordinate compares computed sizes, so an inherited global h2
// size is caught. The 2px allowance admits the 14px .subhead over the 13px
// uppercase card label.
func (w *world) subheadsAreSubordinate() error {
	measured, err := harness.Eval[subheadMeasure](w.browser, `(() => {
		const size = (el) => parseFloat(getComputedStyle(el).fontSize);
		const heading = size(document.querySelector('.card .ch h2'));
		const subheads = [...document.querySelectorAll('.card .subhead')].map((el) => ({
			text: el.textContent.trim(), size: size(el),
		}));
		return { heading, subheads };
	})()`)
	if err != nil {
		return err
	}
	if measured.Heading == nil {
		return errors.New("no card heading was measured")
	}
	if len(measured.Subheads) == 0 {
		return errors.New("no .subhead rendered: the check would be vacuous")
	}
	for _, subhead := range measured.Subheads {
		if subhead.Size > *measured.Heading+2.0 {
			return fmt.Errorf("subheading %q renders at %vpx inside a %vpx card heading", subhead.Text, subhead.Size, *measured.Heading)
		}
	}
	return nil
}

func (w *world) adminProjectsListShows(project, owner string) error {
	b := w.browser
	if err := requireHeading(b, "All projects"); err != nil {
		return err
	}
	var text string
	err := harness.Eventually(fmt.Sprintf("`.check` containing %q", project), func() (bool, error) {
		row, err := b.WithTextContaining(".check", project)
		if err != nil || row == nil {
			return false, err
		}
		text, err = row.NormalizedText()
		return err == nil, nil
	})
	if err != nil {
		return err
	}
	if !strings.Contains(text, "owner: "+owner) {
		return fmt.Errorf("the row for %q reads %q", project, text)
	}
	return nil
}

func requireHeading(b *harness.Browser, name string) error {
	exists, err := b.HeadingExists(name)
	if err != nil {
		return err
	}
	if !exists {
		return fmt.Errorf("no heading reads %q", name)
	}
	return nil
}

func (w *world) memberProjectID() (int64, error) {
	if w.projectID == 0 {
		return 0, errors.New("no step recorded the member's project")
	}
	return w.projectID, nil
}

func (w *world) memberCheckID() (int64, error) {
	if w.checkID == 0 {
		return 0, errors.New("no step recorded the member's check")
	}
	return w.checkID, nil
}

// openAdminProjectSub opens a page under the member's project in the admin area.
func (w *world) openAdminProjectSub(suffix string) (int64, error) {
	id, err := w.memberProjectID()
	if err != nil {
		return 0, err
	}
	return id, w.goTo(fmt.Sprintf("/admin/projects/%d%s", id, suffix))
}

func (w *world) renameProject(name string) error {
	if _, err := w.openAdminProjectSub("/edit"); err != nil {
		return err
	}
	if err := w.browser.Fill("project-name-input", name); err != nil {
		return err
	}
	return w.browser.Submit("project-submit")
}

func (w *world) onAdminProjectPage(name string) error {
	id, err := w.memberProjectID()
	if err != nil {
		return err
	}
	if err := w.expectPath(fmt.Sprintf("/admin/projects/%d", id)); err != nil {
		return err
	}
	return requireHeading(w.browser, name)
}

// addWebhookChannel relies on webhook being the default kind, so only the name
// and URL need filling.
func (w *world) addWebhookChannel(name string) error {
	id, err := w.openAdminProjectSub("/channels/new")
	if err != nil {
		return err
	}
	b := w.browser
	if err := b.FillCSS("#name", name); err != nil {
		return err
	}
	if err := b.FillCSS("#webhook_url", "https://example.com/hook"); err != nil {
		return err
	}
	create, err := b.ButtonNamed("Create channel")
	if err != nil {
		return err
	}
	if create == nil {
		return errors.New("the channel form has no `Create channel` button")
	}
	if err := b.SubmitElementWhenReady(create); err != nil {
		return err
	}
	return w.expectPath(fmt.Sprintf("/admin/projects/%d", id))
}

func (w *world) channelListed(name string) error {
	return harness.Eventually(fmt.Sprintf("`.chk .nm` containing %q", name), func() (bool, error) {
		row, err := w.browser.WithTextContaining(".chk .nm", name)
		return row != nil, err
	})
}

func (w *world) pageHidesSMTPSecret() error {
	body, err := harness.Eval[string](w.browser, "document.body.textContent")
	if err != nil {
		return err
	}
	if strings.Contains(body, "e2e-secret-password") {
		return errors.New("the page prints the configured SMTP password")
	}
	return nil
}

func (w *world) auditRowTexts() ([]string, error) {
	rows, err := w.browser.TestIDs("audit-row")
	if err != nil {
		return nil, err
	}
	texts := make([]string, 0, len(rows))
	for _, row := range rows {
		text, err := row.NormalizedText()
		if err != nil {
			return nil, err
		}
		texts = append(texts, text)
	}
	return texts, nil
}

func (w *world) auditHasRows(count int) error {
	return harness.Eventually(fmt.Sprintf("the audit trail has %d row(s)", count), func() (bool, error) {
		rows, err := w.browser.TestIDs("audit-row")
		return len(rows) >= count, err
	})
}

func (w *world) auditShowsEntry(action string) error {
	return harness.Eventually(fmt.Sprintf("an `%s` audit entry", action), func() (bool, error) {
		texts, err := w.auditRowTexts()
		if err != nil {
			return false, err
		}
		for _, text := range texts {
			if strings.Contains(text, action) {
				return true, nil
			}
		}
		return false, nil
	})
}

func (w *world) filterAuditByAction(action string) error {
	if err := w.browser.SelectOption("audit-action", action); err != nil {
		return err
	}
	if err := w.browser.Click("audit-apply"); err != nil {
		return err
	}
	// Clear renders only once filtered, so this waits out the pre-filter rows.
	return w.browser.ExpectVisible("audit-clear")
}

// filterAuditByActor goes through the URL: the select lists only actors
// present, so an unmatched one cannot be picked.
func (w *world) filterAuditByActor(actor string) error {
	path, err := w.path()
	if err != nil {
		return err
	}
	base, _, _ := strings.Cut(path, "?")
	return w.goTo(base + "?aactor=" + actor)
}

func (w *world) everyAuditRowShows(action string) error {
	if err := w.browser.ExpectVisible("audit-row"); err != nil {
		return err
	}
	texts, err := w.auditRowTexts()
	if err != nil {
		return err
	}
	for _, text := range texts {
		if !strings.Contains(text, action) {
			return fmt.Errorf("an audit row reads %q", text)
		}
	}
	return nil
}

func (w *world) pingURLWithheld() error {
	b := w.browser
	if err := b.ExpectVisible("ping-url-withheld"); err != nil {
		return err
	}
	if err := b.ExpectAbsent("ping-url"); err != nil {
		return err
	}
	// The usage help repeats the URL, so it is gone too.
	return b.ExpectAbsent("ping-help")
}

func (w *world) revealPingURL() error {
	if err := w.browser.Submit("reveal-ping-url"); err != nil {
		return err
	}
	return w.browser.ExpectVisible("ping-url")
}

// unlockAdminActions: elevation gates granting access (create user, set
// password, promote), never removing it.
func (w *world) unlockAdminActions(password string) error {
	if err := w.goTo("/admin/unlock"); err != nil {
		return err
	}
	if err := w.browser.Fill("unlock-input", password); err != nil {
		return err
	}
	if err := w.browser.Submit("unlock-submit"); err != nil {
		return err
	}
	return w.browser.ExpectVisible("elevation-flash")
}

func (w *world) followConfirmLink() error {
	if err := w.goTo("/admin"); err != nil {
		return err
	}
	if err := w.browser.Submit("elevation-confirm-link"); err != nil {
		return err
	}
	return w.expectPath("/admin/unlock")
}

// lockAdminActions signs in again: there is no "lock now" control, and
// elevation dies with the session.
func (w *world) lockAdminActions() error {
	if err := w.signIn("admin", "correct horse battery"); err != nil {
		return err
	}
	if err := w.goTo("/admin"); err != nil {
		return err
	}
	return w.browser.ExpectVisible("elevation-confirm-link")
}

func (w *world) fillInNewUser(username, password string) error {
	if err := w.goTo("/admin"); err != nil {
		return err
	}
	if err := w.browser.Fill("user-username-input", username); err != nil {
		return err
	}
	return w.browser.Fill("user-password-input", password)
}

// clickWhenReady clicks without waiting for navigation: while locked, app.js
// opens a dialog instead.
func (w *world) clickWhenReady(id string) error {
	button, err := w.browser.TestID(id)
	if err != nil {
		return err
	}
	return button.ClickWhenReady()
}

func (w *world) reauthDialogNames(action string) error {
	b := w.browser
	if err := b.ExpectVisible("reauth-dialog"); err != nil {
		return err
	}
	if err := b.ExpectExactText("reauth-action", action); err != nil {
		return err
	}
	// Without this an admin goes hunting for an authenticator app.
	return b.ExpectText("reauth-why", "not a second factor")
}

func (w *world) answerDialogWrongly(password string) error {
	if err := w.browser.Fill("reauth-input", password); err != nil {
		return err
	}
	if err := w.clickWhenReady("reauth-submit"); err != nil {
		return err
	}
	return w.browser.ExpectVisible("reauth-error")
}

func (w *world) dismissDialog() error {
	if err := w.clickWhenReady("reauth-cancel"); err != nil {
		return err
	}
	return w.browser.ExpectHidden("reauth-dialog")
}

// registerAdminSteps covers the merged /admin page: cross-user access, the
// environment card, the audit trail and the elevation gate.
func registerAdminSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	sc.Step(`^`+quoted+` with password `+quoted+` owns a project `+quoted+` with a check `+quoted+` period (-?\d+)$`,
		func(username, password, project, check string, period int64) error {
			return w.memberOwnsProjectAndCheck(unquote(username), unquote(password), unquote(project), unquote(check), period)
		})
	// "All projects" is a section of /admin, not its own page.
	sc.Step(`^I open the admin (?:dashboard|projects list)$`, func() error { return w.goTo("/admin") })
	sc.Step(`^the admin dashboard is shown$`, func() error {
		if err := requireHeading(b(), "Admin"); err != nil {
			return err
		}
		return b().ExpectCount(`[data-testid="admin-scale"] .tile`, 4)
	})
	sc.Step(`^no card subheading renders larger than its card heading$`, w.subheadsAreSubordinate)
	sc.Step(`^the admin projects list shows `+quoted+` owned by `+quoted+`$`, func(project, owner string) error {
		return w.adminProjectsListShows(unquote(project), unquote(owner))
	})
	// Shared owner templates with /admin-prefixed forms, so owner steps apply.
	sc.Step(`^I open the member's project in the admin area$`, func() error {
		_, err := w.openAdminProjectSub("")
		return err
	})
	sc.Step(`^I open the member's check in the admin area$`, func() error {
		id, err := w.memberCheckID()
		if err != nil {
			return err
		}
		return w.goTo(fmt.Sprintf("/admin/checks/%d", id))
	})
	sc.Step(`^I am viewing the check `+quoted+`$`, func(name string) error {
		return requireHeading(b(), unquote(name))
	})
	sc.Step(`^I rename the project to `+quoted+`$`, func(name string) error { return w.renameProject(unquote(name)) })
	sc.Step(`^I am on the admin project page for `+quoted+`$`, func(name string) error {
		return w.onAdminProjectPage(unquote(name))
	})
	sc.Step(`^I add a webhook channel named `+quoted+`$`, func(name string) error {
		return w.addWebhookChannel(unquote(name))
	})
	sc.Step(`^the channel `+quoted+` is listed on the project$`, func(name string) error {
		return w.channelListed(unquote(name))
	})
	// Lands on /admin, where the owner flow lands on /.
	sc.Step(`^I delete the member's project$`, func() error {
		if err := b().ConfirmAndSubmit("delete-project-button"); err != nil {
			return err
		}
		return w.expectPath("/admin")
	})
	sc.Step(`^the admin projects list has no projects$`, func() error {
		return b().ExpectTextSomewhere("No projects yet.")
	})
	sc.Step(`^the Environment card shows the SMTP password as configured$`, func() error {
		return b().ExpectText("env-smtp-password", "configured")
	})
	sc.Step(`^the page does not contain the SMTP secret$`, w.pageHidesSMTPSecret)
	sc.Step(`^the audit trail has at least (-?\d+) row$`, w.auditHasRows)
	sc.Step(`^the audit trail shows an `+quoted+` entry$`, func(action string) error {
		return w.auditShowsEntry(unquote(action))
	})
	// The row is a tr.toggle; its tr.exp sibling gains .open on click.
	sc.Step(`^I expand the first audit row$`, func() error { return b().Click("audit-row") })
	sc.Step(`^the audit detail shows the request path$`, func() error {
		return b().ExpectTextCSS("#audit-section tr.exp.open", "/admin/")
	})
	sc.Step(`^I filter the audit trail by action `+quoted+`$`, func(action string) error {
		return w.filterAuditByAction(unquote(action))
	})
	sc.Step(`^I filter the audit trail by actor `+quoted+`$`, func(actor string) error {
		return w.filterAuditByActor(unquote(actor))
	})
	sc.Step(`^every audit row shows the action `+quoted+`$`, func(action string) error {
		return w.everyAuditRowShows(unquote(action))
	})
	sc.Step(`^the audit clear filter link is visible$`, func() error { return b().ExpectVisible("audit-clear") })
	sc.Step(`^the audit clear filter link is not visible$`, func() error { return b().ExpectAbsent("audit-clear") })
	sc.Step(`^I clear the audit filter$`, func() error {
		if err := b().Click("audit-clear"); err != nil {
			return err
		}
		// The link disappearing is the swap signal.
		return b().ExpectAbsent("audit-clear")
	})
	sc.Step(`^the audit trail is empty with a filtered message$`, func() error {
		return b().ExpectText("audit-empty", "No audit entries match the filter.")
	})
	sc.Step(`^the ping URL is withheld$`, w.pingURLWithheld)
	sc.Step(`^I reveal the ping URL$`, w.revealPingURL)
	sc.Step(`^I unlock admin actions with my password `+quoted+`$`, func(password string) error {
		return w.unlockAdminActions(unquote(password))
	})
	sc.Step(`^I follow the confirm link on the admin page$`, w.followConfirmLink)
	sc.Step(`^the confirmation page explains the requirement$`, func() error {
		for _, id := range []string{"unlock-gated", "unlock-cancel", "unlock-input"} {
			if err := b().ExpectVisible(id); err != nil {
				return err
			}
		}
		return nil
	})
	sc.Step(`^I lock admin actions$`, w.lockAdminActions)
	sc.Step(`^I fill in the new user `+quoted+` with password `+quoted+`$`, func(username, password string) error {
		return w.fillInNewUser(unquote(username), unquote(password))
	})
	sc.Step(`^I submit the new user form$`, func() error { return w.clickWhenReady("user-submit") })
	sc.Step(`^the confirmation dialog appears naming `+quoted+`$`, func(action string) error {
		return w.reauthDialogNames(unquote(action))
	})
	sc.Step(`^I confirm the dialog with password `+quoted+`$`, func(password string) error {
		if err := b().Fill("reauth-input", unquote(password)); err != nil {
			return err
		}
		return b().Submit("reauth-submit")
	})
	sc.Step(`^I answer the dialog with the wrong password `+quoted+`$`, func(password string) error {
		return w.answerDialogWrongly(unquote(password))
	})
	sc.Step(`^the dialog is still open with an error$`, func() error {
		if err := b().ExpectVisible("reauth-dialog"); err != nil {
			return err
		}
		return b().ExpectExactText("reauth-error", "That password is not correct.")
	})
	sc.Step(`^I dismiss the dialog$`, w.dismissDialog)
	sc.Step(`^the new user form still holds `+quoted+`$`, func(username string) error {
		return b().ExpectValue("user-username-input", unquote(username))
	})
	sc.Step(`^no confirmation dialog appears$`, func() error { return b().ExpectAbsent("reauth-dialog") })
}
