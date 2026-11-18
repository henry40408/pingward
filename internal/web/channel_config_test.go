package web

import (
	"testing"

	"github.com/henry40408/pingward/internal/models"
)

func TestValidateChannelRefusals(t *testing.T) {
	cases := []struct {
		name string
		in   ChannelInput
		want string
	}{
		{"no kind", ChannelInput{Name: "n"}, "a channel kind is required"},
		{"bad kind", ChannelInput{Name: "n", Kind: "fax"}, "unknown channel kind"},
		{"no name", ChannelInput{Kind: "webhook"}, "a channel name is required"},
		{"webhook", ChannelInput{Name: "n", Kind: "webhook"}, "a webhook URL is required"},
		{"slack", ChannelInput{Name: "n", Kind: "slack"}, "a Slack incoming-webhook URL is required"},
		{"telegram", ChannelInput{Name: "n", Kind: "telegram", TelegramToken: "t"}, "Telegram requires both a bot token and a chat id"},
		{"ntfy", ChannelInput{Name: "n", Kind: "ntfy"}, "ntfy requires a topic"},
		{"pushover", ChannelInput{Name: "n", Kind: "pushover", PushoverUser: "u"}, "Pushover requires both an application token and a user key"},
		{"email", ChannelInput{Name: "n", Kind: "email"}, "an email recipient address is required"},
	}
	for _, c := range cases {
		if _, _, _, refusal := ValidateChannel(c.in); refusal != c.want {
			t.Errorf("%s: refusal = %q, want %q", c.name, refusal, c.want)
		}
	}
}

func TestChannelConfigJSONIsSortedAndUnescaped(t *testing.T) {
	_, _, config, refusal := ValidateChannel(ChannelInput{Name: "n", Kind: "ntfy", NtfyTopic: "a&b"})
	if refusal != "" {
		t.Fatal(refusal)
	}
	if want := `{"base_url":"https://ntfy.sh","token":"","topic":"a&b"}`; config != want {
		t.Fatalf("config = %s", config)
	}
}

func TestEditKeepsStoredSecretsAndKind(t *testing.T) {
	existing := &models.Channel{Kind: models.ChannelNtfy, Name: "old", ConfigJSON: `{"base_url":"https://x","topic":"t","token":"secret"}`}
	kind, name, config, refusal := ValidateChannelUpdate(ChannelInput{Kind: "webhook", NtfyTopic: "t2"}, existing)
	if refusal != "" || kind != models.ChannelNtfy || name != "old" {
		t.Fatalf("%v %s %s %s", kind, name, config, refusal)
	}
	if want := `{"base_url":"https://x","token":"secret","topic":"t2"}`; config != want {
		t.Fatalf("config = %s", config)
	}
	_, _, config, _ = ValidateChannelUpdate(ChannelInput{NtfyTokenClear: true}, existing)
	if want := `{"base_url":"https://x","token":"","topic":"t"}`; config != want {
		t.Fatalf("cleared config = %s", config)
	}
}
