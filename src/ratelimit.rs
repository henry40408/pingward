//! Fixed-window limiters for `POST /login`, sharing one generic [`RateLimiter`]:
//!
//! - per client IP ([`rate_limit_key`]): [`MAX_ATTEMPTS`] per [`WINDOW_SECS`].
//! - per account ([`account_key`]): [`ACCOUNT_MAX_ATTEMPTS`] per
//!   [`ACCOUNT_WINDOW_SECS`] — catches a distributed attack, where N addresses
//!   would otherwise buy `MAX_ATTEMPTS × N` guesses at one account.
//!
//! In-memory and per-process (a DB counter would add a write per attempt): each
//! replica counts separately, and a restart resets every counter.
//!
//! Invariants, each pinned by a test:
//!
//! 1. Key on the *rightmost* `X-Forwarded-For` hop; the leftmost is client-chosen.
//! 2. Check and record under one lock, or concurrent requests all pass.
//! 3. At the key cap, prune expired windows — never clear live counters, or a
//!    throttled caller resets itself by spraying fresh keys.
//! 4. A caller with no room for its own bucket is charged to a shared overflow
//!    bucket rather than waved through unmetered.

use axum::http::HeaderMap;
use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Mutex;
use std::time::Instant;

/// Attempts allowed per client within [`WINDOW_SECS`].
pub const MAX_ATTEMPTS: u32 = 5;
pub const WINDOW_SECS: u64 = 60;

/// Attempts against one account within [`ACCOUNT_WINDOW_SECS`], from any address.
/// Looser than [`MAX_ATTEMPTS`] because exhausting it locks out the owner — an
/// accepted denial-of-service primitive for anyone who knows a username. A success
/// [`clear`](RateLimiter::clear)s it; the window expires on its own and a restart
/// resets it.
pub const ACCOUNT_MAX_ATTEMPTS: u32 = 10;
pub const ACCOUNT_WINDOW_SECS: u64 = 900;

/// Hard cap on tracked keys, so a spray of addresses or usernames cannot grow the map.
const MAX_ENTRIES: usize = 10_000;

/// Longest key [`account_key`] keeps: the username is unauthenticated input, and
/// [`MAX_ENTRIES`] oversized keys would be a memory-exhaustion lever.
const ACCOUNT_KEY_MAX_CHARS: usize = 64;

/// Shared overflow bucket size as a multiple of `max_attempts`: generous so a spray
/// is not a global lockout, finite so it does not buy unlimited guesses.
const OVERFLOW_FACTOR: u32 = 10;

/// Everything the limiter mutates, behind one lock, so finding the map full and
/// charging the overflow bucket are atomic (invariant 2).
struct Buckets<K> {
    per_key: HashMap<K, (u32, Instant)>,
    /// `(attempts, window start)` shared by keys arriving while `per_key` is full.
    overflow: (u32, Instant),
}

/// Charge one attempt to a `(count, window start)` counter; returns whether allowed.
fn charge(counter: &mut (u32, Instant), max: u32, window_secs: u64) -> bool {
    if counter.1.elapsed().as_secs() >= window_secs {
        *counter = (1, Instant::now());
        return true;
    }
    if counter.0 >= max {
        return false;
    }
    counter.0 += 1;
    true
}

/// Fixed-window limiter keyed by `K`; generic so both login limiters share the
/// invariants in the module docs.
pub struct RateLimiter<K> {
    buckets: Mutex<Buckets<K>>,
    max_attempts: u32,
    window_secs: u64,
    /// [`MAX_ENTRIES`], as a field so tests can lower it.
    max_entries: usize,
}

impl<K: Eq + Hash> RateLimiter<K> {
    pub fn new(max_attempts: u32, window_secs: u64) -> Self {
        Self {
            buckets: Mutex::new(Buckets {
                per_key: HashMap::new(),
                overflow: (0, Instant::now()),
            }),
            max_attempts,
            window_secs,
            max_entries: MAX_ENTRIES,
        }
    }

    /// Reserve an attempt for `key`, returning whether it may proceed. Taken before
    /// the credential check; a success hands it back via [`release`](Self::release)
    /// or [`clear`](Self::clear).
    pub fn try_acquire(&self, key: K) -> bool {
        // Recover from poisoning: one panic must not break `POST /login` for good.
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if buckets.per_key.len() >= self.max_entries && !buckets.per_key.contains_key(&key) {
            let window_secs = self.window_secs;
            buckets
                .per_key
                .retain(|_, (_, started)| started.elapsed().as_secs() < window_secs);
            if buckets.per_key.len() >= self.max_entries {
                // All live: leave them alone (invariant 3), charge the shared
                // bucket (invariant 4).
                let max = self.max_attempts.saturating_mul(OVERFLOW_FACTOR);
                return charge(&mut buckets.overflow, max, window_secs);
            }
        }
        let entry = buckets.per_key.entry(key).or_insert((0, Instant::now()));
        charge(entry, self.max_attempts, self.window_secs)
    }

