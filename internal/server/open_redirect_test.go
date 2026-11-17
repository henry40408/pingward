package server_test

import (
	"net/http"
	"net/url"
	"strings"
	"testing"

	"github.com/henry40408/pingward/internal/testutil"
	"github.com/henry40408/pingward/internal/web"
)

// Every web route, hit with attacker-chosen redirect hints in the query and
// the form, may only ever redirect to a local path. The Rust build has no
// "next" parameter at all; this keeps it that way when handlers are added.
var redirectHints = []string{"next", "redirect", "redirect_to", "return_to", "return", "url", "continue", "target", "dest"}

func isLocalPath(location string) bool {
	return strings.HasPrefix(location, "/") && !strings.HasPrefix(location, "//") && !strings.HasPrefix(location, `/\`)
}

func TestNoRouteRedirectsToAnAttackerChosenTarget(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	token := testutil.NewestSessionCSRF(t, a.store)
	evil := []string{"https://evil.example/", "//evil.example/", `/\evil.example`, "javascript:alert(1)"}

	for _, route := range web.Routes(a.deps) {
		if route.Pattern == "/logout" {
			continue // ends the session; covered by the logout tests
		}
		path := strings.NewReplacer("{id}", "1", "{uuid}", "1").Replace(route.Pattern)
		for _, target := range evil {
			query, form := url.Values{}, url.Values{"_csrf": {token}, "name": {"x"}}
			for _, hint := range redirectHints {
				query.Set(hint, target)
				form.Set(hint, target)
			}
			var res *testutil.Response
			if route.Method == http.MethodGet {
				res = a.client.Get(path + "?" + query.Encode())
			} else {
				res = a.client.PostRaw(path+"?"+query.Encode(), form.Encode(), nil)
			}
			if location := res.Header.Get("Location"); location != "" && !isLocalPath(location) {
				t.Errorf("%s %s with %q redirected to %q", route.Method, path, target, location)
			}
		}
	}
}

func TestLogoutNeverRedirectsToARequestSuppliedTarget(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	token := testutil.NewestSessionCSRF(t, a.store)
	res := a.client.PostForm("/logout?next=https://evil.example/", url.Values{"_csrf": {token}, "next": {"https://evil.example/"}})
	wantStatus(t, res, http.StatusSeeOther)
	if got := res.Header.Get("Location"); got != "/login" {
		t.Fatalf("Location = %q, want /login", got)
	}
}
