package web

import "net/http"

// contentSecurityPolicy: script-src 'self' (no 'unsafe-inline', no nonce) holds
// only while every script is an /assets file and no template has an inline
// handler. style-src 'unsafe-inline' is for the heartbeat bars' height:Npx.
const contentSecurityPolicy = "default-src 'self'; " +
	"script-src 'self'; " +
	"style-src 'self' 'unsafe-inline'; " +
	"img-src 'self' data:; " +
	"font-src 'self'; " +
	"connect-src 'self'; " +
	"object-src 'none'; " +
	"base-uri 'none'; " +
	"form-action 'self'; " +
	"frame-ancestors 'none'"

// CSP covers the web routes only: /api/docs loads Scalar from a CDN, and
// admitting that CDN app-wide would weaken every page.
func CSP(next http.Handler) http.Handler {
	return DefaultHeaders(next, Header{"Content-Security-Policy", contentSecurityPolicy})
}

// NoStore keeps the cookie-bound pages (including /login and /setup, which
// render a session-derived _csrf) out of caches. A handler's own
// Cache-Control wins.
func NoStore(next http.Handler) http.Handler {
	return DefaultHeaders(next, Header{"Cache-Control", "no-store"})
}
