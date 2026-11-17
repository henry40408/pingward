package web

import (
	"net/http"

	"github.com/henry40408/pingward/internal/apikey"
	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/ratelimit"
)

type reauthOutcome int

const (
	// reauthPassed also covers an account with no local password: nothing to verify.
	reauthPassed reauthOutcome = iota
	reauthFailed
	reauthThrottled
)

// reauthenticate re-demands the user's password before a sensitive action (the
// threat is a borrowed session). Attempts charge the account limiter, or this
// would be an unmetered password oracle; success clears the bucket.
func (d *Deps) reauthenticate(user *models.User, submitted, surface string) reauthOutcome {
	if user.PasswordHash == nil {
		return reauthPassed
	}
	key := ratelimit.AccountKey(user.Username)
	if !d.AccountLimiter.Charge(key, d.Now()) {
		d.logReauthFailure(user, surface, "rate_limited")
		return reauthThrottled
	}
	if auth.VerifyPassword(submitted, *user.PasswordHash) {
		d.AccountLimiter.Clear(key)
		return reauthPassed
	}
	d.logReauthFailure(user, surface, "bad_current_password")
	return reauthFailed
}

// logReauthFailure is separate from login.failed, which is unauthenticated and
// carries an address instead of a user_id.
func (d *Deps) logReauthFailure(user *models.User, surface, reason string) {
	d.Logger.Warn("reauth.failed", "target", "auth", "username", auth.LogUsername(user.Username),
		"user_id", user.ID, "surface", surface, "reason", reason)
}

// reauthRefusal is the page text for a failed gate, "" when it passed.
func reauthRefusal(outcome reauthOutcome, failed string) string {
	switch outcome {
	case reauthFailed:
		return failed
	case reauthThrottled:
		return "Too many attempts — try again later."
	default:
		return ""
	}
}

// currentSessionHandle names the verified session by the hash of its id: the id
// is a bearer secret, so the handle stands in for it everywhere but the cookie.
func (d *Deps) currentSessionHandle(r *http.Request) (string, bool) {
	id, ok := d.sessionID(r)
	if !ok {
		return "", false
	}
	return apikey.Hash(id), true
}
