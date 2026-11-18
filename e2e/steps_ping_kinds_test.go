package e2e

import (
	"errors"
	"fmt"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

// pingPillClass is the `.pill` class the pings table renders for a ping kind.
func pingPillClass(kind string) (string, error) {
	switch kind {
	case "success":
		return "ok", nil
	case "fail", "start", "log":
		return kind, nil
	}
	return "", fmt.Errorf("no ping pill is called `%s`", kind)
}

// registerPingKindsSteps covers the /ping/* endpoints other than plain success.
func registerPingKindsSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	sc.Step(`^I send an exit code (-?\d+) ping$`, func(code int) error {
		pingURL, err := w.readPingURL()
		if err != nil {
			return err
		}
		return w.api().Ping(pingURL, harness.PingExit(code))
	})
	sc.Step(`^I send a `+quoted+` ping with body `+quoted+`$`, func(kind, body string) error {
		pingURL, err := w.readPingURL()
		if err != nil {
			return err
		}
		parsed, err := harness.ParsePingKind(unquote(kind))
		if err != nil {
			return err
		}
		return w.api().PingWithBody(pingURL, parsed, unquote(body))
	})
	sc.Step(`^I ping an unknown UUID$`, func() error {
		url := w.server.BaseURL + "/ping/00000000-0000-0000-0000-000000000000"
		status, err := w.api().PingStatus(url, harness.PingSuccess)
		w.pingStatus = status
		return err
	})
	sc.Step(`^the ping response status is (-?\d+)$`, func(status int) error {
		if w.pingStatus == 0 {
			return errors.New("no step recorded a ping response")
		}
		if w.pingStatus != status {
			return fmt.Errorf("the ping answered %d, not %d", w.pingStatus, status)
		}
		return nil
	})
	// The help is a closed `<details>`; open it first.
	sc.Step(`^the ping help documents the fail and start endpoints$`, func() error {
		help, err := b().TestID("ping-help")
		if err != nil {
			return err
		}
		summary, err := help.FindOptional("summary")
		if err != nil {
			return err
		}
		if summary == nil {
			return errors.New("the ping help has no summary to open")
		}
		if err := summary.ClickWhenReady(); err != nil {
			return err
		}
		if err := b().ExpectText("ping-help", "/fail"); err != nil {
			return err
		}
		return b().ExpectText("ping-help", "/start")
	})
	// Scoped: the ping help also renders `.pill`s.
	sc.Step(`^the recent pings table shows a `+quoted+` ping$`, func(kind string) error {
		class, err := pingPillClass(unquote(kind))
		if err != nil {
			return err
		}
		return b().ExpectVisibleCSS("#pings-section .pill." + class)
	})
	sc.Step(`^the recent pings table shows the exit `+quoted+`$`, func(exit string) error {
		return b().ExpectExactTextSomewhere(unquote(exit))
	})
	// Newest-first, and only rows with a body toggle.
	sc.Step(`^I expand the latest ping row$`, func() error { return b().ClickCSS("tr.toggle") })
	// Scoped: the ping help also renders `.out`.
	sc.Step(`^the captured output shows `+quoted+`$`, func(text string) error {
		if err := b().ExpectVisibleCSS("#pings-section .out"); err != nil {
			return err
		}
		return b().ExpectTextCSS("#pings-section .out", unquote(text))
	})
}
