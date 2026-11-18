package e2e

import (
	"fmt"
	"net/url"
	"regexp"
	"strconv"
	"strings"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

// projectPath matches both /projects/{id} and /admin/projects/{id}: the admin
// pages reuse these steps under another prefix.
const projectPath = `/projects/\d+$`

// checkPath matches both /checks/{id} and /admin/checks/{id}.
const checkPath = `/checks/\d+$`

// cssWithText is every element matching the selector whose normalised text
// contains text.
func (w *world) cssWithText(selector, text string) ([]*harness.Element, error) {
	elements, err := w.browser.QueryAll(selector)
	if err != nil {
		return nil, err
	}
	var matched []*harness.Element
	for _, element := range elements {
		content, err := element.NormalizedText()
		if err == nil && strings.Contains(content, text) {
			matched = append(matched, element)
		}
	}
	return matched, nil
}

// cssRow waits for the first match of the selector whose text contains text.
func (w *world) cssRow(selector, text string) (*harness.Element, error) {
	var row *harness.Element
	err := harness.Eventually(fmt.Sprintf("`%s` containing %q", selector, text), func() (bool, error) {
		rows, err := w.cssWithText(selector, text)
		if err != nil || len(rows) == 0 {
			return false, err
		}
		row = rows[0]
		return true, nil
	})
	return row, err
}

func (w *world) testIDsWithText(id, text string) ([]*harness.Element, error) {
	return w.cssWithText(`[data-testid="`+id+`"]`, text)
}

func (w *world) createProject(name string) error {
	if err := w.goTo("/projects/new"); err != nil {
		return err
	}
	if err := w.browser.Fill("project-name-input", name); err != nil {
		return err
	}
	return w.browser.Submit("project-submit")
}

// createCheck creates a check from the current project page; grace and
// timezone are pre-filled.
func (w *world) createCheck(name string, period int) error {
	b := w.browser
	if err := b.Submit("new-check-link"); err != nil {
		return err
	}
	if err := b.Fill("check-name-input", name); err != nil {
		return err
	}
	if err := b.Fill("check-period-input", strconv.Itoa(period)); err != nil {
		return err
	}
	if err := b.Submit("check-submit"); err != nil {
		return err
	}
	return w.expectPathMatching(checkPath)
}

// registerMonitoringSteps covers projects, checks, pings and the dashboard.
func registerMonitoringSteps(sc *godog.ScenarioContext, w *world) {
	registerProjectSteps(sc, w)
	registerCheckLifecycleSteps(sc, w)
	registerDashboardSteps(sc, w)
}

func registerProjectSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	sc.Step(`^I create a project named `+quoted+`$`, func(name string) error {
		return w.createProject(unquote(name))
	})
	sc.Step(`^a project named `+quoted+`$`, func(name string) error {
		if err := w.createProject(unquote(name)); err != nil {
			return err
		}
		if err := w.expectPathMatching(projectPath); err != nil {
			return err
		}
		current, err := w.path()
		w.projectURL = current
		return err
	})
	sc.Step(`^I am on the project page for `+quoted+`$`, func(name string) error {
		name = unquote(name)
		if err := w.expectPathMatching(projectPath); err != nil {
			return err
		}
		found, err := b().HeadingExists(name)
		if err != nil {
			return err
		}
		if !found {
			return fmt.Errorf("no heading reads %q", name)
		}
		return nil
	})
	// The breadcrumb is the only link named exactly after the project.
	sc.Step(`^I visit the project page for `+quoted+`$`, func(project string) error {
		project = unquote(project)
		link, err := b().LinkExact(project)
		if err != nil {
			return err
		}
		if link == nil {
			return fmt.Errorf("no link named %q", project)
		}
		if err := b().SubmitElement(link); err != nil {
			return err
		}
		return w.expectPathMatching(projectPath)
	})
	sc.Step(`^the project has no checks$`, func() error { return b().ExpectVisible("checks-empty") })
	sc.Step(`^I delete the project$`, func() error {
		if err := b().ConfirmAndSubmit("delete-project-button"); err != nil {
			return err
		}
		return w.expectPathMatching(`/$`)
	})
}

func registerCheckLifecycleSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	sc.Step(`^I create a check named `+quoted+` with period (-?\d+)$`, func(name string, period int) error {
		return w.createCheck(unquote(name), period)
	})
	sc.Step(`^a check named `+quoted+` with period (-?\d+)$`, func(name string, period int) error {
		return w.createCheck(unquote(name), period)
	})
	sc.Step(`^I am on the check page$`, func() error { return w.expectPathMatching(checkPath) })
	sc.Step(`^the check status is `+quoted+`$`, func(status string) error {
		return b().ExpectExactText("check-status", unquote(status))
	})
	sc.Step(`^the check status is not `+quoted+`$`, func(status string) error {
		return b().ExpectNotExactText("check-status", unquote(status))
	})
	sc.Step(`^the ping URL is shown$`, func() error { return b().ExpectVisible("ping-url") })
	// The harness sets PINGWARD_BASE_URL, so the rendered URL is this server.
	sc.Step(`^I send a `+quoted+` ping$`, func(kind string) error {
		pingURL, err := w.readPingURL()
		if err != nil {
			return err
		}
		parsed, err := harness.ParsePingKind(unquote(kind))
		if err != nil {
			return err
		}
		if err := w.api().Ping(pingURL, parsed); err != nil {
			return err
		}
		w.pingURL = pingURL
		return nil
	})
	sc.Step(`^I reload the check page$`, func() error { return b().Reload() })
	sc.Step(`^I acknowledge the check$`, func() error { return b().Submit("ack-button") })
	sc.Step(`^the acknowledge control is gone$`, func() error { return b().ExpectAbsent("ack-button") })
	sc.Step(`^I pause the check$`, func() error { return b().Submit("pause-button") })
	sc.Step(`^I resume the check$`, func() error { return b().Submit("resume-button") })
	sc.Step(`^I regenerate the ping URL$`, func() error {
		before, err := w.readPingURL()
		if err != nil {
			return err
		}
		if err := b().ConfirmAndSubmit("regenerate-button"); err != nil {
			return err
		}
		// On the admin route the new URL is withheld again until revealed.
		if err := w.revealPingURLIfWithheld(); err != nil {
			return err
		}
		return b().ExpectNotExactText("ping-url", before)
	})
	// The comparison is made in the `When` step, which holds the old value.
	sc.Step(`^the ping URL is different from before$`, func() error { return b().ExpectVisible("ping-url") })
	sc.Step(`^I delete the check$`, func() error {
		if err := b().ConfirmAndSubmit("delete-check-button"); err != nil {
			return err
		}
		return w.expectPathMatching(projectPath)
	})
	sc.Step(`^the recent pings table shows an empty state$`, func() error { return b().ExpectVisible("pings-empty") })
	sc.Step(`^the recent notifications table shows an empty state$`, func() error {
		return b().ExpectVisible("notifications-empty")
	})
}

func registerDashboardSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	sc.Step(`^the dashboard shows no projects$`, func() error { return b().ExpectVisible("dashboard-empty") })
	sc.Step(`^I filter the dashboard by `+quoted+`$`, func(term string) error {
		term = unquote(term)
		if err := w.goTo("/"); err != nil {
			return err
		}
		if err := b().Fill("dashboard-filter-input", term); err != nil {
			return err
		}
		if err := b().Submit("dashboard-filter-submit"); err != nil {
			return err
		}
		// A GET form, so the term arrives form-encoded (space as `+`); the empty
		// `status` field may follow `q` in the query.
		query := url.Values{"q": {term}}.Encode()
		return w.expectPathMatching(`\?` + regexp.QuoteMeta(query) + `(&|$)`)
	})
	sc.Step(`^I filter the dashboard by status `+quoted+`$`, func(label string) error {
		label = unquote(label)
		if err := w.goTo("/"); err != nil {
			return err
		}
		if err := b().SelectOptionByText("dashboard-status-filter", label); err != nil {
			return err
		}
		if err := b().Submit("dashboard-filter-submit"); err != nil {
			return err
		}
		return w.expectPathMatching("status=" + regexp.QuoteMeta(strings.ToLower(label)) + `(&|$)`)
	})
	sc.Step(`^I clear the dashboard filter$`, func() error {
		if err := b().Submit("dashboard-filter-clear"); err != nil {
			return err
		}
		return w.expectPathMatching(`/$`)
	})
	sc.Step(`^I click the dashboard row for `+quoted+`$`, func(name string) error {
		name = unquote(name)
		if err := w.goTo("/"); err != nil {
			return err
		}
		rows, err := w.testIDsWithText("dashboard-check-row", name)
		if err != nil {
			return err
		}
		if len(rows) == 0 {
			return fmt.Errorf("no dashboard row for %q", name)
		}
		// Not the name: it is a real `<a>`, which would leave `data-href` untested.
		schedule, err := rows[0].FindOptional(".sc")
		if err != nil {
			return err
		}
		if schedule == nil {
			return fmt.Errorf("the row for %q has no schedule line", name)
		}
		return b().SubmitElement(schedule)
	})
	sc.Step(`^I click the row's edit link for `+quoted+`$`, func(name string) error {
		name = unquote(name)
		row, err := w.cssRow(".check", name)
		if err != nil {
			return err
		}
		link, err := row.LinkNamed("edit")
		if err != nil {
			return err
		}
		if link == nil {
			return fmt.Errorf("the row for %q has no edit link", name)
		}
		return link.ClickWhenReady()
	})
	sc.Step(`^I am on the check edit form$`, func() error {
		if err := w.expectPathMatching(`/checks/\d+/edit$`); err != nil {
			return err
		}
		return b().ExpectVisible("check-name-input")
	})
	sc.Step(`^the dashboard shows the check `+quoted+`$`, func(name string) error {
		name = unquote(name)
		return harness.Eventually(fmt.Sprintf("the dashboard lists %q", name), func() (bool, error) {
			rows, err := w.testIDsWithText("dashboard-check-row", name)
			return len(rows) > 0, err
		})
	})
	sc.Step(`^the dashboard does not show the check `+quoted+`$`, func(name string) error {
		name = unquote(name)
		return harness.Eventually(fmt.Sprintf("the dashboard drops %q", name), func() (bool, error) {
			rows, err := w.testIDsWithText("dashboard-check-row", name)
			return len(rows) == 0, err
		})
	})
	sc.Step(`^the dashboard says nothing matched$`, func() error {
		if err := b().ExpectVisible("dashboard-no-results"); err != nil {
			return err
		}
		// "Nothing matched" must not also claim "no projects".
		return b().ExpectAbsent("dashboard-empty")
	})
}
