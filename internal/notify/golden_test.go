package notify

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

// rustWebhookBodies are the bodies a plain HTTP sink received from the Rust
// v0.12.0 image's webhook notifier. [0] is the test event.
var rustWebhookBodies = []string{
	`{"at":"2026-11-17T15:25:50.024938711+00:00","cause":null,"check":"ch-webhook","check_id":0,"event":"test","exit_code":null,"last_ping_at":null,"project":"Alpha","project_id":1,"schedule":null,"text":"🔔 pingward test notification\nChannel \"ch-webhook\" · project Alpha\n2026-11-17 23:25 CST","timezone":"Asia/Taipei","url":null}`,
	`{"at":"2026-11-17T15:26:08.323303007+00:00","cause":"overdue","check":"flaky","check_id":6,"event":"down","exit_code":null,"last_ping_at":null,"project":"Alpha","project_id":1,"schedule":"every 2s (grace 1s)","text":"🔴 DOWN — flaky\nProject: Alpha · every 2s (grace 1s)\nNo ping received yet\nhttp://localhost:18080/checks/6","timezone":"Asia/Taipei","url":"http://localhost:18080/checks/6"}`,
	`{"at":"2026-11-17T15:26:18.101237878+00:00","cause":null,"check":"flaky","check_id":6,"event":"up","exit_code":null,"last_ping_at":null,"project":"Alpha","project_id":1,"schedule":"every 2s (grace 1s)","text":"🟢 UP — flaky\nProject: Alpha · every 2s (grace 1s)\nRecovered\nhttp://localhost:18080/checks/6","timezone":"Asia/Taipei","url":"http://localhost:18080/checks/6"}`,
	`{"at":"2026-11-17T15:26:21.432158577+00:00","cause":"overdue","check":"flaky","check_id":6,"event":"down","exit_code":null,"last_ping_at":"2026-11-17T15:26:18.101237878+00:00","project":"Alpha","project_id":1,"schedule":"every 2s (grace 1s)","text":"🔴 DOWN — flaky\nProject: Alpha · every 2s (grace 1s)\nNo ping since 2026-11-17 23:26 CST (3s ago)\nhttp://localhost:18080/checks/6","timezone":"Asia/Taipei","url":"http://localhost:18080/checks/6"}`,
	`{"at":"2026-11-17T15:26:24.142082667+00:00","cause":null,"check":"flaky","check_id":6,"event":"up","exit_code":null,"last_ping_at":"2026-11-17T15:26:18.101237878+00:00","project":"Alpha","project_id":1,"schedule":"every 2s (grace 1s)","text":"🟢 UP — flaky\nProject: Alpha · every 2s (grace 1s)\nRecovered; previous ping 2026-11-17 23:26 CST (6s ago)\nhttp://localhost:18080/checks/6","timezone":"Asia/Taipei","url":"http://localhost:18080/checks/6"}`,
	`{"at":"2026-11-17T15:26:27.179788+00:00","cause":"failed","check":"flaky","check_id":6,"event":"down","exit_code":null,"last_ping_at":"2026-11-17T15:26:24.142082667+00:00","project":"Alpha","project_id":1,"schedule":"every 2s (grace 1s)","text":"🔴 DOWN — flaky\nProject: Alpha · every 2s (grace 1s)\nFail ping received\nhttp://localhost:18080/checks/6","timezone":"Asia/Taipei","url":"http://localhost:18080/checks/6"}`,
	`{"at":"2026-11-17T15:26:45.615476664+00:00","cause":"overrun","check":"slow-job","check_id":7,"event":"down","exit_code":null,"last_ping_at":null,"project":"Alpha","project_id":1,"schedule":"every 1h (grace 1h)","text":"🔴 DOWN — slow-job\nProject: Alpha · every 1h (grace 1h)\nRun started 2026-11-17 23:26 CST (2s ago) and exceeded its 2s max runtime\nhttp://localhost:18080/checks/7","timezone":"Asia/Taipei","url":"http://localhost:18080/checks/7"}`,
	`{"at":"2026-11-17T15:26:11.352728288+00:00","cause":null,"check":"flaky","check_id":6,"event":"reminder","exit_code":null,"last_ping_at":null,"project":"Alpha","project_id":1,"schedule":"every 2s (grace 1s)","text":"🔴 STILL DOWN — flaky\nProject: Alpha · every 2s (grace 1s)\nNever pinged\nhttp://localhost:18080/checks/6","timezone":"Asia/Taipei","url":"http://localhost:18080/checks/6"}`,
}

// rustSlackTestBody is what the Rust slack notifier sent for the test event.
const rustSlackTestBody = `{"text":"🔔 pingward test notification\nChannel \"ch-slack\" · project Alpha\n2026-11-17 23:25 CST"}`

// rustNtfyTestRequest is what the Rust ntfy notifier sent for the test event.
var rustNtfyTestRequest = struct {
	body    string
	headers map[string]string
}{
	body: `🔔 pingward test notification
Channel "ch-ntfy" · project Alpha
2026-11-17 23:25 CST`,
	headers: map[string]string{
		"title":         "pingward: test notification for \"Alpha/ch-ntfy\"",
		"priority":      "default",
		"tags":          "bell",
		"authorization": "Bearer tk",
	},
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
	if got, want := server.only(t).body, rustWebhookBodies[0]; got != want {
		t.Fatalf("got  %s\nwant %s", got, want)
	}
}

func TestSlackTestEventMatchesTheRustCapture(t *testing.T) {
	server := newSink(t, 200)
	if err := NewSlack(server.URL+"/slack").Send(bg, capturedTestEvent("ch-slack")); err != nil {
		t.Fatal(err)
	}
	if got := server.only(t).body; got != rustSlackTestBody {
		t.Fatalf("got  %s\nwant %s", got, rustSlackTestBody)
	}
}

func TestNtfyTestEventMatchesTheRustCapture(t *testing.T) {
	server := newSink(t, 200)
	if err := NewNtfy(server.URL, "pw", ptr("tk")).Send(bg, capturedTestEvent("ch-ntfy")); err != nil {
		t.Fatal(err)
	}
	got, want := server.only(t), rustNtfyTestRequest
	if got.body != want.body {
		t.Fatalf("got  %s\nwant %s", got.body, want.body)
	}
	for name, value := range want.headers {
		if got.header.Get(name) != value {
			t.Errorf("header %s = %q, want %q", name, got.header.Get(name), value)
		}
	}
}

// Every captured webhook body must survive a decode and re-encode unchanged:
// sorted keys, no spaces, no HTML escaping, explicit nulls.
func TestWebhookEncodingReproducesEveryCapturedBody(t *testing.T) {
	for _, body := range rustWebhookBodies {
		var payload map[string]any
		if err := json.Unmarshal([]byte(body), &payload); err != nil {
			t.Fatal(err)
		}
		req, err := jsonRequest("http://x", payload)
		if err != nil || string(req.body) != body {
			t.Fatalf("got  %s\nwant %s (%v)", req.body, body, err)
		}
	}
}
