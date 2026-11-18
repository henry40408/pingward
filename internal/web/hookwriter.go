package web

import "net/http"

// hookWriter runs before once, just before the response head is sent, so a
// middleware can edit headers after the handler has set its own.
type hookWriter struct {
	http.ResponseWriter
	before func(http.Header)
	sent   bool
}

func (w *hookWriter) runHook() {
	if w.sent {
		return
	}
	w.sent = true
	w.before(w.Header())
}

func (w *hookWriter) WriteHeader(status int) {
	w.runHook()
	w.ResponseWriter.WriteHeader(status)
}

func (w *hookWriter) Write(body []byte) (int, error) {
	w.runHook()
	return w.ResponseWriter.Write(body)
}

// Flush keeps streaming responses (SSE) working through the wrapper.
func (w *hookWriter) Flush() {
	w.runHook()
	if flusher, ok := w.ResponseWriter.(http.Flusher); ok {
		flusher.Flush()
	}
}

// Unwrap lets http.ResponseController reach the real writer.
func (w *hookWriter) Unwrap() http.ResponseWriter { return w.ResponseWriter }

// serveHooked runs next with before applied to the response head. The final
// runHook covers a handler that writes nothing, whose head net/http sends only
// after it returns.
func serveHooked(next http.Handler, w http.ResponseWriter, r *http.Request, before func(http.Header)) {
	hooked := &hookWriter{ResponseWriter: w, before: before}
	next.ServeHTTP(hooked, r)
	hooked.runHook()
}

// Header is one response header.
type Header struct{ Name, Value string }

// DefaultHeaders adds each header the handler did not set itself.
func DefaultHeaders(next http.Handler, defaults ...Header) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		serveHooked(next, w, r, func(h http.Header) {
			for _, header := range defaults {
				if len(h.Values(header.Name)) == 0 {
					h.Set(header.Name, header.Value)
				}
			}
		})
	})
}

// ForceHeader sets a header, replacing whatever the handler wrote.
func ForceHeader(next http.Handler, header Header) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		serveHooked(next, w, r, func(h http.Header) { h.Set(header.Name, header.Value) })
	})
}

// appendSetCookie serves next with cookie added after the handler's own cookies.
func appendSetCookie(next http.Handler, w http.ResponseWriter, r *http.Request, cookie *http.Cookie) {
	serveHooked(next, w, r, func(h http.Header) { h.Add("Set-Cookie", cookie.String()) })
}