    /// Refund the attempt reserved by [`try_acquire`](Self::try_acquire) after a
    /// successful login, so repeated sign-ins never exhaust the window.
    pub fn release(&self, key: &K) {
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = buckets.per_key.get_mut(key) {
            entry.0 = entry.0.saturating_sub(1);
            if entry.0 == 0 {
                buckets.per_key.remove(key);
            }
            return;
        }
        // Charged to the overflow bucket; saturating in case its window rolled over.
        buckets.overflow.0 = buckets.overflow.0.saturating_sub(1);
    }

    /// Drop `key`'s bucket entirely: the account limiter's success path, since the
    /// credential is proven (a refund would leave nine typos one short of lockout).
    /// The per-IP limiter must not use this — a success says nothing about other
    /// clients behind the same NAT or proxy.
    pub fn clear(&self, key: &K) {
        self.buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .per_key
            .remove(key);
    }
}

/// The account limiter's key: the *submitted* username, not a resolved id, so an
/// unknown name throttles exactly like a real one (otherwise a username oracle).
/// Only length is bounded, never case: `find_user_by_username` matches exactly.
pub fn account_key(username: &str) -> String {
    username.chars().take(ACCOUNT_KEY_MAX_CHARS).collect()
}

/// Client address used as the login rate-limit key.
///
/// Do NOT unify with [`crate::auth::client_ip`], which takes the *leftmost*
/// `X-Forwarded-For` entry — fine for attribution, but under an appending proxy
/// (nginx `$proxy_add_x_forwarded_for`, Caddy) it is client-supplied, so an attacker
/// would mint a fresh bucket per request. This takes the *rightmost* hop of the
/// *last* header line (a proxy may append a new line), trusted only when the peer
/// passes [`crate::auth::is_trusted_proxy`]; assumes exactly one proxy. No peer
/// (no `ConnectInfo`) falls back to a shared loopback bucket.
pub fn rate_limit_key(
    peer: Option<IpAddr>,
    headers: &HeaderMap,
    trusted_proxies: &[String],
) -> IpAddr {
    let Some(peer) = peer else {
        return IpAddr::V4(Ipv4Addr::LOCALHOST);
    };
    let peer = peer.to_canonical();
    if crate::auth::is_trusted_proxy(trusted_proxies, peer)
        && let Some(value) = headers.get_all("x-forwarded-for").iter().next_back()
        && let Ok(raw) = value.to_str()
        && let Some(last) = raw.rsplit(',').next()
        && let Ok(ip) = last.trim().parse::<IpAddr>()
    {
        return ip.to_canonical();
    }
    peer
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, last))
    }

    #[test]
    fn allows_up_to_max_attempts() {
        let limiter = RateLimiter::new(5, 60);
        let addr = ip(1);
        for _ in 0..5 {
            assert!(limiter.try_acquire(addr));
        }
        assert!(!limiter.try_acquire(addr));
    }

    #[test]
    fn window_expiry_resets_the_counter() {
        // A zero-second window is elapsed the moment it is recorded.
        let limiter = RateLimiter::new(1, 0);
        let addr = ip(2);
        assert!(limiter.try_acquire(addr));
        assert!(limiter.try_acquire(addr));
    }

    #[test]
    fn distinct_ips_have_independent_buckets() {
        let limiter = RateLimiter::new(1, 60);
        assert!(limiter.try_acquire(ip(3)));
        assert!(!limiter.try_acquire(ip(3)));
        assert!(limiter.try_acquire(ip(4)));
    }

    #[test]
    fn release_returns_the_reserved_attempt() {
        let limiter = RateLimiter::new(2, 60);
        let addr = ip(5);
        for _ in 0..10 {
            assert!(limiter.try_acquire(addr));
            limiter.release(&addr);
        }
        assert!(
            limiter
                .buckets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .per_key
                .is_empty()
        );
    }

    #[test]
    fn a_string_keyed_limiter_counts_per_key() {
        let limiter: RateLimiter<String> = RateLimiter::new(2, 60);
        assert!(limiter.try_acquire("alice".into()));
        assert!(limiter.try_acquire("alice".into()));
        assert!(!limiter.try_acquire("alice".into()));
        // A lockout is never global.
        assert!(limiter.try_acquire("bob".into()));
    }

    #[test]
    fn clear_empties_the_bucket_where_release_refunds_one() {
        let refunded: RateLimiter<String> = RateLimiter::new(10, 60);
        let cleared: RateLimiter<String> = RateLimiter::new(10, 60);
        for _ in 0..9 {
            assert!(refunded.try_acquire("alice".into()));
            assert!(cleared.try_acquire("alice".into()));
        }
        // The tenth attempt is the successful sign-in.
        assert!(refunded.try_acquire("alice".into()));
        refunded.release(&"alice".to_string());
        assert!(cleared.try_acquire("alice".into()));
        cleared.clear(&"alice".to_string());

        // Refunded: one attempt left before the lockout bites again.
        assert!(refunded.try_acquire("alice".into()));
        assert!(!refunded.try_acquire("alice".into()));
        // Cleared: the whole window is available again.
        for _ in 0..10 {
            assert!(cleared.try_acquire("alice".into()));
        }
        assert!(!cleared.try_acquire("alice".into()));
    }

    #[test]
    fn clear_on_an_untracked_key_is_a_no_op() {
        let limiter: RateLimiter<String> = RateLimiter::new(2, 60);
        limiter.clear(&"never-seen".to_string());
        assert!(limiter.try_acquire("never-seen".into()));
    }

    #[test]
    fn account_key_bounds_its_length_without_touching_case() {
        assert_eq!(account_key("Alice"), "Alice");
        assert_ne!(account_key("Alice"), account_key("alice"));
        assert_eq!(account_key("  bob  "), "  bob  ");

        let huge = "x".repeat(10_000);
        assert_eq!(account_key(&huge).chars().count(), ACCOUNT_KEY_MAX_CHARS);
        let cjk = "漢".repeat(10_000);
        assert_eq!(account_key(&cjk).chars().count(), ACCOUNT_KEY_MAX_CHARS);
    }

    #[test]
    fn map_is_pruned_at_capacity() {
        let mut limiter = RateLimiter::new(5, 60);
        limiter.max_entries = 4;
        for last in 0..50u8 {
            limiter.try_acquire(ip(last));
        }
        assert!(
            limiter
                .buckets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .per_key
                .len()
                <= 4
        );
    }

    /// Unlike `map_is_pruned_at_capacity` (60s window, nothing expires), every
    /// window here has elapsed, so the length must actually drop below the cap.
    #[test]
    fn expired_entries_are_pruned_when_capacity_is_reached() {
        let mut limiter = RateLimiter::new(5, 0);
        limiter.max_entries = 4;
        for last in 0..4u8 {
            assert!(limiter.try_acquire(ip(last)));
        }
        assert_eq!(
            4,
            limiter
                .buckets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .per_key
                .len()
        );

        // At capacity with every window elapsed: a fresh key must prune.
        assert!(limiter.try_acquire(ip(99)));
        let len = limiter
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .per_key
            .len();
        assert!(len < 4, "map was not pruned: len = {len}");
    }

    /// Invariant 3.
    #[test]
    fn capacity_spray_does_not_reset_an_existing_counter() {
        let mut limiter = RateLimiter::new(1, 60);
        limiter.max_entries = 4;
        let victim = ip(200);
        assert!(limiter.try_acquire(victim));
        assert!(!limiter.try_acquire(victim));

        for last in 0..50u8 {
            limiter.try_acquire(ip(last));
        }
        assert!(!limiter.try_acquire(victim), "spray reset the counter");
    }

    /// Invariant 2.
    #[test]
    fn concurrent_attempts_cannot_exceed_the_limit() {
        const THREADS: usize = 64;
        let limiter = Arc::new(RateLimiter::new(5, 60));
        let barrier = Arc::new(Barrier::new(THREADS));
        let addr = ip(7);

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let limiter = Arc::clone(&limiter);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    limiter.try_acquire(addr)
                })
            })
            .collect();
        let allowed = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread panicked"))
            .filter(|allowed| *allowed)
            .count();
        assert_eq!(5, allowed, "concurrent requests overran the limit");
    }

    /// Invariant 4: the shared bucket must eventually refuse.
    #[test]
    fn the_overflow_bucket_is_finite() {
        const MAX: u32 = 2;
        let mut limiter = RateLimiter::new(MAX, 60);
        limiter.max_entries = 4;
        // Live windows everywhere, so every further address overflows.
        for last in 0..4u8 {
            assert!(limiter.try_acquire(ip(last)));
        }
        let budget = MAX * OVERFLOW_FACTOR;
        for n in 0..budget {
            assert!(
                limiter.try_acquire(ip(100 + u8::try_from(n).unwrap())),
                "attempt {n} is inside the shared budget"
            );
        }
        assert!(
            !limiter.try_acquire(ip(200)),
            "the shared overflow budget must run out"
        );
        // Not a global lockout: an address with its own bucket still works.
        limiter.release(&ip(0));
        assert!(limiter.try_acquire(ip(0)));
    }

    #[test]
    fn the_overflow_bucket_refills_with_its_window() {
        let mut limiter = RateLimiter::new(1, 0); // zero-second window
        limiter.max_entries = 0; // every address overflows
        for _ in 0..(OVERFLOW_FACTOR * 3) {
            assert!(limiter.try_acquire(ip(1)));
        }
    }

    #[test]
    fn release_refunds_the_overflow_bucket() {
        let mut limiter = RateLimiter::new(1, 60);
        limiter.max_entries = 4;
        for last in 0..4u8 {
            assert!(limiter.try_acquire(ip(last)));
        }
        let addr = ip(150);
        for _ in 0..(OVERFLOW_FACTOR * 3) {
            assert!(limiter.try_acquire(addr));
            limiter.release(&addr);
        }
        assert_eq!(
            0,
            limiter
                .buckets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .overflow
                .0
        );
    }

    fn trusted(patterns: &[&str]) -> Vec<String> {
        patterns.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn rate_limit_key_prefers_xff_only_from_a_trusted_proxy() {
        let proxies = trusted(&["10.0.0.1"]);
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.9, 10.0.0.1".parse().unwrap());

        let trusted_peer = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let appended = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(
            appended,
            rate_limit_key(Some(trusted_peer), &headers, &proxies)
        );

        let stranger = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));
        assert_eq!(stranger, rate_limit_key(Some(stranger), &headers, &proxies));
    }

    /// Invariant 1.
    #[test]
    fn spoofed_leading_xff_hops_do_not_change_the_key() {
        let proxies = trusted(&["10.0.0.1"]);
        let trusted_peer = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let real = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7));
        for spoof in ["1.2.3.4", "5.6.7.8", "9.9.9.9, 8.8.8.8"] {
            let mut headers = HeaderMap::new();
            headers.insert(
                "x-forwarded-for",
                format!("{spoof}, 198.51.100.7").parse().unwrap(),
            );
            assert_eq!(real, rate_limit_key(Some(trusted_peer), &headers, &proxies));
        }
    }

    #[test]
    fn rate_limit_key_uses_the_last_xff_header_line() {
        let proxies = trusted(&["10.0.0.1"]);
        let trusted_peer = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let mut headers = HeaderMap::new();
        // Attacker-supplied, spoofing an unrelated hop.
        headers.append("x-forwarded-for", "9.9.9.9, 8.8.8.8".parse().unwrap());
        // Appended by the trusted proxy itself.
        headers.append("x-forwarded-for", "198.51.100.7".parse().unwrap());
        assert_eq!(
            IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)),
            rate_limit_key(Some(trusted_peer), &headers, &proxies)
        );
    }

    #[test]
    fn rate_limit_key_ignores_garbage_xff_from_a_trusted_proxy() {
        let proxies = trusted(&["10.0.0.1"]);
        let trusted_peer = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "not-an-ip".parse().unwrap());
        assert_eq!(
            trusted_peer,
            rate_limit_key(Some(trusted_peer), &headers, &proxies)
        );
    }

    #[test]
    fn rate_limit_key_falls_back_to_loopback_with_no_peer() {
        let proxies = trusted(&["10.0.0.1"]);
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.9".parse().unwrap());
        assert_eq!(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            rate_limit_key(None, &headers, &proxies)
        );
        assert_eq!(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            rate_limit_key(None, &HeaderMap::new(), &proxies)
        );
    }

    #[test]
    fn rate_limit_key_matches_a_v4_mapped_peer_against_a_v4_pattern() {
        let proxies = trusted(&["10.0.0.1"]);
        let mapped: IpAddr = "::ffff:10.0.0.1".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.9".parse().unwrap());
        assert_eq!(
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)),
            rate_limit_key(Some(mapped), &headers, &proxies)
        );
    }

    #[test]
    fn rate_limit_key_honours_ipv6_and_ignores_garbage_headers_when_untrusted() {
        let proxies = trusted(&["10.0.0.1"]);
        let v6_loopback = IpAddr::V6(Ipv6Addr::LOCALHOST);
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "not-an-ip".parse().unwrap());
        assert_eq!(
            v6_loopback,
            rate_limit_key(Some(v6_loopback), &headers, &proxies)
        );
    }
}
