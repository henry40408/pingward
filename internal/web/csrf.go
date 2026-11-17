package web

import (
	"bytes"
	"io"
	"net/http"
	"net/url"
	"strings"

	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/secret"
)

// csrfMaxBodyBytes caps what a client can make the guard buffer; browser forms
// are far below it.
const csrfMaxBodyBytes = 1 << 20

// logCSRFRejection logs csrf.rejected. noisy (token_missing) logs at debug: the
// guard refuses POST /login before the limiter, so scanners produce it in bulk.
// Other reasons mean a token failed.
func (d *Deps) logCSRFRejection(reason, sessionID string, noisy bool) {
	args := []any{"target", "auth", "reason", reason}
	if sessionID != "" {
		args = append(args, "handle", auth.SessionLogHandle(sessionID))
	} else {
		args = append(args, "handle", nil)
	}
	if noisy {
		d.Logger.Debug("csrf.rejected", args...)
		return
	}
	d.Logger.Warn("csrf.rejected", args...)
}

// CSRFGuard is a synchronizer-token guard over the web routes only. There are
// no path exemptions: AnonymousSession gives everyone a token. The token comes
// from X-CSRF-Token or the first _csrf form field. The 403 is bodyless so it
// does not tell a scanner what to send.
func (d *Deps) CSRFGuard(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.Method {
		case http.MethodGet, http.MethodHead, http.MethodOptions:
			next.ServeHTTP(w, r)
			return
		}
		sessionID, ok := d.sessionID(r)
		if !ok {
			// Unreachable unless layer ordering broke: AnonymousSession runs outside.
			d.logCSRFRejection("no_session", "", false)
			w.WriteHeader(http.StatusForbidden)
			return
		}
		// Prefer the header token: this path never buffers the body.
		if headerTokens := r.Header.Values("X-CSRF-Token"); len(headerTokens) > 0 {
			if secret.VerifyCSRF(d.Config.Secret, sessionID, headerTokens[0]) {
				next.ServeHTTP(w, r)
				return
			}
			d.logCSRFRejection("header_mismatch", sessionID, false)
			w.WriteHeader(http.StatusForbidden)
			return
		}
		d.checkFormToken(w, r, sessionID, next)
	})
}

func (d *Deps) checkFormToken(w http.ResponseWriter, r *http.Request, sessionID string, next http.Handler) {
	// One byte over the cap proves the body is too big.
	body, err := io.ReadAll(io.LimitReader(r.Body, csrfMaxBodyBytes+1))
	if err != nil || len(body) > csrfMaxBodyBytes {
		d.logCSRFRejection("body_unreadable", sessionID, false)
		w.WriteHeader(http.StatusForbidden)
		return
	}
	submitted, found := firstFormValue(string(body), "_csrf")
	if !found {
		// The form was never rendered; this is the noisy one.
		d.logCSRFRejection("token_missing", sessionID, true)
		w.WriteHeader(http.StatusForbidden)
		return
	}
	if !secret.VerifyCSRF(d.Config.Secret, sessionID, submitted) {
		d.logCSRFRejection("token_mismatch", sessionID, false)
		w.WriteHeader(http.StatusForbidden)
		return
	}
	// Restore the body so the handler can read it again.
	r.Body = io.NopCloser(bytes.NewReader(body))
	next.ServeHTTP(w, r)
}

// firstFormValue is the first value of key in an urlencoded body. Unlike
// url.ParseQuery it keeps going past a malformed pair, as form_urlencoded does.
func firstFormValue(body, key string) (string, bool) {
	for _, pair := range strings.Split(body, "&") {
		name, value, _ := strings.Cut(pair, "=")
		decodedName, err := url.QueryUnescape(name)
		if err != nil || decodedName != key {
			continue
		}
		decodedValue, err := url.QueryUnescape(value)
		if err != nil {
			decodedValue = value
		}
		return decodedValue, true
	}
	return "", false
}
