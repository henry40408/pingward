//! Fixed-window limiters for `POST /login`, keyed two different ways.
//!
//! [`RateLimiter`] is generic over its key because `POST /login` needs two of
//! them and they must not diverge in how a window rolls over or how the
//! tracked-key cap behaves:
//!
//! - **per client IP** (`RateLimiter<IpAddr>`, keyed by [`rate_limit_key`]) —
//!   [`MAX_ATTEMPTS`] per [`WINDOW_SECS`]. Stops one source grinding through a
//!   dictionary.
//! - **per account** (`RateLimiter<String>`, keyed by [`account_key`]) —
//!   [`ACCOUNT_MAX_ATTEMPTS`] per [`ACCOUNT_WINDOW_SECS`]. Stops a *distributed*
//!   attack, which the per-IP limiter cannot see at all: an attacker with N
//!   addresses simply gets `MAX_ATTEMPTS × N` guesses against one account.
//!   OWASP's Authentication Cheat Sheet asks for the counter to be associated
//!   with the account rather than the source address for exactly this reason.
//!   See [`ACCOUNT_MAX_ATTEMPTS`] for the denial-of-service trade-off that
//!   comes with it.
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
//!
//! A fourth defect was inherited by the first fix of (3) and is closed here:
//! at capacity, an address with no bucket of its own used to be waved through
//! unmetered, so whoever could hold [`MAX_ENTRIES`] live windows bought
//! themselves an *unlimited* guessing budget from every further address. Such
//! a caller is now charged to a single shared overflow bucket
//! ([`Buckets::overflow`]) — still fail-open, but finitely so.

use axum::http::HeaderMap;
use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Mutex;
use std::time::Instant;

/// Attempts allowed within [`WINDOW_SECS`] before `POST /login` starts
/// refusing service for a given client.
pub const MAX_ATTEMPTS: u32 = 5;
/// Width, in seconds, of the fixed window `MAX_ATTEMPTS` is counted over.
pub const WINDOW_SECS: u64 = 60;

/// Attempts allowed against a single **account** within
/// [`ACCOUNT_WINDOW_SECS`], regardless of how many addresses they arrive from.
///
/// Deliberately looser than [`MAX_ATTEMPTS`] and over a much longer window,
/// because the two limiters answer different questions and have opposite
/// failure modes. Exhausting a per-IP bucket only inconveniences the address
/// that did it. Exhausting an **account** bucket locks out the legitimate
/// owner — so an account lockout is always a denial-of-service primitive
/// handed to whoever knows a username, a trade-off the Cheat Sheet names
/// explicitly. The parameters are chosen so that:
///
/// - a *distributed* attack gets 10 guesses per 15 minutes against a given
///   account (40/hour) instead of `MAX_ATTEMPTS × addresses`, which is far too
///   few for credential stuffing to be worth running; while
/// - a legitimate owner has to fail ten times inside fifteen minutes to feel
///   it, and any single success clears the counter outright
///   ([`RateLimiter::clear`], which is why the account limiter does not use
///   [`RateLimiter::release`]).
///
/// It is a rolling **window**, not a latch: nothing has to be unlocked, and it
/// clears itself. Two further escape hatches exist for an operator being
/// deliberately locked out — the state is per-process, so a restart clears
/// every counter, and a forward-auth deployment does not use password login at
/// all.
pub const ACCOUNT_MAX_ATTEMPTS: u32 = 10;
/// Width, in seconds, of the fixed window [`ACCOUNT_MAX_ATTEMPTS`] is counted
/// over.
pub const ACCOUNT_WINDOW_SECS: u64 = 900;

/// Hard cap on tracked keys, used as the default for
/// `RateLimiter::max_entries`. When the cap is hit the map is pruned of
/// expired windows first, so a spray from many source addresses (or many
/// invented usernames) cannot grow it without bound.
const MAX_ENTRIES: usize = 10_000;

/// Longest account key retained by [`account_key`].
///
/// The username is submitted, unvalidated, on an unauthenticated form, so its
/// length is attacker-chosen — and it becomes a `HashMap` key held until its
/// window elapses. Without a bound, `MAX_ENTRIES` oversized keys would be a
/// memory-exhaustion lever rather than a rate limit.
const ACCOUNT_KEY_MAX_CHARS: usize = 64;

/// Size of the shared overflow bucket, as a multiple of `max_attempts`.
///
/// Only reachable once [`MAX_ENTRIES`] distinct addresses hold a live window
/// at the same time — a state no ordinary deployment enters, and one that
/// already means the caller commands thousands of addresses. The multiple is
/// the trade-off dial between the two failure modes at that point: charge too
/// little and a legitimate sign-in during a spray is refused; charge nothing
/// (the previous behaviour) and the spray buys unlimited guesses. Ten times
/// one address's budget keeps a handful of real users working while still
/// bounding the total.
const OVERFLOW_FACTOR: u32 = 10;

