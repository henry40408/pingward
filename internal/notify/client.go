package notify

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"time"

	"github.com/henry40408/pingward/internal/db"
)

const (
	httpTimeout  = 10 * time.Second
	maxRedirects = 10
)

// Error is a failed delivery. Its text is stored in the notifications table.
type Error struct{ Message string }

func (e *Error) Error() string { return "notify failed: " + e.Message }

func newError(format string, args ...any) *Error {
	return &Error{Message: fmt.Sprintf(format, args...)}
}

// Notifier sends one event to one channel.
type Notifier interface {
	Send(ctx context.Context, ev Event) error
}

var errTooManyRedirects = errors.New("too many redirects")

// newHTTPClient bounds the whole exchange so a hung endpoint cannot block
// delivery forever.
func newHTTPClient() *http.Client {
	return &http.Client{
		Timeout: httpTimeout,
		CheckRedirect: func(_ *http.Request, via []*http.Request) error {
			if len(via) >= maxRedirects {
				return errTooManyRedirects
			}
			return nil
		},
	}
}

// classifyTransportError names the failure only: the underlying error embeds
// the request URL, which for Telegram holds the bot token.
func classifyTransportError(err error) *Error {
	var netErr net.Error
	var opErr *net.OpError
	switch {
	case errors.Is(err, context.DeadlineExceeded), errors.As(err, &netErr) && netErr.Timeout():
		return &Error{Message: "request timed out"}
	case errors.As(err, &opErr) && opErr.Op == "dial":
		return &Error{Message: "connection failed"}
	case errors.Is(err, errTooManyRedirects):
		return &Error{Message: "too many redirects"}
	case errors.Is(err, io.ErrUnexpectedEOF):
		return &Error{Message: "invalid response"}
	}
	return &Error{Message: "request failed"}
}

// request is an outgoing POST.
type request struct {
	url         string
	contentType string
	headers     [][2]string
	body        []byte
}

// post sends req and treats any non-2xx status as a failure.
func post(ctx context.Context, client *http.Client, req request) error {
	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, req.url, bytes.NewReader(req.body))
	if err != nil {
		return &Error{Message: "request failed"}
	}
	if req.contentType != "" {
		httpReq.Header.Set("Content-Type", req.contentType)
	}
	for _, header := range req.headers {
		httpReq.Header.Set(header[0], header[1])
	}
	resp, err := client.Do(httpReq)
	if err != nil {
		return classifyTransportError(err)
	}
	defer func() { _ = resp.Body.Close() }()
	_, _ = io.Copy(io.Discard, io.LimitReader(resp.Body, 4096))
	if resp.StatusCode >= 200 && resp.StatusCode < 300 {
		return nil
	}
	return newError("status %s", statusLine(resp.StatusCode))
}

// statusLine is "401 Unauthorized", the way the Rust client printed it.
func statusLine(code int) string {
	reason := http.StatusText(code)
	if reason == "" {
		reason = "<unknown status code>"
	}
	return fmt.Sprintf("%d %s", code, reason)
}

// jsonRequest encodes payload without HTML escaping, like serde_json. The map
// keys come out sorted, matching serde_json without preserve_order.
func jsonRequest(url string, payload map[string]any) (request, error) {
	var buf bytes.Buffer
	encoder := json.NewEncoder(&buf)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(payload); err != nil {
		return request{}, err
	}
	body := bytes.TrimSuffix(buf.Bytes(), []byte("\n"))
	body = bytes.ReplaceAll(body, []byte(` `), []byte(" "))
	body = bytes.ReplaceAll(body, []byte(` `), []byte(" "))
	return request{url: url, contentType: "application/json", body: body}, nil
}

func trimTrailingSlashes(s string) string { return strings.TrimRight(s, "/") }

func rfc3339(t time.Time) string { return db.FormatTime(t) }
