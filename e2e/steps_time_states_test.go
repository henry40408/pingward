package e2e

import (
	"strconv"
	"time"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

// transitionTimeout is an outer bound; the transition normally lands in about
// two seconds.
const transitionTimeout = 30 * time.Second

// createTimedCheck fills the check form with grace 0 (the form pre-fills 300)
// and an optional max runtime, then submits it.
func (w *world) createTimedCheck(name, period, maxRuntime string) error {
	b := w.browser
	if err := b.Submit("new-check-link"); err != nil {
		return err
	}
	if err := b.Fill("check-name-input", name); err != nil {
		return err
	}
	if err := b.Fill("check-period-input", period); err != nil {
		return err
	}
	if err := b.FillCSS("#grace_secs", "0"); err != nil {
		return err
	}
	if maxRuntime != "" {
		if err := b.FillCSS("#max_runtime_secs", maxRuntime); err != nil {
			return err
		}
	}
	if err := b.Submit("check-submit"); err != nil {
		return err
	}
	return w.expectPathMatching(`/checks/\d+$`)
}

// registerTimeStatesSteps covers states only the scan loop produces.
func registerTimeStatesSteps(sc *godog.ScenarioContext, w *world) {
	sc.Step(`^I create a check that falls due almost immediately$`, func() error {
		return w.createTimedCheck("overdue-job", "1", "")
	})
	// A long period rules out overdue, leaving only the overrun path.
	sc.Step(`^I create a check with a (-?\d+) second max runtime$`, func(seconds int) error {
		return w.createTimedCheck("overrun-job", "3600", strconv.Itoa(seconds))
	})
	// Server-side transition, so poll by reloading.
	sc.Step(`^the check status eventually becomes down$`, func() error {
		return harness.EventuallyWithin(transitionTimeout, "the check goes down", func() (bool, error) {
			if err := w.browser.Reload(); err != nil {
				return false, err
			}
			status, err := w.browser.TextOfTestID("check-status")
			return status != nil && *status == "down", err
		})
	})
}
