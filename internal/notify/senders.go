package notify

import (
	"context"
	"net/http"
	"net/url"
	"strings"
	"unicode"

	"github.com/henry40408/pingward/internal/models"
)

type webhookNotifier struct {
	url    string
	client *http.Client
}

// NewWebhook posts the event as JSON. The original four keys stay verbatim;
// everything else is additive.
func NewWebhook(url string) Notifier { return &webhookNotifier{url: url, client: newHTTPClient()} }

func (n *webhookNotifier) Send(ctx context.Context, ev Event) error {
	detail := ev.Detail
	var cause *string
	var exitCode *int64
	if detail.Cause != nil {
		name := string(detail.Cause.Kind)
		cause = &name
		if detail.Cause.Kind == CauseFailed {
			exitCode = detail.Cause.ExitCode
		}
	}
	var lastPingAt *string
	if detail.LastPingAt != nil {
		text := rfc3339(*detail.LastPingAt)
		lastPingAt = &text
	}
	req, err := jsonRequest(n.url, map[string]any{
		"check":        ev.CheckName,
		"event":        string(ev.Kind),
		"at":           rfc3339(ev.At),
		"project_id":   ev.ProjectID,
		"check_id":     ev.CheckID,
		"project":      detail.ProjectName,
		"url":          detail.URL,
		"schedule":     detail.Schedule,
		"timezone":     detail.Timezone,
		"last_ping_at": lastPingAt,
		"cause":        cause,
		"exit_code":    exitCode,
		"text":         EventText(ev),
	})
	if err != nil {
		return &Error{Message: "request failed"}
	}
	return post(ctx, n.client, req)
}

type telegramNotifier struct {
	token, chatID, baseURL string
	client                 *http.Client
}

// NewTelegram posts to the Bot API at https://api.telegram.org.
func NewTelegram(token, chatID string) Notifier {
	return newTelegram(token, chatID, "https://api.telegram.org")
}

// newTelegram takes the base URL so tests can aim at a local server.
func newTelegram(token, chatID, baseURL string) *telegramNotifier {
	return &telegramNotifier{token: token, chatID: chatID, baseURL: trimTrailingSlashes(baseURL), client: newHTTPClient()}
}

func (n *telegramNotifier) Send(ctx context.Context, ev Event) error {
	req, err := jsonRequest(n.baseURL+"/bot"+n.token+"/sendMessage",
		map[string]any{"chat_id": n.chatID, "text": EventText(ev)})
	if err != nil {
		return &Error{Message: "request failed"}
	}
	return post(ctx, n.client, req)
}

type slackNotifier struct {
	url    string
	client *http.Client
}

// NewSlack posts to a Slack incoming webhook.
func NewSlack(url string) Notifier { return &slackNotifier{url: url, client: newHTTPClient()} }

func (n *slackNotifier) Send(ctx context.Context, ev Event) error {
	req, err := jsonRequest(n.url, map[string]any{"text": EventText(ev)})
	if err != nil {
		return &Error{Message: "request failed"}
	}
	return post(ctx, n.client, req)
}

type ntfyNotifier struct {
	baseURL, topic, token string
	client                *http.Client
}

// NewNtfy publishes to {baseURL}/{topic} with an optional bearer token.
func NewNtfy(baseURL, topic string, token *string) Notifier {
	n := &ntfyNotifier{baseURL: trimTrailingSlashes(baseURL), topic: topic, client: newHTTPClient()}
	if token != nil {
		n.token = *token
	}
	return n
}

func (n *ntfyNotifier) Send(ctx context.Context, ev Event) error {
	priority, tags := "default", "bell"
	switch ev.Kind {
	case models.EventDown, models.EventReminder:
		priority, tags = "high", "red_circle"
	case models.EventUp:
		priority, tags = "default", "green_circle"
	}
	req := request{
		url:     n.baseURL + "/" + n.topic,
		headers: [][2]string{{"Title", EventTitle(ev)}, {"Priority", priority}, {"Tags", tags}},
		body:    []byte(EventText(ev)),
	}
	// Click opens the check page; skipped unless header-safe, since an invalid
	// header value would abort the send.
	if ev.Detail.URL != nil && isHeaderSafeURL(*ev.Detail.URL) {
		req.headers = append(req.headers, [2]string{"Click", *ev.Detail.URL})
	}
	if n.token != "" {
		req.headers = append(req.headers, [2]string{"Authorization", "Bearer " + n.token})
	}
	return post(ctx, n.client, req)
}

func isHeaderSafeURL(u string) bool {
	return u != "" && !strings.ContainsFunc(u, func(r rune) bool { return unicode.IsControl(r) || unicode.IsSpace(r) })
}

type pushoverNotifier struct {
	token, user, baseURL string
	client               *http.Client
}

// NewPushover posts to the Pushover API at https://api.pushover.net.
func NewPushover(token, user string) Notifier {
	return newPushover(token, user, "https://api.pushover.net")
}

// newPushover takes the base URL so tests can aim at a local server.
func newPushover(token, user, baseURL string) *pushoverNotifier {
	return &pushoverNotifier{token: token, user: user, baseURL: trimTrailingSlashes(baseURL), client: newHTTPClient()}
}

func (n *pushoverNotifier) Send(ctx context.Context, ev Event) error {
	priority := "0"
	if ev.Kind == models.EventDown || ev.Kind == models.EventReminder {
		priority = "1"
	}
	fields := [][2]string{
		{"token", n.token}, {"user", n.user}, {"title", EventTitle(ev)},
		{"message", EventText(ev)}, {"priority", priority},
	}
	if ev.Detail.URL != nil && *ev.Detail.URL != "" {
		fields = append(fields, [2]string{"url", *ev.Detail.URL}, [2]string{"url_title", "Open in pingward"})
	}
	return post(ctx, n.client, request{
		url:         n.baseURL + "/1/messages.json",
		contentType: "application/x-www-form-urlencoded",
		body:        []byte(encodeForm(fields)),
	})
}

// encodeForm keeps the field order, which url.Values would sort.
func encodeForm(fields [][2]string) string {
	pairs := make([]string, len(fields))
	for i, field := range fields {
		pairs[i] = url.QueryEscape(field[0]) + "=" + url.QueryEscape(field[1])
	}
	return strings.Join(pairs, "&")
}
