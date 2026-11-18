package e2e

import (
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

// revealPingURLIfWithheld reveals the ping URL when it is withheld: an admin
// on someone else's check must reveal it.
func (w *world) revealPingURLIfWithheld() error {
	control, err := w.browser.TestIDOpt("reveal-ping-url")
	if err != nil || control == nil {
		return err
	}
	if err := w.browser.Submit("reveal-ping-url"); err != nil {
		return err
	}
	return w.browser.ExpectVisible("ping-url")
}

// readPingURL reads the check page's ping URL, revealing it first when
// withheld.
func (w *world) readPingURL() (string, error) {
	if err := w.revealPingURLIfWithheld(); err != nil {
		return "", err
	}
	return w.browser.TextOf("ping-url")
}

func (w *world) openNewCheckForm() error {
	if err := w.browser.Submit("new-check-link"); err != nil {
		return err
	}
	return w.expectPathMatching(`/projects/\d+/checks/new$`)
}

// registerCheckCreateSteps covers the check-creation form's branches.
func registerCheckCreateSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	sc.Step(`^I open the new check form$`, w.openNewCheckForm)
	sc.Step(`^I create a cron check named `+quoted+` with expression `+quoted+`$`, func(name, expression string) error {
		if err := w.openNewCheckForm(); err != nil {
			return err
		}
		if err := b().Fill("check-name-input", unquote(name)); err != nil {
			return err
		}
		if err := b().SelectOptionCSS("#schedule_kind", "cron"); err != nil {
			return err
		}
		if err := b().FillCSS("#cron_expr", unquote(expression)); err != nil {
			return err
		}
		if err := b().Submit("check-submit"); err != nil {
			return err
		}
		return w.expectPathMatching(`/checks/\d+$`)
	})
	sc.Step(`^I fill the check name with `+quoted+`$`, func(name string) error {
		return b().Fill("check-name-input", unquote(name))
	})
	// Both integers and human-readable durations (1h30m).
	sc.Step(`^I fill the check period with (\d+)$`, func(period int) error {
		return b().Fill("check-period-input", strconv.Itoa(period))
	})
	sc.Step(`^I fill the check period with `+quoted+`$`, func(period string) error {
		return b().Fill("check-period-input", unquote(period))
	})
	sc.Step(`^I choose the `+quoted+` schedule kind$`, func(kind string) error {
		return b().SelectOptionCSS("#schedule_kind", unquote(kind))
	})
	sc.Step(`^I submit the check form$`, func() error { return b().Click("check-submit") })
	sc.Step(`^the check schedule shows `+quoted+`$`, func(text string) error {
		return b().ExpectTextSomewhere(unquote(text))
	})
	// `required` blocks the submit client-side; no POST fires.
	sc.Step(`^I am still on the new check form$`, func() error {
		if err := w.expectPathMatching(`/projects/\d+/checks/new$`); err != nil {
			return err
		}
		return b().ExpectVisible("check-submit")
	})
	// Switched by :has() rules in the stylesheet, not script.
	sc.Step(`^only the period field is shown$`, func() error {
		if err := b().ExpectVisible("check-period-input"); err != nil {
			return err
		}
		return b().ExpectHiddenCSS("#cron_expr")
	})
	sc.Step(`^only the cron field is shown$`, func() error {
		if err := b().ExpectVisibleCSS("#cron_expr"); err != nil {
			return err
		}
		return b().ExpectHidden("check-period-input")
	})
	sc.Step(`^the check name field is required$`, func() error {
		missing, err := harness.Eval[bool](b(), `document.querySelector('[data-testid="check-name-input"]').validity.valueMissing`)
		if err != nil {
			return err
		}
		if !missing {
			return errors.New("the browser did not refuse the empty name (validity.valueMissing was false)")
		}
		return nil
	})
	// The message contains quotes, which keep their escaping backslashes.
	sc.Step(`^the check form shows the error `+quoted+`$`, func(message string) error {
		return b().ExpectExactTextCSS(".flash.err", unquote(message))
	})
}

