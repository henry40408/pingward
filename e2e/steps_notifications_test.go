package e2e

import (
	"fmt"
	"regexp"
	"strings"
	"time"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

// deliveryTimeout is how long to keep reloading while waiting for a delivery
// to be recorded.
const deliveryTimeout = 15 * time.Second

// channelDraft is a channel form to fill: the kind decides which inputs show.
type channelDraft struct {
	kind string
	name string
	// url overrides the canned webhook URL.
	url string
}

// fillChannelKind picks the kind first: app.css shows only that kind's
// inputs, and filling needs a visible target.
func (w *world) fillChannelKind(draft channelDraft) error {
	b := w.browser
	if err := b.FillCSS("#name", draft.name); err != nil {
		return err
	}
	if err := b.SelectOptionCSS("#kind", draft.kind); err != nil {
		return err
	}
	fills := map[string][][2]string{
		"webhook":  {{"#webhook_url", "http://127.0.0.1:1/hook"}},
		"slack":    {{"#slack_url", "https://hooks.slack.com/services/T0/B0/xxx"}},
		"telegram": {{"#telegram_token", "123:ABC"}, {"#telegram_chat_id", "999"}},
		"ntfy":     {{"#ntfy_topic", "my-topic"}},
		"pushover": {{"#pushover_token", "apptok"}, {"#pushover_user", "userkey"}},
	}
	inputs, known := fills[draft.kind]
	if !known {
		return fmt.Errorf("unsupported channel kind in test: %s", draft.kind)
	}
	for _, input := range inputs {
		value := input[1]
		if draft.kind == "webhook" && draft.url != "" {
			value = draft.url
		}
		if err := b.FillCSS(input[0], value); err != nil {
			return err
		}
	}
	return nil
}

func (w *world) openNewChannelForm() error {
	project, err := w.requireProject()
	if err != nil {
		return err
	}
	return w.goTo(project + "/channels/new")
}

func (w *world) createChannel(draft channelDraft) error {
	if err := w.openNewChannelForm(); err != nil {
		return err
	}
	if err := w.fillChannelKind(draft); err != nil {
		return err
	}
	if err := w.clickNamedButton("Create channel"); err != nil {
		return err
	}
	return w.expectPathMatching(`/projects/\d+$`)
}

func (w *world) clickNamedButton(name string) error {
	button, err := w.browser.ButtonNamed(name)
	if err != nil {
		return err
	}
	if button == nil {
		return fmt.Errorf("the page has no `%s` button", name)
	}
	return w.browser.SubmitElement(button)
}

// rowButton is the named button inside the row holding text.
func (w *world) rowButton(selector, rowText, button string) (*harness.Element, error) {
	row, err := w.cssRow(selector, rowText)
	if err != nil {
		return nil, err
	}
	found, err := row.ButtonNamed(button)
	if err != nil {
		return nil, err
	}
	if found == nil {
		return nil, fmt.Errorf("the row for %q has no `%s` button", rowText, button)
	}
	return found, nil
}

func (w *world) bindCheckbox(name string) (*harness.Element, error) {
	row, err := w.cssRow("label.chk", name)
	if err != nil {
		return nil, err
	}
	checkbox, err := row.FindOptional(`input[name="channel_ids"]`)
	if err != nil {
		return nil, err
	}
	if checkbox == nil {
		return nil, fmt.Errorf("the row for %q has no bind checkbox", name)
	}
	return checkbox, nil
}

// webhookURLOnMock points the scenario at the mock receiver's /hook.
func (w *world) webhookURLOnMock() string {
	w.webhookURL = w.mock().URL() + "/hook"
	return w.webhookURL
}

// registerNotificationsSteps covers creating, editing, binding and delivering
// notification channels. The channel form and rows mostly lack data-testids,
// so these steps go by input id, class and button text.
func registerNotificationsSteps(sc *godog.ScenarioContext, w *world) {
	registerChannelFormSteps(sc, w)
	registerChannelBindingSteps(sc, w)
	registerChannelDeliverySteps(sc, w)
}

func registerChannelFormSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	sc.Step(`^I create a (\S+) channel named `+quoted+`$`, func(kind, name string) error {
		return w.createChannel(channelDraft{kind: kind, name: unquote(name)})
	})
	// Recorded so the edit form can be checked for not printing it.
	sc.Step(`^a webhook channel named `+quoted+` targeting the mock server$`, func(name string) error {
		return w.createChannel(channelDraft{kind: "webhook", name: unquote(name), url: w.webhookURLOnMock()})
	})
	sc.Step(`^the project lists a channel named `+quoted+` of kind `+quoted+`$`, func(name, kind string) error {
		name = unquote(name)
		row, err := w.cssRow(".chk", name)
		if err != nil {
			return err
		}
		kindElement, err := row.FindOptional(".kind")
		if err != nil {
			return err
		}
		if kindElement == nil {
			return fmt.Errorf("the row for %q shows no kind", name)
		}
		rendered, err := kindElement.NormalizedText()
		if err != nil {
			return err
		}
		if rendered != unquote(kind) {
			return fmt.Errorf("%s is listed as kind %q", name, rendered)
		}
		return nil
	})
	sc.Step(`^I submit a webhook channel with a blank URL$`, func() error {
		if err := w.openNewChannelForm(); err != nil {
			return err
		}
		if err := b().FillCSS("#name", "bad hook"); err != nil {
			return err
		}
		if err := b().SelectOptionCSS("#kind", "webhook"); err != nil {
			return err
		}
		return w.clickNamedButton("Create channel")
	})
	sc.Step(`^the channel form shows an error `+quoted+`$`, func(message string) error {
		return b().ExpectExactTextCSS(".flash.err", unquote(message))
	})
	sc.Step(`^I open the new channel form$`, w.openNewChannelForm)
	sc.Step(`^the `+quoted+` channel kind is not offered$`, func(kind string) error {
		return b().ExpectCount(fmt.Sprintf(`#kind option[value="%s"]`, unquote(kind)), 0)
	})
	// Redirects back to the same URL, so SubmitElement waits on the document
	// being replaced rather than on the URL.
	sc.Step(`^I delete the channel named `+quoted+`$`, func(name string) error {
		button, err := w.rowButton(".chk", unquote(name), "delete")
		if err != nil {
			return err
		}
		b().SetDialogPolicy(harness.AcceptDialogs)
		defer b().SetDialogPolicy(harness.DismissDialogs)
		return b().SubmitElement(button)
	})
	sc.Step(`^the project shows no channels$`, func() error {
		if err := b().ExpectVisible("project-channels-empty"); err != nil {
			return err
		}
		return b().ExpectText("project-channels-empty", "nobody is notified")
	})
	sc.Step(`^I open the edit form for the channel `+quoted+`$`, func(name string) error {
		name = unquote(name)
		row, err := w.cssRow(".chk", name)
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
		if err := b().SubmitElement(link); err != nil {
			return err
		}
		return w.expectPathMatching(`/channels/\d+/edit$`)
	})
	sc.Step(`^the edit form hides the stored webhook URL$`, w.editFormHidesWebhookURL)
	sc.Step(`^I rename the channel to `+quoted+`$`, func(name string) error {
		if err := b().FillCSS("#name", unquote(name)); err != nil {
			return err
		}
		return w.saveChannelChanges()
	})
	sc.Step(`^I change the channel's webhook URL to the mock server$`, func() error {
		if err := b().FillCSS("#webhook_url", w.webhookURLOnMock()); err != nil {
			return err
		}
		return w.saveChannelChanges()
	})
	sc.Step(`^the kind is shown as static text `+quoted+`$`, func(kind string) error {
		if err := b().ExpectExactText("channel-kind-static", unquote(kind)); err != nil {
			return err
		}
		return b().ExpectCount("#kind", 0)
	})
}

