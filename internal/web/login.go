package web

import (
	"net/http"
	"net/netip"

	"github.com/henry40408/pingward/internal/apikey"
	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/ratelimit"
	"github.com/henry40408/pingward/internal/views"
)

// clearSiteData is cache only: "cookies" spans the registered domain and would
// clear the gateway's cookie before its logout redirect; "storage" holds only
// the theme; "executionContexts" forces a reload that fights the redirect.
const clearSiteData = `"cache"`

func (d *Deps) loginPage(w http.ResponseWriter, r *http.Request) {
	users, err := d.Store.CountUsers(r.Context())
	if err != nil {
		internalError(w)
		return
	}
	if users == 0 {
		redirect(w, r, "/setup")
		return
	}
	if d.currentUser(r) != nil {
		redirect(w, r, "/")
		return
	}
	writeHTML(w, http.StatusOK, views.Login(views.AuthView{CSRF: d.csrfToken(r)}))
}

// loginFailure is one login.failed event discriminated by reason: the
// operator's only spray signal. The username is attacker-chosen, so it goes
// through LogUsername.
type loginFailure struct {
	username string
	ip       *string
	bucket   netip.Addr
	reason   string
}

func (d *Deps) logLoginFailure(f loginFailure) {
	d.Logger.Warn("login.failed", "target", "auth", "username", auth.LogUsername(f.username),
		"ip", f.ip, "bucket", f.bucket.String(), "reason", f.reason)
}

// loginRefusal wording never hints that the username is real.
func (d *Deps) loginRefusal(w http.ResponseWriter, r *http.Request, status int, message string) {
	writeHTML(w, status, views.Login(views.AuthView{CSRF: d.csrfToken(r), Error: message}))
}

func (d *Deps) throttledLogin(w http.ResponseWriter, r *http.Request, window int64) {
	setRetryAfter(w, window)
	d.loginRefusal(w, r, http.StatusTooManyRequests, "too many login attempts — try again later")
}

func (d *Deps) loginSubmit(w http.ResponseWriter, r *http.Request) {
	username, password := r.PostFormValue("username"), r.PostFormValue("password")
	peer := auth.PeerAddr(r.RemoteAddr)
	// Keyed by RateLimitKey, not the recorded address. Charged before the
	// lookup so a throttled request never pays for argon2.
	client := auth.RateLimitKey(r.Header, peer, d.Config.TrustedProxies)
	ip := d.clientIP(r, peer)
	failure := loginFailure{username: username, ip: ip, bucket: client}
	if !d.LoginLimiter.Charge(client, d.Now()) {
		failure.reason = "rate_limited"
		d.logLoginFailure(failure)
		d.throttledLogin(w, r, int64(ratelimit.LoginWindow.Seconds()))
		return
	}
	// Per account too, or N addresses buy MaxAttempts x N guesses. Keyed on the
	// submitted name so an invented name throttles identically.
	account := ratelimit.AccountKey(username)
	if !d.AccountLimiter.Charge(account, d.Now()) {
		failure.reason = "account_locked"
		d.logLoginFailure(failure)
		d.throttledLogin(w, r, int64(ratelimit.AccountWindow.Seconds()))
		return
	}
	user, err := d.Store.FindUserByUsername(r.Context(), username)
	if err != nil {
		internalError(w)
		return
	}
	var stored *string
	if user != nil {
		stored = user.PasswordHash
	}
	// An unknown username must still cost one argon2 verification.
	if !auth.VerifyPasswordOrDummy(password, stored) {
		failure.reason = "bad_credentials"
		d.logLoginFailure(failure)
		d.loginRefusal(w, r, http.StatusOK, "invalid username or password")
		return
	}
	if user.Disabled {
		// Not released: a known-disabled credential must not probe the budget forever.
		failure.reason = "account_disabled"
		d.logLoginFailure(failure)
		d.loginRefusal(w, r, http.StatusOK, "account is disabled")
		return
	}
	d.LoginLimiter.Release(client)
	d.AccountLimiter.Clear(account)
	d.startSession(w, r, user.ID, ip)
}

// logout ends the local session. Behind a gateway the next request re-mints
// it from the identity header, so redirect to the configured logout URL, or
// (header present, no URL) flash that. Targets never come from the request.
func (d *Deps) logout(w http.ResponseWriter, r *http.Request) {
	if id, ok := d.sessionID(r); ok {
		if err := d.Store.DeleteSession(writeCtx(r), id); err != nil {
			internalError(w)
			return
		}
		d.Elevations.Revoke(apikey.Hash(id))
		d.Logger.Info("session.destroyed", "target", "session", "reason", "logout", "handle", auth.SessionLogHandle(id))
	}
	removal := d.sessionRemovalCookie()
	removal.MaxAge = -1
	http.SetCookie(w, removal)
	if target := d.Config.ForwardAuthLogout; target != nil {
		w.Header().Set("Clear-Site-Data", clearSiteData)
		redirect(w, r, *target)
		return
	}
	if _, viaGateway := auth.ForwardAuthUsername(r.Header, auth.PeerAddr(r.RemoteAddr), d.Config); viaGateway {
		// No Clear-Site-Data: this exit's job is delivering the flash cookie.
		http.SetCookie(w, d.flashCookie("forward_auth_logout"))
		redirect(w, r, "/")
		return
	}
	w.Header().Set("Clear-Site-Data", clearSiteData)
	redirect(w, r, "/login")
}
