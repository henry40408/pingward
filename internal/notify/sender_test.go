package notify

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

type received struct {
	method, path string
	header       http.Header
	body         string
}

// sink is an HTTP server that records requests and answers with a fixed status.
type sink struct {
	*httptest.Server
	mu       sync.Mutex
	requests []received
}

func newSink(t *testing.T, status int) *sink {
	t.Helper()
	s := &sink{}
	s.Server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		s.mu.Lock()
		s.requests = append(s.requests, received{r.Method, r.URL.Path, r.Header.Clone(), string(body)})
		s.mu.Unlock()
		w.WriteHeader(status)
	}))
	t.Cleanup(s.Close)
	return s
}

func (s *sink) only(t *testing.T) received {
	t.Helper()
	s.mu.Lock()
	defer s.mu.Unlock()
	if len(s.requests) != 1 {
		t.Fatalf("got %d requests, want 1", len(s.requests))
	}
	return s.requests[0]
}

func simpleEvent(name string, kind models.EventKind) Event {
	return Event{CheckID: 1, CheckName: name, Kind: kind, At: time.Now(), ProjectID: 1}
}

var bg = context.Background()

func TestTelegramPostsSendmessageJSON(t *testing.T) {
	server := newSink(t, 200)
	n := newTelegram("123:ABC", "999", server.URL)
	if err := n.Send(bg, simpleEvent("backup", models.EventDown)); err != nil {
		t.Fatal(err)
	}
	got := server.only(t)
	if got.method != "POST" || got.path != "/bot123:ABC/sendMessage" ||
		!strings.Contains(got.body, `"chat_id":"999"`) || !strings.Contains(got.body, "DOWN") {
		t.Fatalf("%+v", got)
	}
}

func TestTelegramReturnsErrOnHTTPError(t *testing.T) {
	err := newTelegram("bad", "1", newSink(t, 401).URL).Send(bg, simpleEvent("backup", models.EventDown))
	if err == nil || err.Error() != "notify failed: status 401 Unauthorized" {
		t.Fatalf("%v", err)
	}
}

func TestWebhookPostsJSON(t *testing.T) {
	server := newSink(t, 200)
	if err := NewWebhook(server.URL+"/hook").Send(bg, simpleEvent("backup", models.EventDown)); err != nil {
		t.Fatal(err)
	}
	got := server.only(t)
	if got.path != "/hook" || got.header.Get("Content-Type") != "application/json" {
		t.Fatalf("%+v", got)
	}
}

// The Rust test waited out the real 10s timeout; this shortens the client's.
func TestWebhookSendTimesOutOnHungEndpoint(t *testing.T) {
	release := make(chan struct{})
	hung := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { <-release }))
	t.Cleanup(func() { close(release); hung.Close() })
	n := NewWebhook(hung.URL + "/hook").(*webhookNotifier)
	n.client.Timeout = 200 * time.Millisecond
	start := time.Now()
	err := n.Send(bg, simpleEvent("backup", models.EventDown))
	if err == nil || err.Error() != "notify failed: request timed out" {
		t.Fatalf("%v", err)
	}
	if elapsed := time.Since(start); elapsed > 5*time.Second {
		t.Fatalf("took %v", elapsed)
	}
}

func TestSlackPostsTextJSON(t *testing.T) {
	server := newSink(t, 200)
	if err := NewSlack(server.URL+"/services/hook").Send(bg, simpleEvent("backup", models.EventUp)); err != nil {
		t.Fatal(err)
	}
	got := server.only(t)
	if got.path != "/services/hook" || !strings.Contains(got.body, `"text"`) || !strings.Contains(got.body, "UP") {
		t.Fatalf("%+v", got)
	}
}

func TestSlackReturnsErrOnHTTPError(t *testing.T) {
	if err := NewSlack(newSink(t, 500).URL+"/x").Send(bg, simpleEvent("backup", models.EventUp)); err == nil {
		t.Fatal("want error")
	}
}

func TestNtfyPostsBodyWithHeadersAndToken(t *testing.T) {
	server := newSink(t, 200)
	token := "tok"
	if err := NewNtfy(server.URL, "mytopic", &token).Send(bg, simpleEvent("backup", models.EventDown)); err != nil {
		t.Fatal(err)
	}
	got := server.only(t)
	if got.path != "/mytopic" || got.header.Get("Authorization") != "Bearer tok" ||
		got.header.Get("Priority") != "high" || got.header.Get("Tags") != "red_circle" {
		t.Fatalf("%+v", got)
	}
}

func TestNtfyWorksWithoutTokenAndErrorsOnHTTPError(t *testing.T) {
	server := newSink(t, 500)
	if err := NewNtfy(server.URL, "t", nil).Send(bg, simpleEvent("backup", models.EventUp)); err == nil {
		t.Fatal("want error")
	}
	if got := server.only(t); got.header.Get("Authorization") != "" {
		t.Fatalf("authorization sent without a token: %+v", got)
	}
}

func TestTelegramSendErrorDoesNotLeakBotToken(t *testing.T) {
	n := newTelegram("123456:SECRETTOKENVALUE", "1", "http://127.0.0.1:1")
	err := n.Send(bg, simpleEvent("backup", models.EventDown))
	if err == nil || strings.Contains(err.Error(), "SECRETTOKENVALUE") {
		t.Fatalf("bot token leaked into the error: %v", err)
	}
	if err.Error() != "notify failed: connection failed" {
		t.Fatalf("%v", err)
	}
}

func TestEventTitleStripsControlCharacters(t *testing.T) {
	title := EventTitle(simpleEvent("back\nup\tjob", models.EventDown))
	if strings.ContainsAny(title, "\n\t") || !strings.Contains(title, "back up job") {
		t.Fatalf("%q", title)
	}
}

func TestNtfySendSucceedsWithControlCharCheckName(t *testing.T) {
	server := newSink(t, 200)
	if err := NewNtfy(server.URL, "topic", nil).Send(bg, simpleEvent("nightly\nbackup", models.EventDown)); err != nil {
		t.Fatal(err)
	}
	server.only(t)
}

func TestPushoverPostsFormWithTokenAndUser(t *testing.T) {
	server := newSink(t, 200)
	if err := newPushover("apptok", "userkey", server.URL).Send(bg, simpleEvent("backup", models.EventDown)); err != nil {
		t.Fatal(err)
	}
	got := server.only(t)
	if got.path != "/1/messages.json" || !strings.Contains(got.body, "token=apptok") ||
		!strings.Contains(got.body, "user=userkey") || !strings.HasPrefix(got.body, "token=apptok&user=userkey&title=") {
		t.Fatalf("%+v", got)
	}
}

func TestPushoverReturnsErrOnHTTPError(t *testing.T) {
	if err := newPushover("bad", "bad", newSink(t, 400).URL).Send(bg, simpleEvent("backup", models.EventUp)); err == nil {
		t.Fatal("want error")
	}
}

func newLoopingRedirector(t *testing.T) *httptest.Server {
	t.Helper()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, r.URL.Path, http.StatusFound)
	}))
	t.Cleanup(server.Close)
	return server
}
