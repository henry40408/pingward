package web

import "net/http"

// Route is one web endpoint. The list is data so tests can walk it.
type Route struct {
	Method  string
	Pattern string
	Handler http.HandlerFunc
}

// Routes lists every web route. Handlers arrive with the setup, login,
// project, check and admin steps.
func Routes(d *Deps) []Route { return nil }

// Mount registers routes behind the web middleware, outermost first:
// CSP, no-store, forward-auth session, anonymous session, CSRF guard. Only
// these routes get them; unknown paths and the other routers do not.
func Mount(mux *http.ServeMux, d *Deps, routes []Route) {
	for _, route := range routes {
		chain := CSP(NoStore(d.ForwardAuthSession(d.AnonymousSession(d.CSRFGuard(route.Handler)))))
		mux.Handle(route.Method+" "+route.Pattern, chain)
	}
}