func (w *world) saveChannelChanges() error {
	if err := w.clickNamedButton("Save changes"); err != nil {
		return err
	}
	return w.expectPathMatching(`/projects/\d+$`)
}

// editFormHidesWebhookURL's kind assertion is the non-vacuity guard: a page
// rendering no channel would pass every "absent" check.
func (w *world) editFormHidesWebhookURL() error {
	b := w.browser
	if err := b.ExpectExactText("channel-kind-static", "webhook"); err != nil {
		return err
	}
	if err := b.ExpectValueCSS("#webhook_url", ""); err != nil {
		return err
	}
	unchanged := "unchanged"
	if err := b.ExpectAttr("#webhook_url", "placeholder", &unchanged); err != nil {
		return err
	}
	if err := b.ExpectExactTextCSS(".pill.ok", "configured"); err != nil {
		return err
	}
	stored, err := w.requireWebhookURL()
	if err != nil {
		return err
	}
	body, err := harness.Eval[string](b, `document.body.textContent`)
	if err != nil {
		return err
	}
	if strings.Contains(body, stored) {
		return fmt.Errorf("the edit form prints the stored webhook URL")
	}
	return nil
}

func registerChannelBindingSteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	// Saving redirects to the same URL; SubmitElement waits on the document.
	sc.Step(`^I bind the channel `+quoted+` to the check$`, func(name string) error {
		checkbox, err := w.bindCheckbox(unquote(name))
		if err != nil {
			return err
		}
		ticked, err := checkbox.Checked()
		if err != nil {
			return err
		}
		if !ticked {
			if err := checkbox.ClickWhenReady(); err != nil {
				return err
			}
		}
		return w.clickNamedButton("Save channels")
	})
	sc.Step(`^the channel `+quoted+` is bound to the check$`, func(name string) error {
		name = unquote(name)
		checkbox, err := w.bindCheckbox(name)
		if err != nil {
			return err
		}
		ticked, err := checkbox.Checked()
		if err != nil {
			return err
		}
		if !ticked {
			return fmt.Errorf("the channel %q is not ticked", name)
		}
		return nil
	})
	sc.Step(`^a `+quoted+` confirmation is shown$`, func(message string) error {
		return b().ExpectExactText("check-flash", unquote(message))
	})
	// One-shot: the flash cookie is cleared on the render that shows it.
	sc.Step(`^the confirmation is gone after reloading$`, func() error {
		if err := b().Reload(); err != nil {
			return err
		}
		return b().ExpectAbsent("check-flash")
	})
	sc.Step(`^the check's notify channels show an empty state$`, func() error {
		return b().ExpectVisible("check-channels-empty")
	})
	// Clicks the row, not the name link, to exercise app.js's data-href.
	sc.Step(`^I visit the check page for `+quoted+`$`, func(name string) error {
		row, err := w.cssRow(".check", unquote(name))
		if err != nil {
			return err
		}
		if err := b().SubmitElement(row); err != nil {
			return err
		}
		return w.expectPathMatching(`/checks/\d+$`)
	})
	sc.Step(`^I visit the dashboard$`, func() error { return w.goTo("/") })
	sc.Step(`^the channel `+quoted+` shows as ON on the check page$`, func(name string) error {
		return w.expectChannelState(unquote(name), true)
	})
	sc.Step(`^the channel `+quoted+` shows as OFF on the check page$`, func(name string) error {
		return w.expectChannelState(unquote(name), false)
	})
	sc.Step(`^the dashboard shows a `+quoted+` chip for the check `+quoted+`$`, w.dashboardShowsChip)
	sc.Step(`^the dashboard shows no `+quoted+` chip for the check `+quoted+`$`, w.dashboardHidesChip)
}

