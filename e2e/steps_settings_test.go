package e2e

import (
	"errors"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

// registerSettingsSteps covers the instance settings form, whose inputs have
// ids but no data-testid.
func registerSettingsSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	sc.Step(`^I fill the settings field `+quoted+` with `+quoted+`$`, func(field, value string) error {
		return b().FillCSS("#"+unquote(field), unquote(value))
	})
	// The URL is unchanged on success or error, so wait for the document to be
	// replaced rather than for the URL.
	sc.Step(`^I save the settings form$`, func() error {
		button, err := b().ButtonNamed("Save changes")
		if err != nil {
			return err
		}
		if button == nil {
			return errors.New("the settings form has no `Save changes` button")
		}
		return b().SubmitElementWhenReady(button)
	})
	sc.Step(`^the settings field `+quoted+` shows `+quoted+`$`, func(field, value string) error {
		return b().ExpectValueCSS("#"+unquote(field), unquote(value))
	})
	sc.Step(`^the settings form shows the error `+quoted+`$`, func(message string) error {
		return b().ExpectExactTextCSS(".flash.err", unquote(message))
	})
	sc.Step(`^the settings page shows the flash `+quoted+`$`, func(message string) error {
		return b().ExpectExactText("settings-flash", unquote(message))
	})
	// One-shot: gone on reload, never set by a rejected save.
	sc.Step(`^the settings page shows no flash$`, func() error {
		return b().ExpectAbsent("settings-flash")
	})
}
