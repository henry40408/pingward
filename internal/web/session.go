package web

import (
	"context"
	"net/http"
	"net/netip"
	"strings"
	"unicode/utf8"

	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/secret"
	"github.com/henry40408/pingward/internal/store"
)

const maxUserAgentChars = 300

func (d *Deps) sessionCookieName() string { return auth.SessionCookieName(d.Config.CookieSecure) }

// sessionCookie carries the signed id. Its attributes must match
// sessionRemovalCookie: a removal cookie whose attributes differ can fail to
// overwrite the original. No Max-Age or Expires: the cookie is non-persistent
// by design and expiry lives in sessions.expires_at.
func (d *Deps) sessionCookie(value string) *http.Cookie {
	return &http.Cookie{
		Name: d.sessionCookieName(), Value: value, Path: "/",
		HttpOnly: true, SameSite: http.SameSiteLaxMode, Secure: d.Config.CookieSecure,
	}
}

func (d *Deps) sessionRemovalCookie() *http.Cookie { return d.sessionCookie("") }

// sessionID is the verified id from the request's session cookie. The first
// cookie of that name wins, so a forged duplicate later in the header cannot
// shadow it; a bad signature is rejected before any database work.
func (d *Deps) sessionID(r *http.Request) (string, bool) {
	cookie, err := r.Cookie(d.sessionCookieName())
	if err != nil {
		return "", false
	}
	return secret.VerifySession(d.Config.Secret, cookie.Value)
}

// csrfToken is the _csrf form value; empty without a valid session
// (unsubmittable, not a bypass).
func (d *Deps) csrfToken(r *http.Request) string {
	id, ok := d.sessionID(r)
	if !ok {
		return ""
	}
	return secret.DeriveCSRF(d.Config.Secret, id)
}

func requestUserAgent(r *http.Request) *string {
	if len(r.Header.Values("User-Agent")) == 0 {
		return nil
	}
	value := r.UserAgent()
	if utf8.RuneCountInString(value) > maxUserAgentChars {
		value = string([]rune(value)[:maxUserAgentChars])
	}
	return &value
}

// sessionOrigin is where a session was opened from, recorded on its row.
type sessionOrigin struct {
	userAgent *string
	ip        *string
	sso       bool
}

// openSession stores a new session and returns its cookie. The cookie carries
// <id>.<hmac>, never the bare id.
func (d *Deps) openSession(ctx context.Context, userID int64, origin sessionOrigin) (*http.Cookie, error) {
	sessionID := auth.NewSessionToken()
	now := d.Now()
	expires := now.Add(auth.SessionIdleTTL)
	err := d.Store.CreateSession(ctx, store.NewSession{
		ID: sessionID, UserID: userID, ExpiresAt: expires,
		UserAgent: origin.userAgent, IP: origin.ip, SSO: origin.sso,
	}, now)
	if err != nil {
		return nil, err
	}
	d.Logger.Info("session.created", "target", "session",
		"handle", auth.SessionLogHandle(sessionID), "user_id", userID, "sso", origin.sso,
		"ip", origin.ip, "user_agent", origin.userAgent, "expires_at", expires.Format("2006-01-02T15:04:05.999999999Z07:00"))
	return d.sessionCookie(secret.SignSession(d.Config.Secret, sessionID)), nil
}

// AnonymousSession gives every visitor a signed session cookie with no row (the
// CSRF token derives from the id), so the CSRF guard needs no path exemptions.
// It sits inside ForwardAuthSession so it cannot shadow a real session.
func (d *Deps) AnonymousSession(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if _, ok := d.sessionID(r); ok {
			next.ServeHTTP(w, r)
			return
		}
		// Signature only: an anonymous id has no row to look up.
		cookie := d.sessionCookie(secret.SignSession(d.Config.Secret, auth.NewSessionToken()))
		appendSetCookie(next, w, replaceRequestCookie(r, cookie), cookie)
	})
}

