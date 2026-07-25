//! Per-client-IP fixed-window limiter for `POST /login`.
//!
//! State is in-memory and per-process **on purpose**. A DB-backed counter
//! would mean a write per login attempt — on `SQLite` that contends with the
//! scan loop's writer for the same connection budget (see the `busy_timeout`
//! comment in `src/db.rs`), and buys nothing on Postgres either. This carries
//! the same limitation as `AppState::events` (see `ARCHITECTURE.md`'s
//! "Live-tail signal bus" section): a multi-replica deployment counts each
//! replica separately (effective budget is `MAX_ATTEMPTS * replicas`), and a
//! restart resets every counter to zero.
//!
//! This is a **corrected** port of an earlier reference implementation
//! (`RateLimiter` in a sibling project's `src/admin/auth.rs`) that had three
//! defects, each fixed here:
//!
//! 1. It keyed on the **leftmost** `X-Forwarded-For` hop, which is
//!    attacker-chosen under a stock appending proxy (nginx's
//!    `$proxy_add_x_forwarded_for`, Caddy's `reverse_proxy`) — an attacker
//!    could mint a fresh bucket per request. Fixed by [`rate_limit_key`]
//!    reading the **rightmost** hop instead.
//! 2. It split the check and the record across two lock acquisitions, so
//!    concurrent requests could all observe the pre-attack count and overrun
//!    the limit. Fixed by [`RateLimiter::try_acquire`] doing both under one
//!    lock.
//! 3. At the tracked-client cap it called `map.clear()`, handing anyone
//!    already throttled a way to reset their own budget by spraying fresh
//!    keys. Fixed by pruning expired windows and otherwise leaving existing
//!    counters alone.

use axum::http::HeaderMap;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Mutex;
use std::time::Instant;

/// Attempts allowed within [`WINDOW_SECS`] before `POST /login` starts
/// refusing service for a given client.
pub const MAX_ATTEMPTS: u32 = 5;
/// Width, in seconds, of the fixed window `MAX_ATTEMPTS` is counted over.
pub const WINDOW_SECS: u64 = 60;

/// Hard cap on tracked IPs, used as the default for `RateLimiter::max_entries`.
/// When the cap is hit the map is pruned of expired windows first, so a spray
/// from many source addresses cannot grow it without bound.
const MAX_ENTRIES: usize = 10_000;

/// Per-client-IP fixed-window limiter for login attempts.
pub struct RateLimiter {
    attempts: Mutex<HashMap<IpAddr, (u32, Instant)>>,
    max_attempts: u32,
    window_secs: u64,
    /// Kept as a field (rather than the [`MAX_ENTRIES`] constant) purely so
    /// tests can lower it and exercise the capacity path without tracking
    /// ten thousand addresses.
    max_entries: usize,
}

impl RateLimiter {
    /// Create a limiter allowing `max_attempts` within `window_secs`.
    pub fn new(max_attempts: u32, window_secs: u64) -> Self {
        Self {
            attempts: Mutex::new(HashMap::new()),
            max_attempts,
            window_secs,
            max_entries: MAX_ENTRIES,
        }
    }

    /// Reserve an attempt for `ip`, returning whether it may proceed.
    ///
    /// Checking and counting happen under a single lock, so concurrent
    /// requests cannot all observe the pre-attack count (defect 2 of the
    /// original implementation, see the module doc). The reservation is
    /// taken *before* the credential comparison — which costs an argon2
    /// verification — and is handed back by [`release`](Self::release) when
    /// the credentials turn out to be valid, so only failures ultimately
    /// consume the window.
    pub fn try_acquire(&self, ip: IpAddr) -> bool {
        // (a) `std::sync::Mutex`, not `parking_lot::Mutex` — pingward has no
        // dependency on `parking_lot` and this task must not add one.
        // Recovering from poisoning (rather than `unwrap()`) means one
        // panicking request under this lock can't turn `POST /login` into a
        // permanently broken endpoint for everyone after it; clippy pedantic
        // also flags a bare `unwrap()` here.
        let mut map = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if map.len() >= self.max_entries && !map.contains_key(&ip) {
            let window_secs = self.window_secs;
            map.retain(|_, (_, started)| started.elapsed().as_secs() < window_secs);
            if map.len() >= self.max_entries {
                // Every entry is still live: a spray from more distinct
                // sources than the cap. Leave the existing counters alone and
                // let this untracked source through. Clearing the map instead
                // would hand anyone already throttled a way to reset their own
                // budget (defect 3, see the module doc); refusing would turn
                // the same spray into a global login lockout. Whoever can
                // hold this many live buckets already commands more addresses
                // than the limit meaningfully bounds.
                return true;
            }
        }
        let entry = map.entry(ip).or_insert((0, Instant::now()));
        if entry.1.elapsed().as_secs() >= self.window_secs {
            *entry = (1, Instant::now());
            return true;
        }
        if entry.0 >= self.max_attempts {
            return false;
        }
        entry.0 += 1;
        true
    }

