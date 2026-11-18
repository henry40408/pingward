// Package server composes the routers and the app-wide middleware.
package server

import (
	"fmt"
	"net/http"
	"strconv"

	"github.com/henry40408/pingward/internal/assets"
	"github.com/henry40408/pingward/internal/web"
)

// New builds the handler: /healthz, the web routes behind their own
// middleware, the static assets, and whatever extra mounts (ping, API) add.
// HSTS sits inside security headers, which are outermost.
func New(d *web.Deps, webRoutes []web.Route, extra ...func(*http.ServeMux)) http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, _ *http.Request) {
		_, _ = fmt.Fprint(w, "ok")
	})
	web.Mount(mux, d, webRoutes)
	assetHandler := assets.Handler()
	for _, pattern := range []string{"GET /assets/", "GET /favicon.svg", "GET /apple-touch-icon.png"} {
		mux.Handle(pattern, assetHandler)
	}
	for _, mount := range extra {
		mount(mux)
	}
	return securityHeaders(hsts(d.Config.HSTSMaxAgeSecs, mux))
}

// securityHeaders covers what the CSP does not: nosniff for JSON and text
// bodies, X-Frame-Options for /api/docs, and a Referrer-Policy since check
// URLs identify checks. Headers a handler set itself are kept.
func securityHeaders(next http.Handler) http.Handler {
	return web.DefaultHeaders(next,
		web.Header{Name: "X-Content-Type-Options", Value: "nosniff"},
		web.Header{Name: "X-Frame-Options", Value: "DENY"},
		web.Header{Name: "Referrer-Policy", Value: "same-origin"},
		web.Header{Name: "Permissions-Policy", Value: "geolocation=(), camera=(), microphone=(), payment=(), usb=()"},
	)
}

// hsts is off unless PINGWARD_HSTS_MAX_AGE is set (TLS terminates at the
// proxy) and app-wide, since HSTS covers the origin. No includeSubDomains or
// preload: those are near-irreversible.
func hsts(maxAgeSecs int64, next http.Handler) http.Handler {
	if maxAgeSecs <= 0 {
		return next
	}
	return web.ForceHeader(next, web.Header{
		Name: "Strict-Transport-Security", Value: "max-age=" + strconv.FormatInt(maxAgeSecs, 10),
	})
}
