package notify

import (
	"encoding/json"
	"log/slog"
	"strings"

	"github.com/henry40408/pingward/internal/config"
	"github.com/henry40408/pingward/internal/models"
)

// configString is a trimmed, non-empty string field of the channel config.
func configString(values map[string]any, key string) (string, bool) {
	text, ok := values[key].(string)
	text = strings.TrimSpace(text)
	return text, ok && text != ""
}

// NotifierFor builds the notifier for a channel. It returns false, after a
// warning, when the config is invalid or a required field is blank; delivery
// skips such channels.
func NotifierFor(channel models.Channel, smtp *config.SMTPConfig) (Notifier, bool) {
	var values map[string]any
	if err := json.Unmarshal([]byte(channel.ConfigJSON), &values); err != nil {
		slog.Warn("invalid config_json", "channel_id", channel.ID, "error", err)
		return nil, false
	}
	missing := func(fields string) (Notifier, bool) {
		slog.Warn("channel missing required config field: "+fields,
			"channel_id", channel.ID, "kind", string(channel.Kind))
		return nil, false
	}
	switch channel.Kind {
	case models.ChannelWebhook, models.ChannelSlack:
		url, ok := configString(values, "url")
		if !ok {
			return missing("url")
		}
		if channel.Kind == models.ChannelSlack {
			return NewSlack(url), true
		}
		return NewWebhook(url), true
	case models.ChannelTelegram:
		token, hasToken := configString(values, "token")
		chatID, hasChat := configString(values, "chat_id")
		if !hasToken || !hasChat {
			return missing("token/chat_id")
		}
		return NewTelegram(token, chatID), true
	case models.ChannelNtfy:
		topic, ok := configString(values, "topic")
		if !ok {
			return missing("topic")
		}
		baseURL, hasBase := configString(values, "base_url")
		if !hasBase {
			baseURL = "https://ntfy.sh"
		}
		var tokenPtr *string
		if token, hasToken := configString(values, "token"); hasToken {
			tokenPtr = &token
		}
		return NewNtfy(baseURL, topic, tokenPtr), true
	case models.ChannelPushover:
		token, hasToken := configString(values, "token")
		user, hasUser := configString(values, "user")
		if !hasToken || !hasUser {
			return missing("token/user")
		}
		return NewPushover(token, user), true
	case models.ChannelEmail:
		to, ok := configString(values, "to")
		if !ok {
			return missing("to")
		}
		return NewEmail(smtp, to), true
	}
	return nil, false
}
