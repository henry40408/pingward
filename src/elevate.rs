//! Short-lived per-session elevation for admin actions that *grant* access.
//!
//! `/admin`'s access-granting controls are single-button inline forms in a
//! table row (`users_toggle_admin` takes no body at all). A per-action password
//! field — the shape `web::reauthenticate` uses on `/account` — does not fit
//! there, so re-authentication is decoupled from the action: an admin unlocks
//! once via `POST /admin/unlock`, and the gated handlers check that the unlock
//! is still fresh.
//!
//! State is in-memory and per-process, matching `crate::ratelimit` and
//! `AppState::events`, and this is the one place where that carries **no**
//! meaningful cost. Elevation is deliberately short-lived, so persisting it
//! would buy at most the tail of one window; a restart or a second replica
//! simply means entering the password again, which is the safe direction for a
//! privilege gate. That is the whole reason this needs no migration.
//!
//! Keyed by the session's SHA-256 **handle** (`crate::apikey::hash_api_key`),
//! never the raw session id — the same rule `auth::session_log_handle` and
//! `/account`'s session rows follow, since the id is the bearer secret the
//! cookie signature is attached to. Per *session*, not per user: elevating one
//! browser must not elevate another that is signed in as the same admin.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// How long an unlock lasts.
///
/// Long enough to finish a batch of user administration without re-typing a
/// password between each control, short enough that a browser walked away from
/// re-locks itself — which is the case the gate exists for, since pingward's
/// CSRF and CSP already close the other routes to a forged admin action.
pub const ELEVATION_TTL_SECS: u64 = 900;

/// Live elevations, keyed by session handle.
pub struct Elevations {
    granted: Mutex<HashMap<String, Instant>>,
    ttl_secs: u64,
}

impl Elevations {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            granted: Mutex::new(HashMap::new()),
            ttl_secs,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Instant>> {
        // Recover from poisoning rather than `unwrap()`, exactly as
        // `ratelimit::RateLimiter` does: one panicking request under this lock
        // must not turn every later admin action into a 500.
        self.granted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Start (or restart) `handle`'s window.
    ///
    /// Expired entries are dropped on the way through, which is all the pruning
    /// this map needs: an entry requires a *successful* password check against a
    /// real account, so its size is bounded by live sessions rather than by
    /// anything an attacker chooses.
    pub fn grant(&self, handle: &str) {
        let mut granted = self.lock();
        let ttl = self.ttl_secs;
        granted.retain(|_, at| at.elapsed().as_secs() < ttl);
        granted.insert(handle.to_owned(), Instant::now());
    }

    /// Seconds left on `handle`'s window, or `None` if it has none.
    pub fn remaining_secs(&self, handle: &str) -> Option<u64> {
        let granted = self.lock();
        let elapsed = granted.get(handle)?.elapsed().as_secs();
        self.ttl_secs.checked_sub(elapsed).filter(|left| *left > 0)
    }

    /// End `handle`'s window early. Called on logout and when the session is
    /// revoked, so a re-signed-in browser starts locked.
    pub fn revoke(&self, handle: &str) {
        self.lock().remove(handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_granted_handle_is_elevated_and_others_are_not() {
        let e = Elevations::new(ELEVATION_TTL_SECS);
        assert_eq!(e.remaining_secs("abc"), None);
        e.grant("abc");
        assert!(e.remaining_secs("abc").is_some());
        // Per session, not per user: another browser is unaffected.
        assert_eq!(e.remaining_secs("def"), None);
    }

    #[test]
    fn a_zero_ttl_never_counts_as_elevated() {
        // The window boundary, without waiting for one: at ttl 0 the elapsed
        // time is already >= the window, so `remaining_secs` must report
        // nothing rather than `Some(0)`.
        let e = Elevations::new(0);
        e.grant("abc");
        assert_eq!(e.remaining_secs("abc"), None);
    }

    #[test]
    fn revoke_ends_the_window_immediately() {
        let e = Elevations::new(ELEVATION_TTL_SECS);
        e.grant("abc");
        e.revoke("abc");
        assert_eq!(e.remaining_secs("abc"), None);
        // Revoking something untracked is a no-op, not a panic — `logout`
        // calls it unconditionally.
        e.revoke("never-seen");
    }

    #[test]
    fn granting_prunes_expired_entries() {
        let e = Elevations::new(0);
        e.grant("stale");
        e.grant("fresh");
        // The zero-length window means "stale" was already expired when
        // "fresh" was granted, so it must have been swept rather than kept
        // forever.
        assert!(!e.lock().contains_key("stale"));
    }

    #[test]
    fn a_second_grant_restarts_the_window() {
        let e = Elevations::new(ELEVATION_TTL_SECS);
        e.grant("abc");
        let first = e.remaining_secs("abc").unwrap();
        e.grant("abc");
        assert!(e.remaining_secs("abc").unwrap() >= first);
    }
}
