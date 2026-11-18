package server_test

import (
	"net/http"
	"strings"
	"testing"

	"github.com/henry40408/pingward/internal/models"
)

// The ping router sits outside the web middleware: no session, no CSRF token,
// and the UUID in the path is the only credential.
func TestPingRouterNeedsNoSessionOrCSRFAndMovesTheCheck(t *testing.T) {
	a := newApp(t, nil)
	ownerID := a.addUser(t, "alice", false)
	a.seed(t, ownerID, "svc")
	res := a.client.Do("POST", "/ping/uuid-svc", strings.NewReader("hello"), nil)
	wantStatus(t, res, http.StatusOK)
	if res.Header.Get("Content-Security-Policy") != "" || res.Header.Get("Set-Cookie") != "" {
		t.Fatalf("a ping must not get web headers or a session: %v", res.Header)
	}
	check, err := a.store.FindCheckByUUID(t.Context(), "uuid-svc")
	if err != nil || check == nil {
		t.Fatalf("check: %v %v", check, err)
	}
	if check.Status != models.StatusUp || check.NextDueAt == nil {
		t.Fatalf("status %s, next due %v: the scheduler's due time must be wired in", check.Status, check.NextDueAt)
	}
}

func TestPingRouterAnswers404ForAnUnknownUUID(t *testing.T) {
	a := newApp(t, nil)
	wantStatus(t, a.client.Get("/ping/does-not-exist"), http.StatusNotFound)
}
