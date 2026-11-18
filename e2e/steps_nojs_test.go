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

// heartbeatAge matches a relative age such as "5s ago".
var heartbeatAge = regexp.MustCompile(`\d+[smhd] ago`)

// registerNoJSSteps covers `no_js.feature` (`@nojs`: page scripts disabled), so
// nothing here may rely on anything `app.js` creates. The output-panel
// assertions are also used by `check_history.feature` with script on, asserting
// the opposite, so leaving every panel open cannot pass both.
func registerNoJSSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	// POSTed, since only a body fills the expandable panel.
	sc.Step(`^I send a failing ping with output `+quoted+`$`, func(output string) error {
		pingURL, err := w.readPingURL()
		if err != nil {
			return err
		}
		return w.api().PingWithBody(pingURL, harness.PingFail, unquote(output))
	})
	// Visibility, not text: the output is always in the DOM, collapsed only by
	// `:root.js tr.exp:not(.open)`.
	sc.Step(`^the captured output `+quoted+` is visible$`, func(output string) error {
		return w.capturedOutputDisplayed(unquote(output), true)
	})
	sc.Step(`^the captured output `+quoted+` is hidden$`, func(output string) error {
		return w.capturedOutputDisplayed(unquote(output), false)
	})
	// Hidden with `opacity` (the column keeps its width), not `display`.
	sc.Step(`^the expand carets are invisible$`, w.expandCaretsInvisible)
	sc.Step(`^I expand the first ping row$`, func() error { return b().ClickCSS("#pings-section tr.toggle") })
	// The row's real anchor, not the delegated `data-href` handler.
	sc.Step(`^I click the dashboard check link for `+quoted+`$`, w.clickDashboardCheckLink)
	// Waits for a real navigation. With script the section is swapped in place
	// instead, hence the separately worded step in check history.
	sc.Step(`^I filter the pings by kind `+quoted+`$`, func(kind string) error {
		if err := b().SelectOption("pings-kind", unquote(kind)); err != nil {
			return err
		}
		return b().Submit("pings-apply")
	})
	sc.Step(`^I filter the notifications by event `+quoted+`$`, func(event string) error {
		if err := b().SelectOption("notifs-event", unquote(event)); err != nil {
			return err
		}
		return b().Submit("notifs-apply")
	})
	sc.Step(`^the pings kind filter shows `+quoted+`$`, func(kind string) error {
		return b().ExpectValue("pings-kind", unquote(kind))
	})
	sc.Step(`^the notifications event filter shows `+quoted+`$`, func(event string) error {
		return b().ExpectValue("notifs-event", unquote(event))
	})
	// A CDP-level media override, so it works with scripting disabled.
	sc.Step(`^my system prefers `+quoted+`$`, func(scheme string) error {
		return b().EmulateColorScheme(unquote(scheme))
	})
	sc.Step(`^the page background is light$`, func() error {
		luminance, err := w.backgroundLuminance()
		if err == nil && luminance <= 0.5 {
			err = fmt.Errorf("the background's luminance is %.3f", luminance)
		}
		return err
	})
	sc.Step(`^the page background is dark$`, func() error {
		luminance, err := w.backgroundLuminance()
		if err == nil && luminance >= 0.5 {
			err = fmt.Errorf("the background's luminance is %.3f", luminance)
		}
		return err
	})
	// Hidden by `:root:not(.js)` CSS, still in the DOM.
	sc.Step(`^the copy button is absent$`, func() error { return b().ExpectHiddenCSS(".copy") })
	sc.Step(`^the live tail toggle is absent$`, func() error { return b().ExpectHidden("pings-live") })
	sc.Step(`^the theme toggle is absent$`, func() error { return b().ExpectHiddenCSS("#pw-theme-toggle") })
	sc.Step(`^the scheduler heartbeat shows an age$`, w.heartbeatShowsAge)
	sc.Step(`^I start creating a check$`, func() error {
		if err := b().Submit("new-check-link"); err != nil {
			return err
		}
		return b().ExpectVisible("check-name-input")
	})
	// "I choose the {string} schedule kind" lives with check creation; it works
	// scriptless because `app.css`'s `:has()` rules follow the select.
	sc.Step(`^the period field is visible$`, func() error { return b().ExpectVisible("check-period-input") })
	sc.Step(`^the period field is hidden$`, func() error { return b().ExpectHidden("check-period-input") })
	sc.Step(`^the cron field is visible$`, func() error { return b().ExpectVisibleCSS("#cron_expr") })
	sc.Step(`^the cron field is hidden$`, func() error { return b().ExpectHiddenCSS("#cron_expr") })
	// Scriptless there is no `confirm()`; the click lands on an interstitial.
	sc.Step(`^I click the delete check button$`, func() error {
		button, err := b().TestID("delete-check-button")
		if err != nil {
			return err
		}
		return button.ClickWhenReady()
	})
	sc.Step(`^the confirmation page asks about deleting$`, func() error {
		if err := b().ExpectText("confirm-message", "history"); err != nil {
			return err
		}
		return b().ExpectVisible("confirm-submit")
	})
	sc.Step(`^I confirm the pending action$`, func() error { return b().Submit("confirm-submit") })
}

func (w *world) capturedOutputDisplayed(output string, expected bool) error {
	state := "hidden"
	if expected {
		state = "shown"
	}
	return harness.Eventually("the captured output is "+state, func() (bool, error) {
		panels, err := w.browser.CSSAll("#pings-section .out")
		if err != nil {
			return false, err
		}
		for _, panel := range panels {
			content, err := panel.NormalizedText()
			if err != nil || !strings.Contains(content, output) {
				continue
			}
			if displayed, _ := panel.Displayed(); displayed == expected {
				return true, nil
			}
		}
		return false, nil
	})
}

func (w *world) expandCaretsInvisible() error {
	carets, err := w.browser.CSSAll("#pings-section .caret")
	if err != nil {
		return err
	}
	if len(carets) == 0 {
		return errors.New("no carets on the page — assertion is vacuous")
	}
	for _, caret := range carets {
		opacity, err := caret.ComputedStyle("opacity")
		if err != nil {
			return err
		}
		if value, err := strconv.ParseFloat(opacity, 64); err != nil || value != 0 {
			return fmt.Errorf("a caret renders at opacity %s", opacity)
		}
	}
	return nil
}

func (w *world) clickDashboardCheckLink(name string) error {
	name = unquote(name)
	rows, err := w.browser.TestIDs("dashboard-check-row")
	if err != nil {
		return err
	}
	for _, row := range rows {
		content, err := row.NormalizedText()
		if err != nil || !strings.Contains(content, name) {
			continue
		}
		link, err := row.LinkNamed(name)
		if err != nil {
			return err
		}
		if link == nil {
			return fmt.Errorf("the row for %q has no name link", name)
		}
		return link.ClickWhenReady()
	}
	return fmt.Errorf("no dashboard row for %q", name)
}

// backgroundLuminance is the body background's luminance, so a palette tweak
// does not fail the test.
func (w *world) backgroundLuminance() (float64, error) {
	return harness.Eval[float64](w.browser, `(() => {
		const [r, g, b] = getComputedStyle(document.body)
			.backgroundColor.match(/\d+(\.\d+)?/g).map(Number);
		return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
	})()`)
}

func (w *world) heartbeatShowsAge() error {
	text, err := w.browser.TextOfCSS("[data-testid=sched-scan] .hb-ago")
	if err != nil {
		return err
	}
	age := ""
	if text != nil {
		age = *text
	}
	if !heartbeatAge.MatchString(age) {
		return fmt.Errorf("the heartbeat age reads %q", age)
	}
	return nil
}
