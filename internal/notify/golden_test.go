package notify

import (
	"encoding/json"
	"os"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

type capturedRequest struct {
	Method  string            `json:"method"`
	Path    string            `json:"path"`
	Headers map[string]string `json:"headers"`
	Body    string            `json:"body"`
}

func loadCaptured(t *testing.T) []capturedRequest {
	t.Helper()
	raw, err := os.ReadFile("testdata/requests.json")
	if err != nil {
		t.Fatal(err)
	}
	var requests []capturedRequest
	if err := json.Unmarshal(raw, &requests); err != nil {
		t.Fatal(err)
	}
	return requests
}

// capturedTestEvent is the test event the Rust v0.12.0 image sent.
func capturedTestEvent(name string) Event {
	at := time.Date(2026, 11, 17, 15, 25, 50, 24938711, time.UTC)
	return Event{
		CheckName: name, Kind: models.EventTest, At: at, ProjectID: 1,
		Detail: EventDetail{ProjectName: ptr("Alpha"), Timezone: ptr("Asia/Taipei")},
	}
}

func TestWebhookTestEventMatchesTheRustCapture(t *testing.T) {
	server := newSink(t, 200)
	if err := NewWebhook(server.URL+"/hook").Send(bg, capturedTestEvent("ch-webhook")); err != nil {
		t.Fatal(err)
	}
	want := loadCaptured(t)[0]
	if got := server.only(t).body; got != want.Body {
		t.Fatalf("got  %s\nwant %s", got, want.Body)
	}
}

func TestSlackTestEventMatchesTheRustCapture(t *testing.T) {
	server := newSink(t, 200)
	if err := NewSlack(server.URL+"/slack").Send(bg, capturedTestEvent("ch-slack")); err != nil {
		t.Fatal(err)
	}
	want := loadCaptured(t)[1]
	if got := server.only(t).body; got != want.Body {
		t.Fatalf("got  %s\nwant %s", got, want.Body)
	}
}

func TestNtfyTestEventMatchesTheRustCapture(t *testing.T) {
	server := newSink(t, 200)
	if err := NewNtfy(server.URL, "pw", ptr("tk")).Send(bg, capturedTestEvent("ch-ntfy")); err != nil {
		t.Fatal(err)
	}
	want, got := loadCaptured(t)[2], server.only(t)
	if got.body != want.Body {
		t.Fatalf("got  %s\nwant %s", got.body, want.Body)
	}
	for name, value := range want.Headers {
		if got.header.Get(name) != value {
			t.Errorf("header %s = %q, want %q", name, got.header.Get(name), value)
		}
	}
}

// Every captured webhook body must survive a decode and re-encode unchanged:
// sorted keys, no spaces, no HTML escaping, explicit nulls.
func TestWebhookEncodingReproducesEveryCapturedBody(t *testing.T) {
	checked := 0
	for _, captured := range loadCaptured(t) {
		if captured.Path != "/hook" {
			continue
		}
		var payload map[string]any
		if err := json.Unmarshal([]byte(captured.Body), &payload); err != nil {
			t.Fatal(err)
		}
		req, err := jsonRequest("http://x", payload)
		if err != nil || string(req.body) != captured.Body {
			t.Fatalf("got  %s\nwant %s (%v)", req.body, captured.Body, err)
		}
		checked++
	}
	if checked == 0 {
		t.Fatal("no webhook captures")
	}
}