    /// Hand back the attempt reserved by [`try_acquire`](Self::try_acquire).
    ///
    /// Called after a *successful* login only. The control exists to stop
    /// password guessing, and a legitimate user who signs in repeatedly (new
    /// device, cleared cookies, a test suite) should not be locked out by it;
    /// an attacker's every attempt is a failure, so their budget is
    /// unchanged.
    pub fn release(&self, ip: IpAddr) {
        let mut map = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = map.get_mut(&ip) {
            entry.0 = entry.0.saturating_sub(1);
            if entry.0 == 0 {
                map.remove(&ip);
            }
        }
    }
}

/// Client address used as the login rate-limit key.
///
/// (c) This is deliberately **not** `crate::ping::ClientIp` /
/// `crate::auth::client_ip`. `client_ip` takes the **leftmost**
/// `X-Forwarded-For` entry, which is correct for *attribution* — stamping
/// which client a session or ping belongs to for a human to read later — but
/// wrong for a *security control*: under a stock appending proxy (nginx's
/// `$proxy_add_x_forwarded_for`, Caddy's `reverse_proxy`) the leftmost entry
/// is whatever the client itself sent, so an attacker could mint a fresh
/// bucket per request and bypass the limit entirely. This function therefore
/// reads the **rightmost** hop instead — the one appended by the trusted
/// proxy itself. This divergence from `client_ip` is intentional; a future
/// reader must not "unify" the two. (This assumes exactly one trusted proxy
/// in the chain; a longer chain would need the Nth-from-the-right, which
/// `PINGWARD_TRUSTED_PROXIES` does not currently express.)
///
/// (b) The trust gate is pingward's own configured trusted proxies
/// (`crate::auth::is_trusted_proxy`), not a loopback heuristic: `peer` must
/// be `Some` and a member of `trusted_proxies` before `X-Forwarded-For` is
/// believed at all. Both the peer and the resolved header IP are compared and
/// returned in canonical form (`IpAddr::to_canonical`), matching how
/// `crate::auth::client_ip` and `crate::auth::is_trusted_proxy` already
/// normalise an IPv4-mapped IPv6 peer.
///
/// (d) HTTP permits a header name to appear on multiple lines, and
/// `HeaderMap::get` returns only the *first* one. A proxy that appends its
/// own hop as a **new** `X-Forwarded-For` line — rather than extending the
/// client-supplied line with a comma, as `$proxy_add_x_forwarded_for` does —
/// would leave that first line entirely client-controlled, reopening exactly
/// the bypass defect 1 fixes, just one header line up. This function
/// therefore reads `headers.get_all("x-forwarded-for")` and takes the
/// **last** line before splitting *that* line on commas for the rightmost
/// hop.
///
/// A missing peer (`None`) — the router driven without `ConnectInfo`, as in
/// tests; see `src/main.rs`'s `into_make_service_with_connect_info` — falls
/// back to a single shared loopback bucket rather than disabling the limiter.
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

    /// A successful login hands its reservation back, so signing in
    /// repeatedly never exhausts the window.
    #[test]
    fn release_returns_the_reserved_attempt() {
        let limiter = RateLimiter::new(2, 60);
        let addr = ip(5);
        for _ in 0..10 {
            assert!(limiter.try_acquire(addr));
            limiter.release(addr);
        }
        assert!(
            limiter
                .attempts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        );
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
                .attempts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
                <= 4
        );
    }

    /// `map_is_pruned_at_capacity` above uses a 60-second window, so nothing
    /// in it ever expires and it only exercises the fail-open `return true`
    /// path (no prune, no insert). Here every tracked window has already
    /// elapsed (a zero-second window, as in `window_expiry_resets_the_counter`),
    /// so hitting capacity must actually prune the map — its length must drop
    /// below the cap, not merely stay at or under it by never growing.
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
                .attempts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
        );

        // The map is now at capacity and every entry's window has already
        // elapsed. A fresh key must trigger a real prune.
        assert!(limiter.try_acquire(ip(99)));
        let len = limiter
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        assert!(len < 4, "map was not pruned: len = {len}");
    }

    /// Regression for defect 3: a spray of fresh keys used to hit
    /// `map.clear()`, handing an already-throttled source a way to reset its
    /// own counter.
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

    /// Regression for defect 2: checking and counting used to take the lock
    /// separately, so requests arriving together all observed the
    /// pre-attack count.
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

    /// Regression for defect 1: nginx's stock `$proxy_add_x_forwarded_for`
    /// appends the peer, so the leftmost entry is whatever the client sent.
    /// Keying on it let an attacker mint a fresh bucket per request.
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

    /// Regression for defect (d), see the module doc: a proxy that appends
    /// its own hop as a **new** `X-Forwarded-For` header line — rather than
    /// extending the existing line with a comma — must not let the first,
    /// fully client-controlled line win. `HeaderMap::get` would return only
    /// that first line; `rate_limit_key` must use `get_all` and take the
    /// last one instead.
    #[test]
    fn rate_limit_key_uses_the_last_xff_header_line() {
        let proxies = trusted(&["10.0.0.1"]);
        let trusted_peer = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let mut headers = HeaderMap::new();
        // First line: entirely attacker-supplied, spoofing an unrelated hop.
        headers.append("x-forwarded-for", "9.9.9.9, 8.8.8.8".parse().unwrap());
        // Second line: appended by the trusted proxy itself.
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
