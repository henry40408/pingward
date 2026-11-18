package web

import "net/http"

// Route is one web endpoint. The list is data so tests can walk it.
type Route struct {
	Method  string
	Pattern string
	Handler http.HandlerFunc
}

// Routes lists every web route.
func Routes(d *Deps) []Route {
	routes := []Route{
		{"GET", "/{$}", d.dashboard},
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
		{"GET", "/admin", d.adminPage},
		{"GET", "/admin/audit", d.adminAuditFragment},
		{"POST", "/admin/settings", d.settingsSave},
		{"GET", "/admin/unlock", d.adminUnlockPage},
		{"POST", "/admin/unlock", d.adminUnlock},
		{"POST", "/admin/users", d.usersCreate},
		{"POST", "/admin/users/{id}/delete", d.usersDelete},
		{"POST", "/admin/users/{id}/password", d.usersSetPassword},
		{"POST", "/admin/users/{id}/admin", d.usersToggleAdmin},
		{"POST", "/admin/users/{id}/disabled", d.usersSetDisabled},
		{"POST", "/admin/checks/{id}/ping-url", d.adminCheckRevealPingURL},
	}
	return append(routes, resourceRoutes(d)...)
}

// resourceRoutes registers each owner route with its /admin twin, built from
// the same handler factory so the two cannot drift apart.
func resourceRoutes(d *Deps) []Route {
	type factory = func(admin bool) http.HandlerFunc
	shared := []struct {
		method, pattern string
		handler         factory
	}{
		{"GET", "/projects/{id}", d.projectShow},
		{"POST", "/projects/{id}", d.projectUpdate},
		{"GET", "/projects/{id}/edit", d.projectEdit},
		{"POST", "/projects/{id}/delete", d.projectDelete},
		{"GET", "/projects/{pid}/checks/new", d.checkNew},
		{"POST", "/projects/{pid}/checks", d.checkCreate},
		{"GET", "/checks/{id}", d.checkShow},
		{"POST", "/checks/{id}", d.checkUpdate},
		{"GET", "/checks/{id}/edit", d.checkEdit},
		{"GET", "/checks/{id}/pings", d.checkPings},
		{"GET", "/checks/{id}/events", d.checkEvents},
		{"GET", "/checks/{id}/notifications", d.checkNotifications},
		{"POST", "/checks/{id}/pause", d.checkPause},
		{"POST", "/checks/{id}/resume", d.checkResume},
		{"POST", "/checks/{id}/ack", d.checkAck},
		{"POST", "/checks/{id}/regenerate", d.checkRegenerate},
		{"POST", "/checks/{id}/delete", d.checkDelete},
		{"POST", "/checks/{id}/channels", d.checkSetChannels},
		{"GET", "/projects/{pid}/channels/new", d.channelNew},
		{"POST", "/projects/{pid}/channels", d.channelCreate},
		{"GET", "/channels/{id}/edit", d.channelEdit},
		{"POST", "/channels/{id}", d.channelUpdate},
		{"POST", "/channels/{id}/delete", d.channelDelete},
		{"POST", "/channels/{id}/test", d.channelTest},
	}
	var routes []Route
	for _, route := range shared {
		routes = append(routes,
			Route{route.method, route.pattern, route.handler(false)},
			Route{route.method, "/admin" + route.pattern, route.handler(true)})
	}
	return routes
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
