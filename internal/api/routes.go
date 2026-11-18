package api

import (
	"net/http"

	"github.com/henry40408/pingward/internal/web"
)

// Routes lists every /api/v1 endpoint. The list is data so a test can hold it
// against openapi.json.
func (a *API) Routes() []web.Route {
	return []web.Route{
		{Method: "GET", Pattern: "/api/v1/projects", Handler: a.guard(a.listProjects)},
		{Method: "POST", Pattern: "/api/v1/projects", Handler: a.guard(a.createProject)},
		{Method: "GET", Pattern: "/api/v1/projects/{id}", Handler: a.guard(a.getProject)},
		{Method: "PATCH", Pattern: "/api/v1/projects/{id}", Handler: a.guard(a.updateProject)},
		{Method: "DELETE", Pattern: "/api/v1/projects/{id}", Handler: a.guard(a.deleteProject)},
		{Method: "GET", Pattern: "/api/v1/projects/{id}/checks", Handler: a.guard(a.listProjectChecks)},
		{Method: "POST", Pattern: "/api/v1/projects/{id}/checks", Handler: a.guard(a.createCheck)},
		{Method: "GET", Pattern: "/api/v1/projects/{id}/channels", Handler: a.guard(a.listProjectChannels)},
		{Method: "POST", Pattern: "/api/v1/projects/{id}/channels", Handler: a.guard(a.createChannel)},
		{Method: "GET", Pattern: "/api/v1/checks/{id}", Handler: a.guard(a.getCheck)},
		{Method: "PATCH", Pattern: "/api/v1/checks/{id}", Handler: a.guard(a.updateCheck)},
		{Method: "DELETE", Pattern: "/api/v1/checks/{id}", Handler: a.guard(a.deleteCheck)},
		{Method: "GET", Pattern: "/api/v1/checks/{id}/pings", Handler: a.guard(a.listCheckPings)},
		{Method: "GET", Pattern: "/api/v1/checks/{id}/notifications", Handler: a.guard(a.listCheckNotifications)},
		{Method: "POST", Pattern: "/api/v1/checks/{id}/pause", Handler: a.guard(a.pauseCheck())},
		{Method: "POST", Pattern: "/api/v1/checks/{id}/resume", Handler: a.guard(a.resumeCheck())},
		{Method: "POST", Pattern: "/api/v1/checks/{id}/ack", Handler: a.guard(a.ackCheck())},
		{Method: "POST", Pattern: "/api/v1/checks/{id}/regenerate", Handler: a.guard(a.regenerateCheck())},
		{Method: "PUT", Pattern: "/api/v1/checks/{id}/channels", Handler: a.guard(a.setCheckChannels)},
		{Method: "GET", Pattern: "/api/v1/channels/{id}", Handler: a.guard(a.getChannel)},
		{Method: "PATCH", Pattern: "/api/v1/channels/{id}", Handler: a.guard(a.updateChannel)},
		{Method: "DELETE", Pattern: "/api/v1/channels/{id}", Handler: a.guard(a.deleteChannel)},
		{Method: "GET", Pattern: "/api/v1/keys", Handler: a.guard(a.listKeys)},
	}
}

// Mount registers the /api/v1 routes, then the two session-gated reference
// routes, which alone carry Cache-Control: no-store.
func (a *API) Mount(mux *http.ServeMux) {
	for _, route := range a.Routes() {
		mux.HandleFunc(route.Method+" "+route.Pattern, route.Handler)
	}
	mux.Handle("GET /api/openapi.json", web.NoStore(http.HandlerFunc(a.openAPIDocument)))
	mux.Handle("GET /api/docs", web.NoStore(http.HandlerFunc(a.referencePage)))
}