// replaceRequestCookie returns a copy of r whose Cookie header carries cookie
// in place of any same-named entry. The stale entry must be dropped, not
// appended past: the first match wins, so an expired id would shadow the fresh one.
func replaceRequestCookie(r *http.Request, cookie *http.Cookie) *http.Request {
	prefix := cookie.Name + "="
	kept := []string{}
	for _, line := range r.Header.Values("Cookie") {
		for _, pair := range strings.Split(line, ";") {
			pair = strings.TrimSpace(pair)
			if pair != "" && !strings.HasPrefix(pair, prefix) {
				kept = append(kept, pair)
			}
		}
	}
	kept = append(kept, cookie.Name+"="+cookie.Value)
	clone := r.Clone(r.Context())
	clone.Header.Set("Cookie", strings.Join(kept, "; "))
	return clone
}

// forwardAuthUser is the user named by a trusted forward-auth header,
// auto-provisioning a passwordless account on first sight (an admin only if it
// is the instance's first user). nil when disabled or not applicable.
func (d *Deps) forwardAuthUser(r *http.Request) *models.User {
	username, ok := auth.ForwardAuthUsername(r.Header, auth.PeerAddr(r.RemoteAddr), d.Config)
	if !ok {
		return nil
	}
	ctx := r.Context()
	user, err := d.Store.FindUserByUsername(ctx, username)
	if err != nil {
		return nil
	}
	if user != nil {
		if user.Disabled {
			return nil
		}
		return user
	}
	// Fail closed: a name the other surfaces would refuse must not mint an account.
	if _, refusal := auth.ValidateUsername(username); refusal != "" {
		return nil
	}
	id, err := d.Store.CreatePasswordlessUserFirstIsAdmin(ctx, username, d.Now())
	if err != nil {
		return nil
	}
	created, err := d.Store.FindUserByID(ctx, id)
	if err != nil || created == nil {
		return nil
	}
	// The first admin exists now, so /setup has nothing left to guard.
	if created.IsAdmin {
		d.SetupToken.Clear()
	}
	return created
}

func (d *Deps) clientIP(r *http.Request, peer netip.Addr) *string {
	ip, ok := auth.ClientIP(r.Header, peer, d.Config.TrustedProxies)
	if !ok {
		return nil
	}
	return &ip
}

// ForwardAuthSession gives a trusted forward-auth identity a real session so
// nothing needs a special case. It sits outside AnonymousSession and the CSRF
// guard; the cookie is injected into the request too, so forms rendered now
// derive a matching token. The short-circuit checks liveness: an anonymous
// cookie is validly signed but has no row.
func (d *Deps) ForwardAuthSession(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if d.Config.ForwardAuthHeader == nil || d.hasLiveSession(r) {
			next.ServeHTTP(w, r)
			return
		}
		user := d.forwardAuthUser(r)
		if user == nil {
			next.ServeHTTP(w, r)
			return
		}
		peer := auth.PeerAddr(r.RemoteAddr)
		cookie, err := d.openSession(r.Context(), user.ID, sessionOrigin{
			userAgent: requestUserAgent(r), ip: d.clientIP(r, peer), sso: true,
		})
		if err != nil {
			d.Logger.Error("failed to open a session for forward-auth user", "error", err)
			next.ServeHTTP(w, r)
			return
		}
		appendSetCookie(next, w, replaceRequestCookie(r, cookie), cookie)
	})
}

func (d *Deps) hasLiveSession(r *http.Request) bool {
	id, ok := d.sessionID(r)
	if !ok {
		return false
	}
	user, err := d.Store.FindSessionUser(r.Context(), id, d.Now())
	return err == nil && user != nil
}

// currentUser is the user from the session cookie, else from a trusted
// forward-auth header.
func (d *Deps) currentUser(r *http.Request) *models.User {
	if id, ok := d.sessionID(r); ok {
		user, err := d.Store.FindSessionUser(r.Context(), id, d.Now())
		if err == nil && user != nil && !user.Disabled {
			return user
		}
	}
	return d.forwardAuthUser(r)
}

// requireUser redirects to /login when nobody is signed in.
func (d *Deps) requireUser(w http.ResponseWriter, r *http.Request) (*models.User, bool) {
	user := d.currentUser(r)
	if user == nil {
		redirect(w, r, "/login")
		return nil, false
	}
	return user, true
}

// requireAdmin answers 403 "admin only" to a signed-in non-admin.
func (d *Deps) requireAdmin(w http.ResponseWriter, r *http.Request) (*models.User, bool) {
	user, ok := d.requireUser(w, r)
	if !ok {
		return nil, false
	}
	if !user.IsAdmin {
		adminOnly(w)
		return nil, false
	}
	return user, true
}
