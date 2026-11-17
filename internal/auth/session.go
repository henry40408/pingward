package auth

import (
	"time"

	"github.com/henry40408/pingward/internal/apikey"
	"github.com/henry40408/pingward/internal/ids"
)

const (
	SessionCookieBase = "pingward_session"
	// SessionCookieHostPrefixed is used when Secure is on: __Host- makes the
	// browser enforce Secure, Path=/ and no Domain, so a sibling subdomain
	// cannot overwrite the cookie.
	SessionCookieHostPrefixed = "__Host-pingward_session"
)

// SessionCookieName must stay conditional on cookieSecure: browsers refuse a
// __Host- cookie over plaintext HTTP, which would make login fail silently.
func SessionCookieName(cookieSecure bool) string {
	if cookieSecure {
		return SessionCookieHostPrefixed
	}
	return SessionCookieBase
}

const (
	// SessionIdleTTL: sessions.expires_at is "last activity + this", renewed
	// only past the half-life (see RefreshedExpiry).
	SessionIdleTTL = 72 * time.Hour
	// SessionAbsoluteMax caps a session from created_at; activity never extends it.
	SessionAbsoluteMax = 30 * 24 * time.Hour
)

// IsPastAbsoluteCap reports whether the session has outlived the absolute cap.
// A nil createdAt is never past it.
func IsPastAbsoluteCap(createdAt *time.Time, now time.Time) bool {
	return createdAt != nil && !now.Before(createdAt.Add(SessionAbsoluteMax))
}

// RenewalKind says which way a renewal moved expires_at; it is logged as
// "renewal" on session.renewed.
type RenewalKind string

const (
	// RenewalSlid moved forward (possibly truncated at the cap): ordinary activity.
	RenewalSlid RenewalKind = "slid"
	// RenewalClamped moved backwards: the stored expiry exceeded current policy.
	RenewalClamped RenewalKind = "clamped"
)

// SessionRenewal is the expires_at to write and which way it moved.
type SessionRenewal struct {
	ExpiresAt time.Time
	Kind      RenewalKind
}

// RefreshedExpiry returns the new expires_at, or nil when no write is needed.
// The target is min(now + idle, createdAt + absolute). It returns nil if the
// stored expiry already reaches the cap, or at least half the idle window
// remains (the write throttle). A stored expiry beyond the target is clamped
// down at once, bypassing the throttle.
func RefreshedExpiry(createdAt *time.Time, expiresAt, now time.Time) *SessionRenewal {
	next := now.Add(SessionIdleTTL)
	if createdAt != nil {
		cap := createdAt.Add(SessionAbsoluteMax)
		if !expiresAt.Before(cap) {
			return nil
		}
		if cap.Before(next) {
			next = cap
		}
	}
	if expiresAt.After(next) {
		return &SessionRenewal{ExpiresAt: next, Kind: RenewalClamped}
	}
	if expiresAt.Sub(now) >= SessionIdleTTL/2 {
		return nil
	}
	return &SessionRenewal{ExpiresAt: next, Kind: RenewalSlid}
}

// NewSessionToken returns a fresh session id (hyphenated UUID v4).
func NewSessionToken() string { return ids.NewV4() }

// logHandleLen is the SHA-256 hex length kept as a session's log identifier.
const logHandleLen = 16

// SessionLogHandle is a session's log identifier: the /account SHA-256 handle
// cut to 16 hex chars. Never log the id itself: it is the bearer secret.
func SessionLogHandle(sessionID string) string {
	return apikey.Hash(sessionID)[:logHandleLen]
}