/// Everything the limiter mutates, behind one lock.
///
/// The overflow counter lives here rather than in its own `Mutex` so a request
/// that finds the map full and charges the shared bucket does both under a
/// single acquisition — splitting them would reintroduce defect 2 (see the
/// module doc) on exactly the path that is under attack.
struct Buckets<K> {
    per_key: HashMap<K, (u32, Instant)>,
    /// `(attempts, window start)` shared by every key that arrives while
    /// `per_key` is full of live windows.
    overflow: (u32, Instant),
}

/// Charge one attempt to a `(count, window start)` counter, returning whether
/// it may proceed. Shared by the per-IP buckets and the overflow bucket so the
/// two cannot drift in how a window rolls over.
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

/// Fixed-window limiter for login attempts, keyed by `K`.
///
/// Generic so the per-IP and per-account limiters are literally the same code:
/// the window roll-over, the single-lock check-and-record, the tracked-key cap
/// and the shared overflow bucket each fixed a defect (see the module doc), and
/// a second hand-written copy would be a second chance to reintroduce one.
pub struct RateLimiter<K> {
    buckets: Mutex<Buckets<K>>,
    max_attempts: u32,
    window_secs: u64,
    /// Kept as a field (rather than the [`MAX_ENTRIES`] constant) purely so
    /// tests can lower it and exercise the capacity path without tracking
    /// ten thousand keys.
    max_entries: usize,
}

impl<K: Eq + Hash> RateLimiter<K> {
    /// Create a limiter allowing `max_attempts` within `window_secs`.
    pub fn new(max_attempts: u32, window_secs: u64) -> Self {
        Self {
            buckets: Mutex::new(Buckets {
                per_key: HashMap::new(),
                // Starting at zero attempts means the first overflow caller
                // rolls the window over rather than inheriting boot time as a
                // window start.
                overflow: (0, Instant::now()),
            }),
            max_attempts,
            window_secs,
            max_entries: MAX_ENTRIES,
        }
    }

    /// Reserve an attempt for `key`, returning whether it may proceed.
    ///
    /// Checking and counting happen under a single lock, so concurrent
    /// requests cannot all observe the pre-attack count (defect 2 of the
    /// original implementation, see the module doc). The reservation is
    /// taken *before* the credential comparison — which costs an argon2
    /// verification — and is handed back by [`release`](Self::release) when
    /// the credentials turn out to be valid, so only failures ultimately
    /// consume the window.
    pub fn try_acquire(&self, key: K) -> bool {
        // (a) `std::sync::Mutex`, not `parking_lot::Mutex` — pingward has no
        // dependency on `parking_lot` and this task must not add one.
        // Recovering from poisoning (rather than `unwrap()`) means one
        // panicking request under this lock can't turn `POST /login` into a
        // permanently broken endpoint for everyone after it; clippy pedantic
        // also flags a bare `unwrap()` here.
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
                // Every entry is still live: a spray from more distinct
                // sources than the cap. Existing counters are left alone —
                // clearing the map would hand anyone already throttled a way
                // to reset their own budget (defect 3, see the module doc) —
                // and this untracked source is charged to the shared overflow
                // bucket instead of being waved through. Admitting it
                // unmetered (the previous behaviour) made the cap itself the
                // bypass: hold `max_entries` live windows and every further
                // address guesses without limit. Refusing outright would
                // instead turn the same spray into a global login lockout, so
                // the bucket is deliberately generous — see
                // [`OVERFLOW_FACTOR`].
                let max = self.max_attempts.saturating_mul(OVERFLOW_FACTOR);
                return charge(&mut buckets.overflow, max, window_secs);
            }
        }
        let entry = buckets.per_key.entry(key).or_insert((0, Instant::now()));
        charge(entry, self.max_attempts, self.window_secs)
    }

    /// Hand back the attempt reserved by [`try_acquire`](Self::try_acquire).
    ///
    /// Called after a *successful* login only. The control exists to stop
    /// password guessing, and a legitimate user who signs in repeatedly (new
    /// device, cleared cookies, a test suite) should not be locked out by it;
    /// an attacker's every attempt is a failure, so their budget is
    /// unchanged.
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
        // No bucket of its own: the attempt was charged to the shared overflow
        // bucket (the map was full), so that is what gets the refund —
        // otherwise a successful sign-in during a spray would still burn the
        // shared budget. A window that rolled over between the two calls can
        // over-refund by one attempt, which costs nothing: the counter is
        // already saturating at zero.
        buckets.overflow.0 = buckets.overflow.0.saturating_sub(1);
    }

    /// Drop `key`'s bucket entirely, rather than refunding one attempt.
    ///
    /// This is the **account** limiter's success path, and the difference from
    /// [`release`](Self::release) is deliberate. Refunding a single attempt
    /// would leave an owner who mistyped nine times and then signed in
    /// correctly sitting one failure away from a fifteen-minute lockout, with
    /// the credential already proven — which is the availability half of the
    /// trade-off this limiter exists inside of (see [`ACCOUNT_MAX_ATTEMPTS`]),
    /// falling on precisely the wrong person.
    ///
    /// The per-IP limiter must *not* do this: a success there says nothing
    /// about the other attempts from that address, which may be a shared NAT
    /// or a proxy carrying an attacker as well. A success *here* is proof of
    /// the credential the preceding failures were guessing at.
    ///
    /// It does hand a small amount back to an attacker — someone spraying an
    /// account has their budget reset whenever its owner signs in. That is
    /// bounded by how often a person actually logs in (sessions here idle out
    /// after 72 hours), so it is a few extra guesses a day against a budget of
    /// 10 per 15 minutes, and it is worth the owner not being locked out of
    /// their own monitoring during an attack.
    pub fn clear(&self, key: &K) {
        self.buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .per_key
            .remove(key);
    }
}