// expectChannelState: channel-state-N wraps both .on and .off and CSS shows
// one, so visibility is asserted: textContent reads "ONOFF" either way.
func (w *world) expectChannelState(name string, on bool) error {
	row, err := w.cssRow("label.chk", name)
	if err != nil {
		return err
	}
	state, err := row.FindOptional(`[data-testid^="channel-state-"]`)
	if err != nil {
		return err
	}
	if state == nil {
		return fmt.Errorf("the row for %q has no state marker", name)
	}
	shownClass, hiddenClass, label := ".on", ".off", "ON"
	if !on {
		shownClass, hiddenClass, label = ".off", ".on", "OFF"
	}
	shown, err := state.FindOptional(shownClass)
	if err != nil {
		return err
	}
	if shown == nil {
		return fmt.Errorf("the state marker has no span")
	}
	hidden, err := state.FindOptional(hiddenClass)
	if err != nil {
		return err
	}
	if hidden == nil {
		return fmt.Errorf("the state marker has no counterpart span")
	}
	shownDisplayed, err := shown.Displayed()
	if err != nil {
		return err
	}
	if !shownDisplayed {
		return fmt.Errorf("the channel does not read %s", label)
	}
	hiddenDisplayed, err := hidden.Displayed()
	if err != nil {
		return err
	}
	if hiddenDisplayed {
		return fmt.Errorf("both states are displayed at once")
	}
	return nil
}

func (w *world) dashboardRow(name string) (*harness.Element, error) {
	rows, err := w.testIDsWithText("dashboard-check-row", name)
	if err != nil {
		return nil, err
	}
	if len(rows) == 0 {
		return nil, fmt.Errorf("no dashboard row for the check %q", name)
	}
	return rows[0], nil
}

func (w *world) dashboardShowsChip(chip, name string) error {
	chip, name = unquote(chip), unquote(name)
	row, err := w.dashboardRow(name)
	if err != nil {
		return err
	}
	marker, err := row.Find(`[data-testid="check-no-channel"]`)
	if err != nil {
		return err
	}
	rendered, err := marker.NormalizedText()
	if err != nil {
		return err
	}
	if rendered != chip {
		return fmt.Errorf("the check %q has no bound channel, so its row must carry the %q chip; it reads %q",
			name, chip, rendered)
	}
	return nil
}

