package web

import (
	"net/http"

	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/ratelimit"
	"github.com/henry40408/pingward/internal/secret"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/views"
)

const setupTokenRefusal = "the setup token is missing or wrong — find it in the server log"

func (d *Deps) setupPage(w http.ResponseWriter, r *http.Request) {
	users, err := d.Store.CountUsers(r.Context())
	if err != nil {
		internalError(w)
		return
	}
	if users > 0 {
		redirect(w, r, "/login")
		return
	}
	writeHTML(w, http.StatusOK, views.Setup(views.AuthView{CSRF: d.csrfToken(r)}))
}

func (d *Deps) setupTokenMatches(submitted string) bool {
	expected := d.SetupToken.Get()
	if expected == "" {
		return false
	}
	return secret.VerifySetupToken(d.Config.Secret, expected, submitted)
}

func (d *Deps) setupRefusal(w http.ResponseWriter, r *http.Request, status int, message string) {
	if status == http.StatusTooManyRequests {
		setRetryAfter(w, int64(ratelimit.SetupWindow.Seconds()))
	}
	writeHTML(w, status, views.Setup(views.AuthView{CSRF: d.csrfToken(r), Error: message}))
}

// rejectSetupToken audits a wrong token: it is the only signal of someone
// probing a fresh instance. No user exists yet, hence actor id 0.
func (d *Deps) rejectSetupToken(w http.ResponseWriter, r *http.Request, ip *string) {
	d.Logger.Warn("setup.token_rejected", "target", "auth", "ip", ip)
	method, path := "POST", "/setup"
	_, err := d.Store.RecordAudit(r.Context(), store.NewAudit{
		ActorUsername: "(setup)", Action: "setup.token_rejected",
		Method: &method, Path: &path, Detail: ip,
	}, d.Now())
	if err != nil {
		internalError(w)
		return
	}
	d.setupRefusal(w, r, http.StatusForbidden, setupTokenRefusal)
}

// setupSubmit throttles per client, then checks the token, then does the
// password work: the token is what keeps a stranger away from argon2 and the
// insert.
func (d *Deps) setupSubmit(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()
	users, err := d.Store.CountUsers(ctx)
	if err != nil {
		internalError(w)
		return
	}
	if users > 0 {
		redirect(w, r, "/login")
		return
	}
	peer := auth.PeerAddr(r.RemoteAddr)
	client := auth.RateLimitKey(r.Header, peer, d.Config.TrustedProxies)
	if !d.SetupLimiter.Charge(client, d.Now()) {
		d.setupRefusal(w, r, http.StatusTooManyRequests, "too many setup attempts — try again later")
		return
	}
	ip := d.clientIP(r, peer)
	if !d.setupTokenMatches(r.PostFormValue("setup_token")) {
		d.refuseSetupToken(w, r, ip)
		return
	}
	username, refusal := auth.ValidateUsername(r.PostFormValue("username"))
	if refusal == "" {
		refusal = auth.ValidatePassword(r.PostFormValue("password"))
	}
	if refusal != "" {
		d.setupRefusal(w, r, http.StatusOK, refusal)
		return
	}
	d.createFirstAdmin(w, r, username, ip)
}

// refuseSetupToken: the winner of a race clears the token; its losers are
// late, not hostile.
func (d *Deps) refuseSetupToken(w http.ResponseWriter, r *http.Request, ip *string) {
	users, err := d.Store.CountUsers(r.Context())
	if err != nil {
		internalError(w)
		return
	}
	if users > 0 {
		redirect(w, r, "/login")
		return
	}
	d.rejectSetupToken(w, r, ip)
}

// createFirstAdmin relies on the insert being the gate: two racing first
// visitors both pass the count check.
func (d *Deps) createFirstAdmin(w http.ResponseWriter, r *http.Request, username string, ip *string) {
	hash, err := auth.HashPassword(r.PostFormValue("password"))
	if err != nil {
		internalError(w)
		return
	}
	id, created, err := d.Store.CreateFirstAdmin(r.Context(), username, hash, d.Now())
	if err != nil {
		internalError(w)
		return
	}
	if !created {
		redirect(w, r, "/login")
		return
	}
	d.SetupToken.Clear()
	d.startSession(w, r, id, ip)
}

// startSession opens a fresh session id on every login (session fixation).
func (d *Deps) startSession(w http.ResponseWriter, r *http.Request, userID int64, ip *string) {
	cookie, err := d.openSession(r.Context(), userID, sessionOrigin{userAgent: requestUserAgent(r), ip: ip})
	if err != nil {
		internalError(w)
		return
	}
	http.SetCookie(w, cookie)
	redirect(w, r, "/")
}