/// The account limiter's key: the **submitted** username, bounded in length.
///
/// Keyed on what the form carried, *not* on a resolved `users.id`, and looked
/// up before the account is. An unknown username has to consume and exhaust a
/// budget exactly as a real one does — otherwise "this request was throttled"
/// becomes a username oracle, and the whole point of
/// `auth::verify_password_or_dummy` (which equalises the response *time* for
/// the same reason) is given back at a different layer.
///
/// Only the length is normalised, never the case: `find_user_by_username`
/// compares exactly on both backends, so `Alice` and `alice` are different
/// accounts and must be different buckets. Truncation can therefore make two
/// very long usernames share a bucket; that direction is safe (they throttle
/// each other sooner) and needs a 64-character common prefix to happen at all.
pub fn account_key(username: &str) -> String {
    username.chars().take(ACCOUNT_KEY_MAX_CHARS).collect()
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

    /// The generic parameter is not decoration: the account limiter is a
    /// `RateLimiter<String>` and must behave identically to the IP one.
    #[test]
    fn a_string_keyed_limiter_counts_per_key() {
        let limiter: RateLimiter<String> = RateLimiter::new(2, 60);
        assert!(limiter.try_acquire("alice".into()));
        assert!(limiter.try_acquire("alice".into()));
        assert!(!limiter.try_acquire("alice".into()));
        // A different account is untouched — a lockout is never global.
        assert!(limiter.try_acquire("bob".into()));
    }

    /// `clear` is the account limiter's success path, and differs from
    /// `release` on purpose: proving the credential empties the bucket rather
    /// than refunding the single attempt it just cost. Nine failures followed
    /// by a success must leave the owner with a *full* budget, not one attempt.
    #[test]
    fn clear_empties_the_bucket_where_release_refunds_one() {
        let refunded: RateLimiter<String> = RateLimiter::new(10, 60);
        let cleared: RateLimiter<String> = RateLimiter::new(10, 60);
        for _ in 0..9 {
            assert!(refunded.try_acquire("alice".into()));
            assert!(cleared.try_acquire("alice".into()));
        }
        // The tenth attempt is the successful sign-in, in both cases.
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
        // Case is load-bearing: `find_user_by_username` compares exactly on
        // both backends, so these are two different accounts and must not
        // share a bucket.
        assert_eq!(account_key("Alice"), "Alice");
        assert_ne!(account_key("Alice"), account_key("alice"));
        // Nor is anything else normalised away.
        assert_eq!(account_key("  bob  "), "  bob  ");

        // An attacker-chosen username cannot become an attacker-chosen
        // allocation: the key is bounded however long the field was.
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
                .buckets
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .per_key
                .len()
        );

        // The map is now at capacity and every entry's window has already
        // elapsed. A fresh key must trigger a real prune.
        assert!(limiter.try_acquire(ip(99)));
        let len = limiter
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .per_key
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

    /// The capacity path used to admit an untracked address unmetered, so
    /// holding `max_entries` live windows bought unlimited guesses from every
    /// further address. It is now charged to a shared bucket of
    /// `max_attempts * OVERFLOW_FACTOR`, which must eventually refuse.
    #[test]
    fn the_overflow_bucket_is_finite() {
        const MAX: u32 = 2;
        let mut limiter = RateLimiter::new(MAX, 60);
        limiter.max_entries = 4;
        // Fill the map with live windows so every further address overflows.
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
        // Still not a global lockout: an address that owns a bucket with room
        // left is unaffected by the exhausted shared one.
        limiter.release(&ip(0));
        assert!(limiter.try_acquire(ip(0)));
    }

    /// A window rollover refills the shared bucket, exactly as it refills a
    /// per-IP one — otherwise a single spray would refuse every untracked
    /// address forever.
    #[test]
    fn the_overflow_bucket_refills_with_its_window() {
        let mut limiter = RateLimiter::new(1, 0); // zero-second window
        limiter.max_entries = 0; // every address overflows
        for _ in 0..(OVERFLOW_FACTOR * 3) {
            assert!(limiter.try_acquire(ip(1)));
        }
    }

    /// A successful sign-in charged to the shared bucket hands its attempt
    /// back, so a legitimate user does not permanently consume the budget the
    /// spray is competing for.
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
