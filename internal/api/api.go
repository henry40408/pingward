// Package api is the bearer-authenticated /api/v1 REST API, plus its OpenAPI
// document and reference page.
//
// /api/v1 authenticates only by Authorization: Bearer, never the session
// cookie, which is what makes it structurally CSRF-safe: it is mounted outside
// the web middleware. /api/docs and /api/openapi.json do use the web session,
// but are read-only GETs.
package api

import (
	"context"
	"net/http"
	"strconv"
	"strings"

	"github.com/henry40408/pingward/internal/apikey"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/web"
)

// API holds what every handler shares.
type API struct{ d *web.Deps }

func New(d *web.Deps) *API { return &API{d: d} }

// call is one authenticated request.
type call struct {
	a    *API
	w    http.ResponseWriter
	r    *http.Request
	user *models.User
	// id is the {id} path segment; 0 on routes without one.
	id int64
}

// writeCtx outlives a client that hangs up: a write that began must finish,
// or the audit trail and the data part ways.
func (c *call) writeCtx() context.Context { return context.WithoutCancel(c.r.Context()) }

// guard authenticates before anything else is read: a bad key is a 401 even
// when the path id or the body is also malformed. Order matters: bearer, path
// id, then (inside the handler) query and body, then ownership.
func (a *API) guard(handler func(*call)) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		user, ok := a.authenticate(w, r)
		if !ok {
			return
		}
		c := &call{a: a, w: w, r: r, user: user}
		if raw := r.PathValue("id"); raw != "" {
			id, err := strconv.ParseInt(raw, 10, 64)
			if err != nil {
				plainText(w, http.StatusBadRequest, "Invalid URL: Cannot parse `"+raw+"` to a `i64`.")
				return
			}
			c.id = id
		}
		handler(c)
	}
}

// authenticate resolves the bearer key to its owner. A key outlives its owner
// being disabled, so the owner is re-checked.
func (a *API) authenticate(w http.ResponseWriter, r *http.Request) (*models.User, bool) {
	token, ok := bearerToken(r)
	if !ok {
		unauthorized(w)
		return nil, false
	}
	userID, found, err := a.d.Store.ValidateAPIKey(r.Context(), apikey.Hash(token), a.d.Now())
	if err != nil {
		a.internalError(w, err)
		return nil, false
	}
	if !found {
		unauthorized(w)
		return nil, false
	}
	user, err := a.d.Store.FindUserByID(r.Context(), userID)
	if err != nil {
		a.internalError(w, err)
		return nil, false
	}
	if user == nil || user.Disabled {
		unauthorized(w)
		return nil, false
	}
	return user, true
}

// bearerToken is the text after "Bearer ". A header with bytes outside
// visible ASCII is no credential.
func bearerToken(r *http.Request) (string, bool) {
	header := r.Header.Get("Authorization")
	for i := 0; i < len(header); i++ {
		if header[i] != '\t' && (header[i] < 0x20 || header[i] > 0x7e) {
			return "", false
		}
	}
	token, ok := strings.CutPrefix(header, "Bearer ")
	return strings.TrimSpace(token), ok
}
