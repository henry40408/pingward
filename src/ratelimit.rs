//! Fixed-window limiters for `POST /login`, keyed two different ways.
//!
//! [`RateLimiter`] is generic over its key so the two cannot diverge in how a
//! window rolls over or how the tracked-key cap behaves:
//!
//! - per client IP (`RateLimiter<IpAddr>`, keyed by [`rate_limit_key`]) —
//!   [`MAX_ATTEMPTS`] per [`WINDOW_SECS`]. Stops one source grinding through a
//!   dictionary.
//! - per account (`RateLimiter<String>`, keyed by [`account_key`]) —
//!   [`ACCOUNT_MAX_ATTEMPTS`] per [`ACCOUNT_WINDOW_SECS`]. Stops a distributed
//!   attack the per-IP limiter cannot see at all: N addresses otherwise buy
//!   `MAX_ATTEMPTS × N` guesses against one account. OWASP's Authentication
//!   Cheat Sheet asks for an account-associated counter for this reason; see
//!   [`ACCOUNT_MAX_ATTEMPTS`] for the denial-of-service trade-off.
//!
//! State is in-memory and per-process: a DB-backed counter would mean a write
//! per login attempt, which on `SQLite` contends with the scan loop's writer for
//! the same connection budget (see the `busy_timeout` comment in `src/db.rs`).
//! Same limitation as `AppState::events` (see ARCHITECTURE.md): each replica
//! counts separately, and a restart resets every counter.
//!
//! Four defects of the reference implementation this was ported from, each
//! closed here and pinned by a test:
//!
//! 1. It keyed on the leftmost `X-Forwarded-For` hop, which is attacker-chosen
//!    under a stock appending proxy (nginx's `$proxy_add_x_forwarded_for`,
//!    Caddy's `reverse_proxy`), so an attacker could mint a fresh bucket per
//!    request. [`rate_limit_key`] reads the rightmost hop.
//! 2. It split the check and the record across two lock acquisitions, so
//!    concurrent requests could all observe the pre-attack count.
//!    [`RateLimiter::try_acquire`] does both under one lock.
//! 3. At the tracked-key cap it called `map.clear()`, handing anyone already
//!    throttled a way to reset their own budget by spraying fresh keys.
//!    Expired windows are pruned instead and other counters left alone.
//! 4. At capacity an address with no bucket of its own was waved through
//!    unmetered, so holding [`MAX_ENTRIES`] live windows bought an unlimited
//!    budget from every further address. Such a caller is charged to a shared
//!    overflow bucket ([`Buckets::overflow`]) — still fail-open, but finitely.

use axum::http::HeaderMap;
use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Mutex;
use std::time::Instant;

/// Attempts allowed per client within [`WINDOW_SECS`].
pub const MAX_ATTEMPTS: u32 = 5;
pub const WINDOW_SECS: u64 = 60;

/// Attempts allowed against one account within [`ACCOUNT_WINDOW_SECS`],
/// however many addresses they arrive from.
///
/// Looser than [`MAX_ATTEMPTS`] and over a longer window because the failure
/// modes are opposite: exhausting a per-IP bucket inconveniences that address,
/// exhausting an account bucket locks out the owner. An account lockout is
/// therefore a denial-of-service primitive handed to whoever knows a username.
/// 10 per 15 minutes is too few for credential stuffing to be worth running,
/// while an owner has to fail ten times in fifteen minutes to feel it, and any
/// success clears the counter ([`RateLimiter::clear`] rather than
/// [`RateLimiter::release`]).
///
/// It is a rolling window, not a latch: nothing has to be unlocked. Two escape
/// hatches for an operator locked out on purpose — the state is per-process,
/// so a restart clears it, and forward-auth deployments have no password
/// login at all.
pub const ACCOUNT_MAX_ATTEMPTS: u32 = 10;
pub const ACCOUNT_WINDOW_SECS: u64 = 900;

/// Hard cap on tracked keys. At the cap expired windows are pruned first, so a
/// spray of addresses (or invented usernames) cannot grow the map unbounded.
const MAX_ENTRIES: usize = 10_000;

/// Longest account key retained by [`account_key`]. The username arrives
/// unvalidated on an unauthenticated form and becomes a `HashMap` key held
/// until its window elapses; unbounded, `MAX_ENTRIES` oversized keys would be
/// a memory-exhaustion lever.
const ACCOUNT_KEY_MAX_CHARS: usize = 64;

/// Size of the shared overflow bucket, as a multiple of `max_attempts`. Only
/// reachable once [`MAX_ENTRIES`] addresses hold a live window at once. The
/// dial between charging too little (a legitimate sign-in during a spray is
/// refused) and charging nothing (the spray buys unlimited guesses).
const OVERFLOW_FACTOR: u32 = 10;

