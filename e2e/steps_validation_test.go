package e2e

import (
	"fmt"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

// registerValidationSteps covers the server-side validation of the project and
// check forms.
func registerValidationSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	sc.Step(`^I open the new project form$`, func() error { return w.goTo("/projects/new") })
	sc.Step(`^I fill the project name with `+quoted+`$`, func(name string) error {
		return b().Fill("project-name-input", unquote(name))
	})
	sc.Step(`^I fill the project scan interval with `+quoted+`$`, func(value string) error {
		return b().FillCSS("#scan_interval_secs", unquote(value))
	})
	sc.Step(`^I submit the project form$`, func() error { return b().Submit("project-submit") })
	sc.Step(`^the project form shows the error `+quoted+`$`, func(message string) error {
		return b().ExpectExactTextCSS(".flash.err", unquote(message))
	})
	sc.Step(`^the project name field shows `+quoted+`$`, func(name string) error {
		return b().ExpectValue("project-name-input", unquote(name))
	})
	sc.Step(`^I fill the check max runtime with `+quoted+`$`, func(value string) error {
		return b().FillCSS("#max_runtime_secs", unquote(value))
	})
	sc.Step(`^I fill the check timezone with `+quoted+`$`, func(value string) error {
		return b().FillCSS("#timezone", unquote(value))
	})
	sc.Step(`^the timezone field offers a list of zones$`, func() error {
		if err := b().ExpectAttrIs("#timezone", "list", "tz-list"); err != nil {
			return err
		}
		// Non-vacuity guard: an empty <datalist> would satisfy the above alone.
		count, err := b().CountCSS("#tz-list option")
		if err != nil {
			return err
		}
		if count <= 100 {
			return fmt.Errorf("the zone list offers only %d options", count)
		}
		return b().ExpectCount(`#tz-list option[value="Asia/Taipei"]`, 1)
	})
	sc.Step(`^every check duration field offers the same list of durations$`, func() error {
		// One shared list, so the fields cannot drift apart.
		for _, field := range []string{
			"#period_secs", "#grace_secs", "#scan_interval_secs", "#max_runtime_secs", "#nag_interval_secs",
		} {
			if err := b().ExpectAttrIs(field, "list", "dur-list"); err != nil {
				return err
			}
		}
		count, err := b().CountCSS("#dur-list option")
		if err != nil {
			return err
		}
		if count <= 3 {
			return fmt.Errorf("the duration list offers only %d options", count)
		}
		return b().ExpectCount(`#dur-list option[value="5m"]`, 1)
	})
}
