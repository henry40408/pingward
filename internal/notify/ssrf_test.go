package notify

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/henry40408/pingward/internal/models"
)

// Pins the SSRF posture inherited from the Rust build: channel URLs are
// trusted configuration, so loopback targets and redirects to them are
// followed, exactly as before. Restricting them is a deliberate product
// decision (待 Opus 審查); when it is made, these tests flip to expect refusal
// and name the new rule. A silent change in either direction fails here.
func TestWebhookReachesALoopbackTargetLikeTheRustBuild(t *testing.T) {
	internal := newSink(t, 200)
	if err := NewWebhook(internal.URL+"/admin").Send(bg, simpleEvent("x", models.EventDown)); err != nil {
		t.Fatal(err)
	}
	if got := internal.only(t); got.method != http.MethodPost || got.path != "/admin" {
		t.Fatalf("got %+v", got)
	}
}

func TestWebhookFollowsARedirectToAnInternalHost(t *testing.T) {
	internal := newSink(t, 200)
	redirector := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, internal.URL+"/metadata", http.StatusTemporaryRedirect)
	}))
	t.Cleanup(redirector.Close)
	if err := NewWebhook(redirector.URL).Send(bg, simpleEvent("x", models.EventDown)); err != nil {
		t.Fatal(err)
	}
	if got := internal.only(t); got.path != "/metadata" {
		t.Fatalf("the redirect target was not reached: %+v", got)
	}
}

// Whatever the posture, the redirect chain stays bounded and the URL (which
// may carry a token) never reaches the error text.
func TestRedirectChainIsBoundedAndErrorsDoNotLeakTheURL(t *testing.T) {
	loop := newLoopingRedirector(t)
	err := NewWebhook(loop.URL+"/token-in-path").Send(bg, simpleEvent("x", models.EventDown))
	if err == nil {
		t.Fatal("a redirect loop must fail")
	}
	if got := err.Error(); got != "notify failed: too many redirects" {
		t.Fatalf("error = %q", got)
	}
}