/// Everything the limiter mutates, behind one lock. The overflow counter lives
/// here rather than in its own `Mutex` so finding the map full and charging the
/// shared bucket happen under one acquisition; splitting them would reintroduce
/// defect 2 on exactly the path under attack.
struct Buckets<K> {
    per_key: HashMap<K, (u32, Instant)>,
    /// `(attempts, window start)` shared by every key that arrives while
    /// `per_key` is full of live windows.
    overflow: (u32, Instant),
}

/// Charge one attempt to a `(count, window start)` counter. Shared by the
/// per-key buckets and the overflow bucket so window roll-over cannot drift.
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

/// Fixed-window limiter for login attempts, keyed by `K`. Generic so the
/// per-IP and per-account limiters are the same code: each of the window
/// roll-over, single-lock check-and-record, tracked-key cap and overflow
/// bucket fixed a defect, and a second copy could reintroduce one.
pub struct RateLimiter<K> {
    buckets: Mutex<Buckets<K>>,
    max_attempts: u32,
    window_secs: u64,
    /// A field rather than the [`MAX_ENTRIES`] constant so tests can lower it
    /// and reach the capacity path without tracking ten thousand keys.
    max_entries: usize,
}

impl<K: Eq + Hash> RateLimiter<K> {
    pub fn new(max_attempts: u32, window_secs: u64) -> Self {
        Self {
            buckets: Mutex::new(Buckets {
                per_key: HashMap::new(),
                // Zero attempts means the first overflow caller rolls the
                // window over rather than inheriting boot time as its start.
                overflow: (0, Instant::now()),
            }),
            max_attempts,
            window_secs,
            max_entries: MAX_ENTRIES,
        }
    }

    /// Reserve an attempt for `key`, returning whether it may proceed.
    ///
    /// Checking and counting happen under a single lock (defect 2). The
    /// reservation is taken *before* the credential comparison and handed back
    /// by [`release`](Self::release) on success, so only failures ultimately
    /// consume the window.
    pub fn try_acquire(&self, key: K) -> bool {
        // Recover from poisoning: one panicking request under this lock must
        // not break `POST /login` permanently for everyone after it.
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
                // Every entry is still live: a spray from more sources than
                // the cap. Existing counters are left alone (clearing them is
                // defect 3), and this untracked source is charged to the
                // shared bucket rather than waved through unmetered, which
                // made the cap itself the bypass (defect 4). Refusing outright
                // would turn the spray into a global login lockout, hence the
                // generous [`OVERFLOW_FACTOR`].
                let max = self.max_attempts.saturating_mul(OVERFLOW_FACTOR);
                return charge(&mut buckets.overflow, max, window_secs);
            }
        }
        let entry = buckets.per_key.entry(key).or_insert((0, Instant::now()));
        charge(entry, self.max_attempts, self.window_secs)
    }

    /// Hand back the attempt reserved by [`try_acquire`](Self::try_acquire).
    /// Called after a successful login only, so repeated legitimate sign-ins
    /// never exhaust the window while an attacker's failures still count.
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
        // No bucket of its own: the attempt was charged to the overflow
        // bucket, so that is what gets the refund. A window that rolled over
        // between the two calls over-refunds by one, which the saturating
        // subtraction makes harmless.
        buckets.overflow.0 = buckets.overflow.0.saturating_sub(1);
    }

    /// Drop `key`'s bucket entirely, rather than refunding one attempt.
    ///
    /// The account limiter's success path: refunding one attempt would leave
    /// an owner who mistyped nine times and then signed in correctly one
    /// failure from a fifteen-minute lockout, credential already proven.
    ///
    /// The per-IP limiter must *not* do this — a success there says nothing
    /// about the other attempts from a shared NAT or proxy, whereas a success
    /// here proves the credential the failures were guessing at.
    ///
    /// It does reset an account spray's budget whenever the owner signs in;
    /// bounded by how often a person logs in, so a few extra guesses a day
    /// against 10 per 15 minutes.
    pub fn clear(&self, key: &K) {
        self.buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .per_key
            .remove(key);
    }
}

/// The account limiter's key: the submitted username, bounded in length.
///
/// Keyed on what the form carried, not a resolved `users.id`, and looked up
/// before the account is: an unknown username must exhaust a budget exactly as
/// a real one does, or "this request was throttled" is a username oracle.
///
/// Only the length is normalised, never the case — `find_user_by_username`
/// compares exactly on both backends, so `Alice` and `alice` need different
/// buckets. Truncation can make two long usernames share one; that direction
/// is safe and needs a 64-character common prefix.
pub fn account_key(username: &str) -> String {
    username.chars().take(ACCOUNT_KEY_MAX_CHARS).collect()
}

