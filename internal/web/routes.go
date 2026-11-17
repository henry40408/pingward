package web

import "net/http"

// Route is one web endpoint. The list is data so tests can walk it.
type Route struct {
	Method  string
	Pattern string
	Handler http.HandlerFunc
}

// Routes lists every web route. Dashboard, check, channel and admin handlers
// arrive with their own steps.
func Routes(d *Deps) []Route {
	return []Route{
		{"GET", "/setup", d.setupPage},
		{"POST", "/setup", d.setupSubmit},
		{"GET", "/login", d.loginPage},
		{"POST", "/login", d.loginSubmit},
		{"POST", "/logout", d.logout},
		{"GET", "/projects/new", d.projectNew},
		{"POST", "/projects", d.projectCreate},
		{"GET", "/account", d.accountPage},
		{"POST", "/account/password", d.accountPassword},
		{"POST", "/account/api-keys", d.apiKeyCreate},
		{"POST", "/account/api-keys/{id}/delete", d.apiKeyDelete},
		{"POST", "/account/sessions/{handle}/revoke", d.sessionRevoke},
		{"POST", "/account/sessions/revoke-others", d.sessionsRevokeOthers},
		{"GET", "/admin/unlock", d.adminUnlockPage},
		{"POST", "/admin/unlock", d.adminUnlock},
	}
}

// Mount registers routes behind the web middleware, outermost first:
// CSP, no-store, forward-auth session, anonymous session, CSRF guard. Only
// these routes get them; unknown paths and the other routers do not.
func Mount(mux *http.ServeMux, d *Deps, routes []Route) {
	for _, route := range routes {
		chain := CSP(NoStore(d.ForwardAuthSession(d.AnonymousSession(d.CSRFGuard(route.Handler)))))
		mux.Handle(route.Method+" "+route.Pattern, chain)
	}
}
