package server_test

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/testutil"
	"github.com/henry40408/pingward/internal/web"
)

// ownerBodies gives each parameterised route a complete form: form parsing
// runs before the ownership lookup, so a short body would fail there instead
// of reaching the 404 this test is about.
func ownerBodies() map[string]url.Values {
	project := url.Values{"name": {"x"}, "description": {""}, "scan_interval_secs": {""}, "nag_interval_secs": {""}}
	check := url.Values{
		"name": {"x"}, "description": {""}, "schedule_kind": {"period"}, "period_secs": {"60"}, "cron_expr": {""},
		"grace_secs": {"30"}, "timezone": {"UTC"}, "scan_interval_secs": {""}, "max_runtime_secs": {""}, "nag_interval_secs": {""},
	}
	return map[string]url.Values{
		"POST /projects/{id}":           project,
		"POST /projects/{pid}/checks":   check,
		"POST /checks/{id}":             check,
		"POST /projects/{pid}/channels": {"name": {"x"}, "kind": {"webhook"}},
		"POST /channels/{id}":           {"_": {""}},
		"POST /checks/{id}/channels":    {"_": {""}},
	}
}

// statusOf sends the request and returns the status without reading the body,
// because /checks/{id}/events streams until the client goes away.
func statusOf(t *testing.T, client *testutil.Client, method, path string, form url.Values, header http.Header) int {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, method, client.Server.URL+path, strings.NewReader(form.Encode()))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	for name, values := range header {
		req.Header[name] = values
	}
	res, err := client.HTTP.Do(req)
	if err != nil {
		t.Fatalf("%s %s: %v", method, path, err)
	}
	_ = res.Body.Close()
	return res.StatusCode
}

// The route list comes from web.Routes, so a new owner route that skips the
// ownership lookup fails here. The owner must not get 404, or the member's 404
// could mean the id never existed.
func TestMemberCannotReachAnotherUsersResourceOnAnyWebRoute(t *testing.T) {
	member := newApp(t, nil)
	ownerID := member.addUser(t, "alice", false)
	member.addUser(t, "mallory", false)
	owner := &app{client: testutil.NewClient(t, newHandler(member.deps)), store: member.store, deps: member.deps, logs: member.logs}
	owner.login(t, "alice")
	ownerHeader := owner.csrfHeader()
	member.login(t, "mallory")
	memberHeader := member.csrfHeader()

	var routes []web.Route
	for _, route := range web.Routes(member.deps) {
		if (strings.Contains(route.Pattern, "{id}") || strings.Contains(route.Pattern, "{pid}")) && !strings.HasPrefix(route.Pattern, "/admin") && !strings.HasPrefix(route.Pattern, "/account") {
			routes = append(routes, route)
		}
	}
	if len(routes) < 15 {
		t.Fatalf("only %d parameterised owner routes, the filter is probably wrong", len(routes))
	}
	bodies := ownerBodies()
	for i, route := range routes {
		f := member.seed(t, ownerID, fmt.Sprintf("alice-%d", i))
		path := strings.NewReplacer("{pid}", itoa(f.projectID), "{id}", itoa(idFor(route.Pattern, f))).Replace(route.Pattern)
		form := bodies[route.Method+" "+route.Pattern]
		if got := statusOf(t, member.client, route.Method, path, form, memberHeader); got != http.StatusNotFound {
			t.Errorf("%s %s as non-owner: %d, want 404", route.Method, path, got)
		}
		// B goes first: the owner's request may delete the resource.
		if got := statusOf(t, owner.client, route.Method, path, form, ownerHeader); got == http.StatusNotFound {
			t.Errorf("%s %s: the owner got 404 too, so the non-owner's 404 proves nothing", route.Method, path)
		}
	}
}

func idFor(pattern string, f fixture) int64 {
	switch {
	case strings.HasPrefix(pattern, "/checks/"):
		return f.checkID
	case strings.HasPrefix(pattern, "/channels/"):
		return f.channelID
	}
	return f.projectID
}