/// Client address used as the login rate-limit key.
///
/// Do NOT unify this with `crate::auth::client_ip`, which takes the *leftmost*
/// `X-Forwarded-For` entry: that is right for attribution but wrong for a
/// security control, since under a stock appending proxy (nginx's
/// `$proxy_add_x_forwarded_for`, Caddy's `reverse_proxy`) the leftmost entry
/// is client-supplied and an attacker mints a fresh bucket per request. This
/// reads the *rightmost* hop, the one the trusted proxy appended. That assumes
/// exactly one proxy in the chain; a longer chain would need the Nth from the
/// right, which `PINGWARD_TRUSTED_PROXIES` cannot express.
///
/// The trust gate is `crate::auth::is_trusted_proxy`, not a loopback
/// heuristic: `peer` must be `Some` and trusted before `X-Forwarded-For` is
/// believed. Peer and resolved IP are compared and returned canonically
/// (`IpAddr::to_canonical`), so an IPv4-mapped IPv6 peer matches a v4 entry.
///
/// A header name may appear on multiple lines and `HeaderMap::get` returns
/// only the first. A proxy that appends its hop as a *new* line would leave
/// that first line client-controlled, reopening the bypass one line up, so
/// this uses `get_all` and takes the last line before splitting on commas.
///
/// A missing peer (the router driven without `ConnectInfo`, as in tests) falls
/// back to a shared loopback bucket rather than disabling the limiter.
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

    /// Signing in repeatedly must never exhaust the window.
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

    // --- the account-keyed limiter ---

    /// The `RateLimiter<String>` account limiter must behave identically to
    /// the IP one.
    #[test]
    fn a_string_keyed_limiter_counts_per_key() {
        let limiter: RateLimiter<String> = RateLimiter::new(2, 60);
        assert!(limiter.try_acquire("alice".into()));
        assert!(limiter.try_acquire("alice".into()));
        assert!(!limiter.try_acquire("alice".into()));
        // A lockout is never global.
        assert!(limiter.try_acquire("bob".into()));
    }

    /// Nine failures then a success must leave the owner a full budget, not
    /// the single attempt `release` would refund.
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
        // `find_user_by_username` compares exactly on both backends, so
        // these are different accounts and must not share a bucket.
        assert_eq!(account_key("Alice"), "Alice");
        assert_ne!(account_key("Alice"), account_key("alice"));
        // Nor is anything else normalised away.
        assert_eq!(account_key("  bob  "), "  bob  ");

        // An attacker-chosen username must not become an attacker-chosen
        // allocation.
        let huge = "x".repeat(10_000);
        assert_eq!(account_key(&huge).chars().count(), ACCOUNT_KEY_MAX_CHARS);
        // Truncation is by characters, so it never splits a code point.
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

    /// `map_is_pruned_at_capacity` uses a 60-second window, so nothing in it
    /// expires and it only reaches the fail-open path. Here every window has
    /// already elapsed, so capacity must trigger a real prune: the length has
    /// to drop below the cap, not merely stay under it by never growing.
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

    /// Defect 3: a spray of fresh keys used to hit `map.clear()`, letting an
    /// already-throttled source reset its own counter.
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

    /// Defect 2: checking and counting used to take the lock separately, so
    /// requests arriving together all observed the pre-attack count.
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

    /// Defect 4: the capacity path used to admit an untracked address
    /// unmetered. The shared bucket must eventually refuse.
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

    /// A rollover must refill the shared bucket, or one spray would refuse
    /// every untracked address forever.
    #[test]
    fn the_overflow_bucket_refills_with_its_window() {
        let mut limiter = RateLimiter::new(1, 0); // zero-second window
        limiter.max_entries = 0; // every address overflows
        for _ in 0..(OVERFLOW_FACTOR * 3) {
            assert!(limiter.try_acquire(ip(1)));
        }
    }

    /// A success charged to the shared bucket must hand its attempt back.
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

    /// Defect 1: `$proxy_add_x_forwarded_for` appends the peer, so the
    /// leftmost entry is whatever the client sent — keying on it let an
    /// attacker mint a fresh bucket per request.
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

    /// A proxy that appends its hop as a new `X-Forwarded-For` line must not
    /// let the first, client-controlled line win: `HeaderMap::get` would
    /// return only that one, so `rate_limit_key` uses `get_all`.
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

    /// A dual-stack listener reports an IPv4 client as `::ffff:a.b.c.d`; the
    /// operator writes the plain v4 address in `PINGWARD_TRUSTED_PROXIES`.
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
