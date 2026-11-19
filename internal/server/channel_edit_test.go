package server_test

import (
	"encoding/json"
	"net/http"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
)

type config map[string]string

func (a *app) addChannel(t *testing.T, projectID int64, kind models.ChannelKind, cfg config) int64 {
	t.Helper()
	encoded, err := json.Marshal(cfg)
	if err != nil {
		t.Fatal(err)
	}
	id, err := a.store.CreateChannel(t.Context(), store.NewChannel{
		ProjectID: projectID, Kind: kind, Name: "chan", ConfigJSON: string(encoded),
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return id
}

func (a *app) storedConfig(t *testing.T, channelID int64) config {
	t.Helper()
	channel, err := a.store.FindChannel(t.Context(), channelID)
	if err != nil || channel == nil {
		t.Fatalf("channel %d: %v", channelID, err)
	}
	var cfg config
	if err := json.Unmarshal([]byte(channel.ConfigJSON), &cfg); err != nil {
		t.Fatal(err)
	}
	return cfg
}

func (a *app) editBody(t *testing.T, channelID int64) string {
	t.Helper()
	res := a.client.Get("/channels/" + itoa(channelID) + "/edit")
	wantStatus(t, res, http.StatusOK)
	return res.Body
}

func memberProject(t *testing.T) (*app, int64) {
	t.Helper()
	a, ownerID := memberApp(t)
	projectID, err := a.store.CreateProject(t.Context(), ownerID, store.ProjectSettings{Name: "p"}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return a, projectID
}

// Every kind that stores a secret shows a configured pill instead of the
// value; the positive controls stop a broken page from passing.
func TestEditFormNeverRendersAStoredSecret(t *testing.T) {
	a, projectID := memberProject(t)
	cases := []struct {
		kind    models.ChannelKind
		config  config
		secrets []string
	}{
		{models.ChannelWebhook, config{"url": "https://hooks.example.com/WEBHOOK-SECRET"}, []string{"WEBHOOK-SECRET"}},
		{models.ChannelSlack, config{"url": "https://hooks.slack.com/services/SLACK-SECRET"}, []string{"SLACK-SECRET"}},
		{models.ChannelTelegram, config{"token": "TG-BOT-SECRET", "chat_id": "4242"}, []string{"TG-BOT-SECRET"}},
		{models.ChannelNtfy, config{"base_url": "https://ntfy.example.com", "topic": "alerts", "token": "NTFY-SECRET"}, []string{"NTFY-SECRET"}},
		{models.ChannelPushover, config{"token": "PO-APP-SECRET", "user": "PO-USER-SECRET"}, []string{"PO-APP-SECRET", "PO-USER-SECRET"}},
	}
	for _, c := range cases {
		body := a.editBody(t, a.addChannel(t, projectID, c.kind, c.config))
		for _, secret := range c.secrets {
			if strings.Contains(body, secret) {
				t.Errorf("%s edit form leaked a stored secret (%s)", c.kind, secret)
			}
		}
		for message, want := range map[string]string{
			"the pre-filled name":        `value="chan"`,
			"the immutable kind":         ">" + string(c.kind) + "<",
			"a configured pill":          ">configured<",
			"an unchanged-secret marker": `placeholder="unchanged"`,
		} {
			if !strings.Contains(body, want) {
				t.Errorf("%s edit form must show %s", c.kind, message)
			}
		}
	}
}

func TestEditFormPrefillsNonSecretFields(t *testing.T) {
	a, projectID := memberProject(t)
	telegram := a.addChannel(t, projectID, models.ChannelTelegram, config{"token": "TG-SECRET", "chat_id": "4242"})
	if !strings.Contains(a.editBody(t, telegram), `value="4242"`) {
		t.Error("the chat id must pre-fill")
	}
	ntfy := a.addChannel(t, projectID, models.ChannelNtfy, config{"base_url": "https://ntfy.example.com", "topic": "alerts", "token": ""})
	body := a.editBody(t, ntfy)
	for _, want := range []string{`value="https://ntfy.example.com"`, `value="alerts"`, ">not set<"} {
		if !strings.Contains(body, want) {
			t.Errorf("%q missing: an unset optional secret must read as not set", want)
		}
	}
}

// The secret input is always blank, so the merge must keep the stored value.
func TestEditRenamesWithoutResubmittingTheSecret(t *testing.T) {
	a, projectID := memberProject(t)
	const secret = "https://hooks.example.com/KEEP-ME"
	channelID := a.addChannel(t, projectID, models.ChannelWebhook, config{"url": secret})
	form := url.Values{"name": {"renamed"}, "webhook_url": {""}}
	wantStatus(t, a.post(t, "/channels/"+itoa(channelID), form), http.StatusSeeOther)
	channel, _ := a.store.FindChannel(t.Context(), channelID)
	if channel.Name != "renamed" || a.storedConfig(t, channelID)["url"] != secret {
		t.Fatalf("name = %q, config = %v", channel.Name, a.storedConfig(t, channelID))
	}
}

// A submitted secret overwrites; the page still shows neither value.
func TestEditRotatesASecretWithoutRenderingIt(t *testing.T) {
	a, projectID := memberProject(t)
	channelID := a.addChannel(t, projectID, models.ChannelWebhook, config{"url": "https://hooks.example.com/OLD"})
	form := url.Values{"name": {"chan"}, "webhook_url": {"https://hooks.example.com/NEW-SECRET"}}
	wantStatus(t, a.post(t, "/channels/"+itoa(channelID), form), http.StatusSeeOther)
	if got := a.storedConfig(t, channelID)["url"]; got != "https://hooks.example.com/NEW-SECRET" {
		t.Fatalf("url = %q", got)
	}
	body := a.editBody(t, channelID)
	if strings.Contains(body, "NEW-SECRET") || strings.Contains(body, "/OLD") {
		t.Fatal("neither the old nor the new secret may render")
	}
}

// The clear checkbox's value must parse as a bool, or it silently does nothing.
func TestEditClearsTheNtfyTokenViaTheCheckbox(t *testing.T) {
	a, projectID := memberProject(t)
	channelID := a.addChannel(t, projectID, models.ChannelNtfy, config{"base_url": "https://ntfy.sh", "topic": "alerts", "token": "NTFY-SECRET"})
	if !strings.Contains(a.editBody(t, channelID), `name="ntfy_token_clear" value="true"`) {
		t.Fatal("the clear checkbox must submit a parseable bool")
	}
	form := url.Values{
		"name": {"chan"}, "ntfy_base_url": {"https://ntfy.sh"}, "ntfy_topic": {"alerts"},
		"ntfy_token": {""}, "ntfy_token_clear": {"true"},
	}
	wantStatus(t, a.post(t, "/channels/"+itoa(channelID), form), http.StatusSeeOther)
	if cfg := a.storedConfig(t, channelID); cfg["token"] != "" || cfg["topic"] != "alerts" {
		t.Fatalf("config = %v", cfg)
	}
	if strings.Contains(a.editBody(t, channelID), `name="ntfy_token_clear"`) {
		t.Fatal("no token, no checkbox")
	}
}

// A rejected edit re-renders with the error, saves nothing and prints no
// stored secret; a blank name alone is not a rejection.
func TestRejectedEditReRendersWithoutLeakingOrSaving(t *testing.T) {
	a, projectID := memberProject(t)
	const secret = "https://hooks.example.com/KEEP-ME"
	kept := a.addChannel(t, projectID, models.ChannelWebhook, config{"url": secret})
	// Blank names are legal on edit, so reject via a blank URL with none stored.
	empty := a.addChannel(t, projectID, models.ChannelWebhook, config{})
	res := a.post(t, "/channels/"+itoa(empty), url.Values{"name": {"renamed"}, "webhook_url": {""}})
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "A webhook URL is required.") {
		t.Fatalf("missing the error: %s", res.Body)
	}
	if channel, _ := a.store.FindChannel(t.Context(), empty); channel.Name != "chan" {
		t.Fatal("a rejected edit must not apply the name")
	}

	wantStatus(t, a.post(t, "/channels/"+itoa(kept), url.Values{"name": {""}, "webhook_url": {""}}), http.StatusSeeOther)
	if channel, _ := a.store.FindChannel(t.Context(), kept); channel.Name != "chan" || a.storedConfig(t, kept)["url"] != secret {
		t.Fatal("a blank name and URL keep both stored values")
	}

	const telegramSecret = "123:TG-TOKEN-KEEP-ME"
	telegram := a.addChannel(t, projectID, models.ChannelTelegram, config{"token": telegramSecret})
	res = a.post(t, "/channels/"+itoa(telegram), url.Values{"telegram_chat_id": {""}})
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "Telegram requires both a bot token and a chat ID.") {
		t.Fatalf("missing the error: %s", res.Body)
	}
	if strings.Contains(res.Body, telegramSecret) {
		t.Fatal("the rejected re-render leaked the stored token")
	}
	if a.storedConfig(t, telegram)["token"] != telegramSecret {
		t.Fatal("a rejected edit must save nothing")
	}
}

// kind is immutable: no select on edit, and a submitted kind is ignored.
func TestEditFormDoesNotOfferAKindSelect(t *testing.T) {
	a, projectID := memberProject(t)
	channelID := a.addChannel(t, projectID, models.ChannelWebhook, config{"url": "https://hooks.example.com/S"})
	create := a.client.Get("/projects/" + itoa(projectID) + "/channels/new")
	if !strings.Contains(create.Body, `<select name="kind"`) {
		t.Fatal("the create form must still offer the kind select")
	}
	if strings.Contains(a.editBody(t, channelID), `<select name="kind"`) {
		t.Fatal("the edit form must render the kind as static text")
	}
	form := url.Values{"name": {"chan"}, "kind": {"slack"}, "slack_url": {"https://hooks.slack.com/services/X"}}
	wantStatus(t, a.post(t, "/channels/"+itoa(channelID), form), http.StatusSeeOther)
	if channel, _ := a.store.FindChannel(t.Context(), channelID); channel.Kind != models.ChannelWebhook {
		t.Fatal("a submitted kind must be ignored on edit")
	}
}

func TestProjectPageLinksToTheChannelEditForm(t *testing.T) {
	a, projectID := memberProject(t)
	channelID := a.addChannel(t, projectID, models.ChannelWebhook, config{"url": "https://hooks.example.com/S"})
	if !strings.Contains(a.projectBody(t, projectID), "/channels/"+itoa(channelID)+"/edit") {
		t.Fatal("the project page must link to each channel's edit form")
	}
}

// The admin edit form does not leak either, and posts back to /admin/....
func TestAdminCanEditAnotherUsersChannelWithoutLeaking(t *testing.T) {
	a, memberID := adminApp(t)
	projectID, err := a.store.CreateProject(t.Context(), memberID, store.ProjectSettings{Name: "p"}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	const secret = "https://hooks.example.com/BOB-SECRET"
	channelID := a.addChannel(t, projectID, models.ChannelWebhook, config{"url": secret})
	res := a.client.Get("/admin/channels/" + itoa(channelID) + "/edit")
	wantStatus(t, res, http.StatusOK)
	if strings.Contains(res.Body, "BOB-SECRET") {
		t.Fatal("the admin edit form leaked a secret")
	}
	if !strings.Contains(res.Body, `action="/admin/channels/`+itoa(channelID)+`"`) {
		t.Fatal("the admin form must post back to the admin route")
	}
	form := url.Values{"name": {"renamed-by-admin"}, "webhook_url": {""}}
	wantStatus(t, a.post(t, "/admin/channels/"+itoa(channelID), form), http.StatusSeeOther)
	channel, _ := a.store.FindChannel(t.Context(), channelID)
	if channel.Name != "renamed-by-admin" || a.storedConfig(t, channelID)["url"] != secret {
		t.Fatal("the admin rename must keep the secret")
	}
}
