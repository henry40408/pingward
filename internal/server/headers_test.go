package server_test

import (
	"net/http"
	"strings"
	"testing"
)

// One route per router, so an "app-wide" header is checked on all of them.
var everyRouter = []string{"/login", "/ping/does-not-exist", "/healthz", "/assets/app.css", "/api/openapi.json"}

func TestOriginWideHeadersAreOnEveryRouter(t *testing.T) {
	a := newApp(t, nil)
	for _, route := range everyRouter {
		res := a.client.Get(route)
		want := map[string]string{
			"X-Content-Type-Options": "nosniff",
			"X-Frame-Options":        "DENY",
			"Referrer-Policy":        "same-origin",
		}
		for name, value := range want {
			if got := res.Header.Get(name); got != value {
				t.Errorf("%s %s = %q, want %q", route, name, got, value)
			}
		}
		if !strings.Contains(res.Header.Get("Permissions-Policy"), "geolocation=()") {
			t.Errorf("%s: Permissions-Policy = %q", route, res.Header.Get("Permissions-Policy"))
		}
	}
}

func TestBrowserUICarriesAScriptSrcSelfPolicy(t *testing.T) {
	a := newApp(t, nil)
	csp := a.client.Get("/login").Header.Get("Content-Security-Policy")
	if !strings.Contains(csp, "script-src 'self';") {
		t.Errorf("script-src 'self' missing: %s", csp)
	}
	for _, widened := range []string{"script-src 'self' 'unsafe-inline'", "nonce-"} {
		if strings.Contains(csp, widened) {
			t.Errorf("CSP must not contain %q: %s", widened, csp)
		}
	}
	for _, directive := range []string{
		"default-src 'self'", "object-src 'none'", "base-uri 'none'", "form-action 'self'",
		"frame-ancestors 'none'", "connect-src 'self'", "style-src 'self' 'unsafe-inline'",
	} {
		if !strings.Contains(csp, directive) {
			t.Errorf("missing %q: %s", directive, csp)
		}
	}
}

// Scalar loads from a CDN, so /api/docs skips the CSP rather than widening it
// for everyone. The same holds for every non-web router.
func TestOnlyWebRoutesCarryTheCSPAndNoStore(t *testing.T) {
	a := newApp(t, nil)
	for _, route := range []string{"/ping/does-not-exist", "/healthz", "/assets/app.css", "/api/v1/projects"} {
		res := a.client.Get(route)
		if res.Header.Get("Content-Security-Policy") != "" {
			t.Errorf("%s must not inherit the browser UI's CSP", route)
		}
		if res.Header.Get("Cache-Control") == "no-store" {
			t.Errorf("%s must not get the web no-store default", route)
		}
	}
	if got := a.client.Get("/login").Header.Get("Cache-Control"); got != "no-store" {
		t.Errorf("/login Cache-Control = %q, want no-store", got)
	}
}

func TestScriptsAreJavaScriptAndDoNotShadowTheStylesheet(t *testing.T) {
	a := newApp(t, nil)
	for _, script := range []string{"/assets/app.js", "/assets/theme-init.js"} {
		res := a.client.Get(script)
		wantStatus(t, res, http.StatusOK)
		if !strings.HasPrefix(res.Header.Get("Content-Type"), "text/javascript") {
			t.Errorf("%s Content-Type = %q", script, res.Header.Get("Content-Type"))
		}
	}
	css := a.client.Get("/assets/app.css")
	wantStatus(t, css, http.StatusOK)
	if !strings.HasPrefix(css.Header.Get("Content-Type"), "text/css") {
		t.Errorf("app.css Content-Type = %q", css.Header.Get("Content-Type"))
	}
	wantStatus(t, a.client.Get("/assets/nope.js"), http.StatusNotFound)
}

var hstsRoutes = []string{"/", "/healthz", "/assets/app.css"}

func TestNoHSTSByDefault(t *testing.T) {
	a := newApp(t, nil)
	for _, route := range hstsRoutes {
		if got := a.client.Get(route).Header.Get("Strict-Transport-Security"); got != "" {
			t.Errorf("%s carries Strict-Transport-Security by default: %q", route, got)
		}
	}
}

func TestHSTSIsAppWideWhenConfigured(t *testing.T) {
	a := newApp(t, map[string]string{"PINGWARD_HSTS_MAX_AGE": "31536000"})
	for _, route := range hstsRoutes {
		got := a.client.Get(route).Header.Get("Strict-Transport-Security")
		if got != "max-age=31536000" {
			t.Errorf("%s HSTS = %q, want max-age=31536000", route, got)
		}
	}
}

// The header is forced: a handler's own value must not weaken the configured one.
func TestHSTSOverridesAHandlersOwnValue(t *testing.T) {
	a := newApp(t, map[string]string{"PINGWARD_HSTS_MAX_AGE": "600"}, setsOwnHSTS)
	got := a.client.Get("/own-hsts").Header.Values("Strict-Transport-Security")
	if len(got) != 1 || got[0] != "max-age=600" {
		t.Errorf("HSTS = %v, want exactly max-age=600", got)
	}
}

func TestHealthzReturnsOK(t *testing.T) {
	a := newApp(t, nil)
	wantStatus(t, a.client.Get("/healthz"), http.StatusOK)
}

// Scalar loads from a CDN, so /api/docs skips the CSP rather than widening it
// for everyone; the other origin-wide headers stay.
func TestTheAPIDocsPageIsOutsideTheCSPButNotTheRest(t *testing.T) {
	a := newApp(t, nil)
	res := a.client.Get("/api/docs")
	if res.Header.Get("Content-Security-Policy") != "" {
		t.Error("/api/docs must not inherit the browser UI's CSP")
	}
	if got := res.Header.Get("X-Frame-Options"); got != "DENY" {
		t.Errorf("X-Frame-Options = %q, want DENY", got)
	}
}
