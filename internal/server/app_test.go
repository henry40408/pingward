package server_test

import (
	"net/http"
	"net/url"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/server"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
	"github.com/henry40408/pingward/internal/web"
)

// app is a server over a throwaway SQLite database.
type app struct {
	client *testutil.Client
	store  *store.Store
	deps   *web.Deps
	logs   *testutil.LockedBuffer
}

func newApp(t *testing.T, env map[string]string, extraRoutes ...web.Route) *app {
	t.Helper()
	st := testutil.NewStore(t)
	logger, logs := testutil.JSONLogger()
	deps := web.NewDeps(st, testutil.TestConfig(env), logger, time.Now)
	handler := server.New(deps, append(web.Routes(deps), extraRoutes...), server.Mounts(deps))
	return &app{client: testutil.NewClient(t, handler), store: st, deps: deps, logs: logs}
}

const testPassword = "correct-horse-battery-staple"

func (a *app) addUser(t *testing.T, username string, isAdmin bool) int64 {
	t.Helper()
	hash, err := auth.HashPassword(testPassword)
	if err != nil {
		t.Fatal(err)
	}
	id, err := a.store.CreateUser(t.Context(), username, &hash, isAdmin, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return id
}

// login signs username in and leaves the session cookie in the client's jar.
func (a *app) login(t *testing.T, username string) {
	t.Helper()
	csrf := testutil.AnonymousCSRF(t, a.client, "/login")
	res := a.client.PostForm("/login", url.Values{"_csrf": {csrf}, "username": {username}, "password": {testPassword}})
	if res.StatusCode != http.StatusSeeOther || res.Header.Get("Location") != "/" {
		t.Fatalf("login failed: %d %s", res.StatusCode, res.Body)
	}
}

func (a *app) loggedInAdmin(t *testing.T) {
	t.Helper()
	a.addUser(t, "admin", true)
	a.login(t, "admin")
}

func wantStatus(t *testing.T, res *testutil.Response, want int) {
	t.Helper()
	if res.StatusCode != want {
		t.Fatalf("status = %d, want %d; body: %s", res.StatusCode, want, res.Body)
	}
}

func newProjectForm() url.Values {
	return url.Values{"name": {"web"}, "description": {""}, "scan_interval_secs": {""}, "nag_interval_secs": {""}}
}

func newHandler(deps *web.Deps) http.Handler {
	return server.New(deps, web.Routes(deps), server.Mounts(deps))
}

var setsOwnHSTS = web.Route{Method: "GET", Pattern: "/own-hsts", Handler: func(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Strict-Transport-Security", "max-age=1; includeSubDomains")
}}
