package web

import (
	"net/http"

	"github.com/henry40408/pingward/internal/secret"
)

const (
	flashCookieBase         = "pingward_flash"
	flashCookieHostPrefixed = "__Host-pingward_flash"
)

// flashMessages maps the fixed keys a flash cookie may carry to their text, so
// a cookie value never renders verbatim.
var flashMessages = map[string]string{
	"channels":         "Notify channels saved.",
	"settings":         "Settings saved.",
	"users_blocked":    "That action was refused: you cannot remove your own access, and the last enabled admin cannot be removed.",
	"password_changed": "Password changed. Any other signed-in sessions were signed out; API keys are unaffected.",
	"admin_locked":     "That action wasn't performed — it grants access, so it needs confirming first.",
	// Names no action: a refused one is dropped, not replayed, and this page is
	// also reached by plain navigation.
	"admin_unlocked":      "Confirmed. If an action was refused a moment ago it was not performed — do it again now.",
	"forward_auth_logout": "Signed out locally, but you're authenticated through your reverse proxy — this app can't end that session. To sign out completely, log out at your proxy or SSO provider.",
}

func (d *Deps) flashCookieName() string {
	if d.Config.CookieSecure {
		return flashCookieHostPrefixed
	}
	return flashCookieBase
}

// flashCookieRaw is shared by setter and remover so their attributes cannot drift.
func (d *Deps) flashCookieRaw(raw string) *http.Cookie {
	return &http.Cookie{
		Name: d.flashCookieName(), Value: raw, Path: "/",
		HttpOnly: true, SameSite: http.SameSiteLaxMode, Secure: d.Config.CookieSecure,
	}
}

// flashCookie stores <payload>.<hmac>. surface is a fixed key, never user input.
func (d *Deps) flashCookie(surface string) *http.Cookie {
	return d.flashCookieRaw(secret.SignFlash(d.Config.Secret, surface))
}

// flashRemovalCookie is unsigned: removal works by attributes, and flashPayload
// rejects an unsigned value anyway. MaxAge -1 also sends an Expires in the past.
func (d *Deps) flashRemovalCookie() *http.Cookie {
	cookie := d.flashCookieRaw("")
	cookie.MaxAge = -1
	return cookie
}

// flashPayload is empty when the cookie is absent, malformed, or not signed
// with this process's secret.
func (d *Deps) flashPayload(r *http.Request) string {
	cookie, err := r.Cookie(d.flashCookieName())
	if err != nil {
		return ""
	}
	payload, ok := secret.VerifyFlash(d.Config.Secret, cookie.Value)
	if !ok {
		return ""
	}
	return payload
}

// takeFlash consumes the flash only if it was set for surface (otherwise it is
// left for its own page) and returns its text.
func (d *Deps) takeFlash(w http.ResponseWriter, r *http.Request, surface string) (string, bool) {
	if d.flashPayload(r) != surface {
		return "", false
	}
	message, known := flashMessages[surface]
	if !known {
		return "", false
	}
	http.SetCookie(w, d.flashRemovalCookie())
	return message, true
}
