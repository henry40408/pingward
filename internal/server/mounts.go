package server

import (
	"net/http"

	"github.com/henry40408/pingward/internal/api"
	"github.com/henry40408/pingward/internal/notify"
	"github.com/henry40408/pingward/internal/ping"
	"github.com/henry40408/pingward/internal/web"
)

// Mounts adds the ping router and the v1 API. Neither goes through the web
// middleware (no CSRF, no sessions): pings authenticate by their UUID and the
// API by Bearer key.
func Mounts(d *web.Deps) func(*http.ServeMux) {
	deliverer := notify.Deliverer{Store: d.Store, Retry: notify.DefaultRetryPolicy, SMTP: d.Config.SMTP}
	pings := &ping.Handler{
		Store: d.Store, Config: d.Config, Events: d.Events, Now: d.Now, DueTime: d.DueTime,
		Deliver: deliverer.DeliverEvent,
	}
	return func(mux *http.ServeMux) {
		pings.Mount(mux)
		api.New(d).Mount(mux)
	}
}