// registerEditFlowSteps covers renaming and re-scheduling existing projects
// and checks, and the markdown descriptions.
func registerEditFlowSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	openEditForm := func(expected string) error {
		link, err := b().LinkExact("Edit")
		if err != nil {
			return err
		}
		if link == nil {
			return errors.New("the page has no `Edit` link")
		}
		if err := b().SubmitElement(link); err != nil {
			return err
		}
		return w.expectPathMatching(expected)
	}
	sc.Step(`^I open the project edit form$`, func() error { return openEditForm(`/projects/\d+/edit$`) })
	sc.Step(`^I open the check edit form$`, func() error { return openEditForm(`/checks/\d+/edit$`) })
	fillAndSubmit := func(field, value, submit string) error {
		if err := b().Fill(field, value); err != nil {
			return err
		}
		return b().Submit(submit)
	}
	sc.Step(`^I change the project name to `+quoted+`$`, func(name string) error {
		return fillAndSubmit("project-name-input", unquote(name), "project-submit")
	})
	sc.Step(`^I change the check name to `+quoted+`$`, func(name string) error {
		return fillAndSubmit("check-name-input", unquote(name), "check-submit")
	})
	sc.Step(`^I change the check period to (\d+)$`, func(period int) error {
		return fillAndSubmit("check-period-input", strconv.Itoa(period), "check-submit")
	})
	sc.Step(`^I change the check grace to (\d+)$`, func(grace int) error {
		if err := b().FillCSS("#grace_secs", strconv.Itoa(grace)); err != nil {
			return err
		}
		return b().Submit("check-submit")
	})
	sc.Step(`^I change the check timezone to `+quoted+`$`, func(timezone string) error {
		if err := b().FillCSS("#timezone", unquote(timezone)); err != nil {
			return err
		}
		return b().Submit("check-submit")
	})
	sc.Step(`^the check name is `+quoted+`$`, func(name string) error {
		name = unquote(name)
		found, err := b().HeadingExists(name)
		if err != nil {
			return err
		}
		if !found {
			return fmt.Errorf("no heading reads %q", name)
		}
		return nil
	})
	// Read off the edit form; the check page does not display the timezone.
	sc.Step(`^the check timezone field shows `+quoted+`$`, func(timezone string) error {
		return b().ExpectValueCSS("#timezone", unquote(timezone))
	})
	sc.Step(`^the check period field shows `+quoted+`$`, func(period string) error {
		return b().ExpectValue("check-period-input", unquote(period))
	})
	sc.Step(`^I set the project description to `+quoted+`$`, func(description string) error {
		return fillAndSubmit("project-description-input", unquote(description), "project-submit")
	})
	sc.Step(`^I set the check description to `+quoted+`$`, func(description string) error {
		return fillAndSubmit("check-description-input", unquote(description), "check-submit")
	})
	descriptionBold := func(id, text string) error {
		card, err := b().TestID(id)
		if err != nil {
			return err
		}
		strong, err := card.FindOptional("strong")
		if err != nil {
			return err
		}
		if strong == nil {
			return fmt.Errorf("`%s` rendered no <strong>", id)
		}
		rendered, err := strong.NormalizedText()
		if err != nil {
			return err
		}
		if rendered != text {
			return fmt.Errorf("the bold run reads %q, not %q", rendered, text)
		}
		return nil
	}
	sc.Step(`^the project description shows `+quoted+` in bold$`, func(text string) error {
		return descriptionBold("project-description", unquote(text))
	})
	sc.Step(`^the check description shows `+quoted+` in bold$`, func(text string) error {
		return descriptionBold("check-description", unquote(text))
	})
	// Markers are stripped; the ellipsis proves truncation happened.
	sc.Step(`^the check row shows a truncated description$`, func() error {
		if err := b().ExpectVisible("check-description-summary"); err != nil {
			return err
		}
		if err := b().ExpectText("check-description-summary", "…"); err != nil {
			return err
		}
		summary, err := b().TextOf("check-description-summary")
		if err != nil {
			return err
		}
		if strings.Contains(summary, "**") {
			return fmt.Errorf("the summary still carries markdown markers: %q", summary)
		}
		return nil
	})
}

