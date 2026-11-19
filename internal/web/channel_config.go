package web

import (
	"bytes"
	"encoding/json"
	"strings"

	"github.com/henry40408/pingward/internal/models"
)

// ChannelInput is a channel form as submitted. Blank fields keep the stored
// value on edit, except NtfyTokenClear, which exists so a stored token can be removed.
type ChannelInput struct {
	Name, Kind                           string
	WebhookURL, SlackURL                 string
	TelegramToken, TelegramChatID        string
	NtfyBaseURL, NtfyTopic, NtfyToken    string
	NtfyTokenClear                       bool
	PushoverToken, PushoverUser, EmailTo string
}

// storedConfig is the config of the channel being edited; empty when creating.
type storedConfig map[string]any

// parseStoredConfig counts an unparseable stored config as empty (create
// rules), not an error.
func parseStoredConfig(existing *models.Channel) storedConfig {
	if existing == nil {
		return nil
	}
	var values map[string]any
	if json.Unmarshal([]byte(existing.ConfigJSON), &values) != nil {
		return nil
	}
	return values
}

// merged is submitted trimmed, or whatever is stored under key when blank.
func (c storedConfig) merged(submitted, key string) string {
	if trimmed := trimRustSpace(submitted); trimmed != "" {
		return trimmed
	}
	stored, _ := c[key].(string)
	return stored
}

// compactJSON is serde_json::to_string: sorted keys (map order), no HTML escaping.
func compactJSON(values map[string]string) string {
	var out bytes.Buffer
	encoder := json.NewEncoder(&out)
	encoder.SetEscapeHTML(false)
	_ = encoder.Encode(values)
	return strings.TrimSuffix(out.String(), "\n")
}

func parseNewChannelKind(raw string) (models.ChannelKind, string) {
	kind := trimRustSpace(raw)
	if kind == "" {
		return "", "A channel kind is required."
	}
	switch parsed := models.ChannelKind(kind); parsed {
	case models.ChannelWebhook, models.ChannelTelegram, models.ChannelSlack,
		models.ChannelNtfy, models.ChannelPushover, models.ChannelEmail:
		return parsed, ""
	}
	return "", "Unknown channel kind."
}

// ValidateChannelUpdate checks a channel form. existing is nil when creating;
// otherwise its kind always wins, since the kind is immutable. The refusal is
// empty on success.
func ValidateChannelUpdate(in ChannelInput, existing *models.Channel) (kind models.ChannelKind, name, configJSON, refusal string) {
	name = trimRustSpace(in.Name)
	if name == "" {
		if existing == nil {
			return "", "", "", "A channel name is required."
		}
		name = existing.Name
	}
	if existing != nil {
		kind = existing.Kind
	} else if kind, refusal = parseNewChannelKind(in.Kind); refusal != "" {
		return "", "", "", refusal
	}
	configJSON, refusal = channelConfig(kind, in, parseStoredConfig(existing))
	if refusal != "" {
		return "", "", "", refusal
	}
	return kind, name, configJSON, ""
}

// ValidateChannel is the create-side check, shared with the API.
func ValidateChannel(in ChannelInput) (kind models.ChannelKind, name, configJSON, refusal string) {
	return ValidateChannelUpdate(in, nil)
}

func channelConfig(kind models.ChannelKind, in ChannelInput, stored storedConfig) (string, string) {
	switch kind {
	case models.ChannelWebhook:
		url := stored.merged(in.WebhookURL, "url")
		if url == "" {
			return "", "A webhook URL is required."
		}
		return compactJSON(map[string]string{"url": url}), ""
	case models.ChannelSlack:
		url := stored.merged(in.SlackURL, "url")
		if url == "" {
			return "", "A Slack incoming-webhook URL is required."
		}
		return compactJSON(map[string]string{"url": url}), ""
	case models.ChannelTelegram:
		token, chatID := stored.merged(in.TelegramToken, "token"), stored.merged(in.TelegramChatID, "chat_id")
		if token == "" || chatID == "" {
			return "", "Telegram requires both a bot token and a chat ID."
		}
		return compactJSON(map[string]string{"token": token, "chat_id": chatID}), ""
	case models.ChannelNtfy:
		return ntfyConfig(in, stored)
	case models.ChannelPushover:
		token, user := stored.merged(in.PushoverToken, "token"), stored.merged(in.PushoverUser, "user")
		if token == "" || user == "" {
			return "", "Pushover requires both an application token and a user key."
		}
		return compactJSON(map[string]string{"token": token, "user": user}), ""
	case models.ChannelEmail:
		to := stored.merged(in.EmailTo, "to")
		if to == "" {
			return "", "An email recipient address is required."
		}
		return compactJSON(map[string]string{"to": to}), ""
	}
	return "", "Unknown channel kind."
}

func ntfyConfig(in ChannelInput, stored storedConfig) (string, string) {
	topic := stored.merged(in.NtfyTopic, "topic")
	if topic == "" {
		return "", "A topic is required for ntfy."
	}
	baseURL := stored.merged(in.NtfyBaseURL, "base_url")
	if baseURL == "" {
		baseURL = "https://ntfy.sh"
	}
	token := ""
	if !in.NtfyTokenClear {
		token = stored.merged(in.NtfyToken, "token")
	}
	return compactJSON(map[string]string{"base_url": baseURL, "topic": topic, "token": token}), ""
}
