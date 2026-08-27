//! Short-lived per-session elevation for admin actions that *grant* access.
//!
//! `/admin`'s access-granting controls are single-button inline forms with no
//! room for a per-action password field (the shape `web::reauthenticate` uses
//! on `/account`), so re-authentication is decoupled: an admin unlocks once via
//! `POST /admin/unlock` and the gated handlers check that the unlock is fresh.
//!
//! State is in-memory and per-process, which needs no migration: a restart or a
//! second replica just means entering the password again.
//!
//! Keyed by the session's SHA-256 handle (`crate::apikey::hash_api_key`), never
//! the raw session id — the id is the bearer secret. Per session, not per user:
//! elevating one browser must not elevate another signed in as the same admin.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// How long an unlock lasts: long enough for a batch of user administration,
/// short enough that a browser walked away from re-locks itself.
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
        // Recover from poisoning: one panicking request under this lock must
        // not turn every later admin action into a 500.
        self.granted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Start (or restart) `handle`'s window, sweeping expired entries. That is
    /// all the pruning needed: an entry costs a successful password check, so
    /// the map is bounded by live sessions.
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
        // The window boundary without waiting for one: at ttl 0 elapsed is
        // already >= the window, so the answer must be None, not Some(0).
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
        // Revoking something untracked is a no-op; `logout` calls it
        // unconditionally.
        e.revoke("never-seen");
    }

    #[test]
    fn granting_prunes_expired_entries() {
        let e = Elevations::new(0);
        e.grant("stale");
        e.grant("fresh");
        // The zero-length window means "stale" was already expired when
        // "fresh" was granted, so it must have been swept.
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