// registerCheckHistorySteps covers the check page's pings pager, filters and
// heartbeat strip.
func registerCheckHistorySteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	// Bodyless GETs, so each renders as one plain (non-toggle) row.
	sc.Step(`^I send (\d+) `+quoted+` pings$`, func(count int, kind string) error {
		pingURL, err := w.readPingURL()
		if err != nil {
			return err
		}
		parsed, err := harness.ParsePingKind(unquote(kind))
		if err != nil {
			return err
		}
		for range count {
			if err := w.api().Ping(pingURL, parsed); err != nil {
				return err
			}
		}
		return nil
	})
	sc.Step(`^the pings table shows (\d+) rows$`, func(count int) error {
		return b().ExpectCount(`[data-testid="ping-row"]`, count)
	})
	// A pager end renders as <span class="btn disabled">, not hidden, so this
	// asserts the class rather than visibility.
	pagerState := func(direction string, expectedDisabled bool) error {
		link, err := b().TestID("pings-" + direction)
		if err != nil {
			return err
		}
		classes, err := link.Classes()
		if err != nil {
			return err
		}
		if slices.Contains(classes, "disabled") != expectedDisabled {
			return fmt.Errorf("the %s link's classes are %q", direction, classes)
		}
		return nil
	}
	sc.Step(`^the pings (\w+) link is enabled$`, func(direction string) error { return pagerState(direction, false) })
	sc.Step(`^the pings (\w+) link is disabled$`, func(direction string) error { return pagerState(direction, true) })
	sc.Step(`^I click the pings older link$`, func() error { return b().Click("pings-older") })
	sc.Step(`^I click the pings newer link$`, func() error { return b().Click("pings-newer") })
	// Distinct from the no-JS "I filter the pings by kind", which waits for a
	// navigation: with script the section is swapped in place, so the following
	// row-count assertion is what waits.
	sc.Step(`^I filter pings by kind `+quoted+`$`, func(kind string) error {
		if err := b().SelectOption("pings-kind", unquote(kind)); err != nil {
			return err
		}
		return b().Click("pings-apply")
	})
	sc.Step(`^I clear the pings filter$`, func() error { return b().Click("pings-clear") })
	sc.Step(`^I set the pings from date to `+quoted+`$`, func(value string) error {
		return b().Fill("pings-from", unquote(value))
	})
	sc.Step(`^I apply the pings filter$`, func() error { return b().Click("pings-apply") })
	// The local wall clock round-trips through UTC, so any runner zone works.
	sc.Step(`^the pings from date is `+quoted+`$`, func(value string) error {
		return b().ExpectValue("pings-from", unquote(value))
	})
	sc.Step(`^the pings clear filter link is visible$`, func() error { return b().ExpectVisible("pings-clear") })
	sc.Step(`^the pings clear filter link is not visible$`, func() error { return b().ExpectAbsent("pings-clear") })
	sc.Step(`^the newest heartbeat bar is flush with the strip's right edge$`, func() error {
		gap, err := harness.Eval[float64](b(), `(() => {
			const beat = document.querySelector('.beat');
			const bars = beat.querySelectorAll('i');
			const last = bars[bars.length - 1].getBoundingClientRect();
			return Math.round(beat.getBoundingClientRect().right - last.right);
		})()`)
		if err != nil {
			return err
		}
		if gap > 1 {
			return fmt.Errorf("the newest run sits %vpx from the right edge: the strip is not right-aligned", gap)
		}
		return nil
	})
	// scrollWidth ignores overflow off the left, so count bars instead.
	sc.Step(`^the oldest heartbeat bars are clipped off the left$`, func() error {
		type bars struct{ Rendered, Visible, Box int }
		measured, err := harness.Eval[bars](b(), `(() => {
			const beat = document.querySelector('.beat');
			const box = beat.getBoundingClientRect();
			const bars = [...beat.querySelectorAll('i')];
			return {
				Rendered: bars.length,
				Visible: bars.filter((x) => x.getBoundingClientRect().left >= box.left - 0.5).length,
				Box: Math.round(box.width),
			};
		})()`)
		if err != nil {
			return err
		}
		if measured.Rendered <= 30 {
			return fmt.Errorf("only %d bars were rendered", measured.Rendered)
		}
		if measured.Visible >= measured.Rendered {
			return fmt.Errorf("all %d bars fit the %dpx strip: nothing is being clipped, so this proves nothing",
				measured.Rendered, measured.Box)
		}
		return nil
	})
}

// registerLiveTailSteps covers the check page's opt-in SSE live tail.
func registerLiveTailSteps(sc *godog.ScenarioContext, w *world) {
	// Events publish only while a receiver is subscribed, with no catch-up, so
	// a ping before the EventSource opens is lost: wait for data-live="open".
	sc.Step(`^I turn on the live tail$`, func() error {
		if err := w.browser.Click("pings-live"); err != nil {
			return err
		}
		return w.browser.ExpectAttrIs(`[data-testid="pings-live"]`, "data-live", "open")
	})
	// A fixed wait: this asserts an absence, and the tail refreshes 500 ms
	// after its signal, so an immediate check would pass with the tail wrongly on.
	sc.Step(`^the recent pings table still shows no pings$`, func() error {
		time.Sleep(1500 * time.Millisecond)
		if err := w.browser.ExpectVisible("pings-empty"); err != nil {
			return err
		}
		return w.browser.ExpectAbsent("ping-row")
	})
	sc.Step(`^the ping filters are hidden$`, func() error { return w.browser.ExpectHidden("pings-filters") })
}
