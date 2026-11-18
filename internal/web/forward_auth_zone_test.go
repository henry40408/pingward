package web

import (
	"log/slog"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/testutil"
)

// A link-local proxy peer carries an IPv6 zone in its RemoteAddr. Without
// stripping it the peer is not matched against the trusted pattern, which
// would silently turn forward auth off for that proxy.
func TestForwardAuthTrustsALinkLocalProxyDespiteItsZone(t *testing.T) {
	cfg := testutil.TestConfig(map[string]string{
		"PINGWARD_FORWARD_AUTH_HEADER": "X-Forwarded-User", "PINGWARD_TRUSTED_PROXIES": "fe80::/10",
	})
	deps := NewDeps(testutil.NewStore(t), cfg, nil2Logger(), time.Now)
	reached := false
	handler := deps.ForwardAuthSession(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { reached = true }))

	request := httptest.NewRequest("GET", "/", nil)
	request.RemoteAddr = "[fe80::1%eth0]:4321"
	request.Header.Set("X-Forwarded-User", "zoned")
	recorder := httptest.NewRecorder()
	handler.ServeHTTP(recorder, request)

	user, err := deps.Store.FindUserByUsername(t.Context(), "zoned")
	if err != nil || user == nil {
		t.Fatalf("a zoned trusted peer must provision the user: %v %v", user, err)
	}
	if !reached || recorder.Header().Get("Set-Cookie") == "" {
		t.Fatal("the session cookie must be set and the request passed on")
	}
}

// The zone must not make an untrusted peer trusted either.
func TestForwardAuthStillRefusesAZonedPeerOutsideThePattern(t *testing.T) {
	cfg := testutil.TestConfig(map[string]string{
		"PINGWARD_FORWARD_AUTH_HEADER": "X-Forwarded-User", "PINGWARD_TRUSTED_PROXIES": "fe80::/10",
	})
	deps := NewDeps(testutil.NewStore(t), cfg, nil2Logger(), time.Now)
	handler := deps.ForwardAuthSession(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
	request := httptest.NewRequest("GET", "/", nil)
	request.RemoteAddr = "[2001:db8::1%eth0]:4321"
	request.Header.Set("X-Forwarded-User", "intruder")
	handler.ServeHTTP(httptest.NewRecorder(), request)
	if user, _ := deps.Store.FindUserByUsername(t.Context(), "intruder"); user != nil {
		t.Fatal("a peer outside the trusted pattern must not provision a user")
	}
}

func nil2Logger() *slog.Logger { return slog.New(slog.DiscardHandler) }
