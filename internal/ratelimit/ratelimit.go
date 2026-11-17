// Package ratelimit holds the fixed-window limiter shared by the login,
// account and setup throttles.
//
// It is in-memory and per-process: each replica counts separately and a
// restart resets every counter. Invariants, each pinned by a test:
//
//  1. Check and record happen under one lock, or concurrent requests all pass.
//  2. At the key cap, expired windows are pruned and live counters are never
//     cleared, or a throttled caller resets itself by spraying fresh keys.
//  3. A caller with no room for its own bucket is charged to a shared overflow
//     bucket rather than waved through unmetered.
package ratelimit

import (
	"sync"
	"time"
)

const (
	LoginMaxAttempts = 5
	LoginWindow      = 60 * time.Second

	// AccountMaxAttempts is looser than the login limit because exhausting it
	// locks out the owner: an accepted denial-of-service primitive for anyone
	// who knows a username.
	AccountMaxAttempts = 10
	AccountWindow      = 900 * time.Second

	// SetupMaxAttempts is strict: the only thing to guess is a 50-bit token.
	SetupMaxAttempts = 5
	SetupWindow      = 900 * time.Second
)

// MaxEntries caps tracked keys so a spray of addresses or usernames cannot
// grow the map.
const MaxEntries = 10_000

// accountKeyMaxChars bounds the unauthenticated username used as a key.
const accountKeyMaxChars = 64

// overflowFactor sizes the shared overflow bucket as a multiple of
// maxAttempts: generous so a spray is not a global lockout, finite so it does
// not buy unlimited guesses.
const overflowFactor = 10

type counter struct {
	count   uint32
	started time.Time
}

// charge adds one attempt to the counter and reports whether it is allowed.
func (c *counter) charge(max uint32, window time.Duration, now time.Time) bool {
	if now.Sub(c.started) >= window {
		*c = counter{count: 1, started: now}
		return true
	}
	if c.count >= max {
		return false
	}
	c.count++
	return true
}

// Limiter is a fixed-window limiter keyed by K.
type Limiter[K comparable] struct {
	mu          sync.Mutex
	perKey      map[K]*counter
	overflow    counter
	maxAttempts uint32
	window      time.Duration
	maxEntries  int
}

// New returns a limiter allowing maxAttempts per window per key. The overflow
// bucket's window starts at now.
func New[K comparable](maxAttempts uint32, window time.Duration, now time.Time) *Limiter[K] {
	return &Limiter[K]{
		perKey:      make(map[K]*counter),
		overflow:    counter{started: now},
		maxAttempts: maxAttempts,
		window:      window,
		maxEntries:  MaxEntries,
	}
}

// Charge reserves an attempt for key and reports whether it may proceed. It is
// taken before the credential check; a success hands it back via Release or
// Clear.
func (l *Limiter[K]) Charge(key K, now time.Time) bool {
	l.mu.Lock()
	defer l.mu.Unlock()

	entry, tracked := l.perKey[key]
	if !tracked && len(l.perKey) >= l.maxEntries {
		l.pruneExpired(now)
		if len(l.perKey) >= l.maxEntries {
			return l.overflow.charge(satMul(l.maxAttempts, overflowFactor), l.window, now)
		}
	}
	if !tracked {
		entry = &counter{started: now}
		l.perKey[key] = entry
	}
	return entry.charge(l.maxAttempts, l.window, now)
}

func (l *Limiter[K]) pruneExpired(now time.Time) {
	for key, entry := range l.perKey {
		if now.Sub(entry.started) >= l.window {
			delete(l.perKey, key)
		}
	}
}

func satMul(a, b uint32) uint32 {
	product := uint64(a) * uint64(b)
	if product > uint64(^uint32(0)) {
		return ^uint32(0)
	}
	return uint32(product)
}

// Release refunds the attempt reserved by Charge after a successful login, so
// repeated sign-ins never exhaust the window.
func (l *Limiter[K]) Release(key K) {
	l.mu.Lock()
	defer l.mu.Unlock()

	if entry, ok := l.perKey[key]; ok {
		if entry.count > 0 {
			entry.count--
		}
		if entry.count == 0 {
			delete(l.perKey, key)
		}
		return
	}
	// Charged to the overflow bucket; saturating in case its window rolled over.
	if l.overflow.count > 0 {
		l.overflow.count--
	}
}

// Clear drops key's bucket entirely: the account limiter's success path, since
// the credential is proven. The per-IP limiter must not use it: a success says
// nothing about other clients behind the same NAT or proxy.
func (l *Limiter[K]) Clear(key K) {
	l.mu.Lock()
	defer l.mu.Unlock()
	delete(l.perKey, key)
}

// AccountKey is the account limiter's key: the submitted username, not a
// resolved id, so an unknown name throttles exactly like a real one (otherwise
// a username oracle). Only length is bounded, never case.
func AccountKey(username string) string {
	runes := 0
	for index := range username {
		if runes == accountKeyMaxChars {
			return username[:index]
		}
		runes++
	}
	return username
}
