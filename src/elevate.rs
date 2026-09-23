//! Short-lived per-session elevation for admin actions that *grant* access.
//!
//! `/admin`'s single-button forms have no room for a password field, so an admin
//! unlocks once via `POST /admin/unlock` and gated handlers check freshness.
//! In-memory and per-process: a restart or another replica just asks again.
//! Keyed by the session's SHA-256 handle, never the raw id (the bearer secret),
//! and per session so elevating one browser does not elevate another.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// How long an unlock lasts.
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
        // Recover from poisoning: one panic must not 500 every later admin action.
        self.granted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Start (or restart) `handle`'s window, sweeping expired entries — enough
    /// pruning, since each entry costs a successful password check.
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

    /// End `handle`'s window early; called on logout.
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
        assert_eq!(e.remaining_secs("def"), None);
    }

    #[test]
    fn a_zero_ttl_never_counts_as_elevated() {
        // At the boundary the answer is None, not Some(0).
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
        // Untracked: a no-op.
        e.revoke("never-seen");
    }

    #[test]
    fn granting_prunes_expired_entries() {
        let e = Elevations::new(0);
        e.grant("stale");
        e.grant("fresh");
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
