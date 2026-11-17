// Package elevate tracks short-lived per-session elevation for admin actions
// that grant access.
//
// The single-button admin forms have no room for a password field, so an
// admin unlocks once and gated handlers check freshness. It is in-memory and
// per-process: a restart or another replica just asks again. Entries are keyed
// by the session's SHA-256 handle, never the raw id (the bearer secret).
// Every handler that ends a session drops its entry; expiry (prune, the
// absolute cap) leaves it to the TTL, far shorter than any session lifetime.
package elevate

import (
	"sync"
	"time"
)

// TTL is how long an unlock lasts.
const TTL = 900 * time.Second

type grant struct {
	userID int64
	at     time.Time
}

// Elevations holds the live elevations: session handle to (user id, granted at).
type Elevations struct {
	mu      sync.Mutex
	granted map[string]grant
	ttl     time.Duration
}

func New(ttl time.Duration) *Elevations {
	return &Elevations{granted: make(map[string]grant), ttl: ttl}
}

// Grant starts (or restarts) handle's window, sweeping expired entries: enough
// pruning, since each entry costs a successful password check.
func (e *Elevations) Grant(handle string, userID int64, now time.Time) {
	e.mu.Lock()
	defer e.mu.Unlock()
	for key, entry := range e.granted {
		if now.Sub(entry.at) >= e.ttl {
			delete(e.granted, key)
		}
	}
	e.granted[handle] = grant{userID: userID, at: now}
}

// RemainingSecs returns the whole seconds left on handle's window; ok is false
// when it has none. At the boundary the answer is "none", never zero.
func (e *Elevations) RemainingSecs(handle string, now time.Time) (secs int64, ok bool) {
	e.mu.Lock()
	defer e.mu.Unlock()
	entry, found := e.granted[handle]
	if !found {
		return 0, false
	}
	elapsedSecs := int64(now.Sub(entry.at) / time.Second)
	secs = int64(e.ttl/time.Second) - elapsedSecs
	if secs <= 0 {
		return 0, false
	}
	return secs, true
}

// Revoke ends handle's window early, as its session ends.
func (e *Elevations) Revoke(handle string) {
	e.mu.Lock()
	defer e.mu.Unlock()
	delete(e.granted, handle)
}

// RevokeUser ends the window of every userID session except keep (empty keeps
// none): the in-memory half of deleting a user's sessions.
func (e *Elevations) RevokeUser(userID int64, keep string) {
	e.mu.Lock()
	defer e.mu.Unlock()
	for handle, entry := range e.granted {
		if entry.userID == userID && handle != keep {
			delete(e.granted, handle)
		}
	}
}
