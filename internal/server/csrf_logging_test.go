package server_test

import (
	"net/http"
	"net/url"
	"strings"
	"testing"

	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/secret"
	"github.com/henry40408/pingward/internal/testutil"
)

func loginForm(csrf string) url.Values {
	form := url.Values{"username": {"alice"}, "password": {testPassword}}
	if csrf != "" {
		form.Set("_csrf", csrf)
	}
	return form
}

// A presented-but-invalid token must warn, naming the session by its log
// handle, never the raw session id.
func TestMismatchedTokenWarnsAndNamesTheSessionByHandle(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "alice", false)
	a.client.ClearCookies()
	a.client.Get("/login")
	rawID, ok := secret.VerifySession([]byte(testutil.TestSecret), a.client.Cookie("pingward_session"))
	if !ok {
		t.Fatal("the anonymous-session layer signs its cookie")
	}
	// Well-formed hex, so this fails VerifyCSRF, not a decode.
	res := a.client.PostForm("/login", loginForm(strings.Repeat("00", 32)))
	wantStatus(t, res, http.StatusForbidden)

	events := a.logs.EventsNamed(t, "csrf.rejected")
	if len(events) != 1 {
		t.Fatalf("want one csrf.rejected event, got %v", events)
	}
	event := events[0]
	if event["reason"] != "token_mismatch" || event["level"] != "WARN" {
		t.Errorf("want WARN token_mismatch, got %v", event)
	}
	if event["handle"] != auth.SessionLogHandle(rawID) {
		t.Errorf("handle = %v, want %s", event["handle"], auth.SessionLogHandle(rawID))
	}
	if strings.Contains(a.logs.String(), rawID) {
		t.Errorf("the raw session id leaked into the log: %s", a.logs.String())
	}
}

// Tokenless posts are what scanners send, unthrottled; warning on them would
// drown the event.
func TestTokenlessPostIsRecordedButNotWarned(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "alice", false)
	testutil.AnonymousCSRF(t, a.client, "/login")
	wantStatus(t, a.client.PostForm("/login", loginForm("")), http.StatusForbidden)

	events := a.logs.EventsNamed(t, "csrf.rejected")
	if len(events) != 1 || events[0]["reason"] != "token_missing" {
		t.Fatalf("want one token_missing event, got %v", events)
	}
	if events[0]["level"] != "DEBUG" {
		t.Errorf("a tokenless POST must log at debug, got %v", events[0]["level"])
	}
	for _, event := range a.logs.Events(t) {
		if event["level"] == "WARN" {
			t.Errorf("a tokenless POST must not warn: %v", event)
		}
	}
}

func TestHeaderMismatchWarns(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	header := http.Header{"X-Csrf-Token": {"nope"}}
	wantStatus(t, a.client.PostRaw("/projects", newProjectForm().Encode(), header), http.StatusForbidden)
	events := a.logs.EventsNamed(t, "csrf.rejected")
	if len(events) != 1 || events[0]["reason"] != "header_mismatch" || events[0]["level"] != "WARN" {
		t.Fatalf("want one WARN header_mismatch, got %v", events)
	}
}