// dashboardHidesChip relies on dashboardRow erroring on a missing row as the
// non-vacuity guard.
func (w *world) dashboardHidesChip(chip, name string) error {
	chip, name = unquote(chip), unquote(name)
	row, err := w.dashboardRow(name)
	if err != nil {
		return err
	}
	marker, err := row.FindOptional(`[data-testid="check-no-channel"]`)
	if err != nil {
		return err
	}
	if marker != nil {
		return fmt.Errorf("the check %q is bound to a channel, so its row must not carry the chip", name)
	}
	// The wording too, in case the warning moves to another test id.
	text, err := row.NormalizedText()
	if err != nil {
		return err
	}
	if strings.Contains(text, chip) {
		return fmt.Errorf("the check %q is bound to a channel, yet its row still reads %q", name, chip)
	}
	return nil
}

// notificationPillClass is the `.pill` class the recent-notifications table
// renders for an event.
func notificationPillClass(event string) (string, error) {
	switch event {
	case "down":
		return "fail", nil
	case "up":
		return "ok", nil
	case "reminder":
		return "start", nil
	}
	return "", fmt.Errorf("no notification pill is called `%s`", event)
}

func registerChannelDeliverySteps(sc *godog.ScenarioContext, w *world) {
	b := func() *harness.Browser { return w.browser }
	// Renders the project page directly (no redirect); the next assertion is
	// what waits for the banner.
	sc.Step(`^I send a test notification to the channel `+quoted+`$`, func(name string) error {
		button, err := w.rowButton(".chk", unquote(name), "Send test")
		if err != nil {
			return err
		}
		return button.ClickWhenReady()
	})
	sc.Step(`^a channel success banner is shown$`, func() error { return b().ExpectVisibleCSS(".flash.ok") })
	sc.Step(`^a channel error banner is shown$`, func() error { return b().ExpectVisibleCSS(".flash.err") })
	sc.Step(`^the mock server receives a `+quoted+` notification$`, func(event string) error {
		_, err := w.mock().WaitForPayload(unquote(event))
		return err
	})
	sc.Step(`^the `+quoted+` notification payload names project `+quoted+`, links the check, and blames `+quoted+`$`,
		w.payloadIsEnriched)
	// The row is recorded after the webhook POST returns, so reload until it
	// appears.
	sc.Step(`^the check's recent notifications show a delivery to `+quoted+`$`, func(channel string) error {
		channel = unquote(channel)
		return harness.EventuallyWithin(deliveryTimeout, "a recorded delivery to "+channel, func() (bool, error) {
			if err := b().Reload(); err != nil {
				return false, err
			}
			rows, err := w.cssWithText("tr", channel)
			if err != nil {
				return false, err
			}
			for _, row := range rows {
				text, err := row.NormalizedText()
				if err != nil {
					return false, err
				}
				if strings.Contains(text, "sent") {
					return true, nil
				}
			}
			return false, nil
		})
	})
	// Scoped so a ping's `.pill.fail` cannot satisfy a "down" event.
	sc.Step(`^the recent notifications table shows a `+quoted+` event$`, func(event string) error {
		class, err := notificationPillClass(unquote(event))
		if err != nil {
			return err
		}
		return b().ExpectVisibleCSS("#notifs-section .pill." + class)
	})
}

func (w *world) payloadIsEnriched(event, project, cause string) error {
	project, cause = unquote(project), unquote(cause)
	payload, err := w.mock().WaitForPayload(unquote(event))
	if err != nil {
		return err
	}
	field := func(name string) string {
		value, _ := payload[name].(string)
		return value
	}
	if field("project") != project {
		return fmt.Errorf("the payload names project %q", field("project"))
	}
	if field("cause") != cause {
		return fmt.Errorf("the payload blames %q", field("cause"))
	}
	link := field("url")
	linksCheck := regexp.MustCompile(`^` + regexp.QuoteMeta(w.server.BaseURL) + `/checks/\d+$`)
	if !linksCheck.MatchString(link) {
		return fmt.Errorf("the payload links %q", link)
	}
	if field("schedule") != "every 1h (grace 5m)" {
		return fmt.Errorf("the payload's schedule reads %q", field("schedule"))
	}
	if !strings.Contains(field("text"), link) {
		return fmt.Errorf("the message body does not carry the link")
	}
	return nil
}
