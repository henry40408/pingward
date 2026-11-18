package e2e

import (
	"fmt"
	"regexp"
	"strings"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

var apiKeyToken = regexp.MustCompile(`^pw_[0-9a-f]{64}$`)

// registerAccountSteps covers the per-user account page: sessions, password
// and API keys.
func registerAccountSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	sc.Step(`^I open the account page$`, func() error { return w.goTo("/account") })
	sc.Step(`^the current session is marked as this device$`, func() error {
		return b().ExpectVisible("session-current")
	})
	sc.Step(`^I revoke the current session$`, func() error {
		return b().ConfirmAndSubmit("session-revoke")
	})
	sc.Step(`^I change my password from `+quoted+` to `+quoted+`$`, func(current, next string) error {
		current, next = unquote(current), unquote(next)
		for _, field := range [][2]string{
			{"current-password-input", current},
			{"new-password-input", next},
			{"confirm-password-input", next},
		} {
			if err := b().Fill(field[0], field[1]); err != nil {
				return err
			}
		}
		return b().Submit("password-submit")
	})
	sc.Step(`^the password change is confirmed$`, func() error {
		return b().ExpectVisible("password-changed-flash")
	})
	sc.Step(`^the password change is rejected$`, func() error {
		if err := b().ExpectVisible("password-error"); err != nil {
			return err
		}
		return b().ExpectAbsent("password-changed-flash")
	})
	// Re-authenticates: a key outlives the session that minted it.
	sc.Step(`^I create an API key named `+quoted+` with my password `+quoted+`$`, func(name, password string) error {
		if err := b().Fill("api-key-name-input", unquote(name)); err != nil {
			return err
		}
		if err := b().Fill("api-key-password-input", unquote(password)); err != nil {
			return err
		}
		return b().Submit("api-key-submit")
	})
	sc.Step(`^the API key creation is rejected$`, func() error {
		if err := b().ExpectVisible("api-key-error"); err != nil {
			return err
		}
		return b().ExpectAbsent("api-key-token")
	})
	sc.Step(`^the new API key token is shown once$`, func() error {
		if err := b().ExpectVisible("api-key-token"); err != nil {
			return err
		}
		token, err := b().TextOf("api-key-token")
		if err != nil {
			return err
		}
		if !apiKeyToken.MatchString(token) {
			return fmt.Errorf("the token reads %q", token)
		}
		return nil
	})
	sc.Step(`^the API keys list shows a key named `+quoted+`$`, func(name string) error {
		return b().ExpectExactTextSomewhere(unquote(name))
	})
	sc.Step(`^I revoke the API key$`, func() error { return b().ConfirmAndSubmit("api-key-delete") })
	sc.Step(`^no API keys remain$`, func() error { return b().ExpectVisible("api-keys-empty") })
	// Honoured only under the @trusted-proxy tag.
	sc.Step(`^requests arrive through a trusted proxy as `+quoted+`$`, func(ip string) error {
		return b().SetExtraHeaders(map[string]string{"x-forwarded-for": unquote(ip)})
	})
	// Proves login stores the resolved client IP, not the socket peer.
	sc.Step(`^the current session shows the IP `+quoted+`$`, func(ip string) error {
		ip = unquote(ip)
		if err := b().ExpectVisible("session-current"); err != nil {
			return err
		}
		text, err := b().RowTextContainingTestID("session-current")
		if err != nil {
			return err
		}
		if !strings.Contains(text, ip) {
			return fmt.Errorf("the current session row reads %q", text)
		}
		return nil
	})
}
