//! Regression test for the `pingward::auth` failure-logging control
//! (`web::log_login_failure` for the login form, `web::log_reauth_failure` for
//! the re-authentication gates).
//!
//! This log is the only signal that the login page is being sprayed: the audit
//! table records what succeeded, the rate limiter keeps its counters in memory,
//! and nothing else observes a rejected attempt — delete the `tracing::warn!`
//! calls and the rest of the suite still passes. Borrows the
//! capturing-subscriber harness from `tests/session_logging.rs` and asserts both
//! halves: the events are emitted with the discriminating `reason`, and the
//! submitted password never appears.

use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};
use std::io::Write;
use std::sync::{Arc, Mutex};

mod common;

/// See `tests/session_logging.rs` — a `Write` sink whose clones all append into
/// one shared buffer, as `tracing_subscriber::fmt`'s `MakeWriter` requires.
#[derive(Clone, Default)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl SharedBuf {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

/// Short on purpose: the length policy governs the surfaces that *set* a
/// password, never `/login`, so a hash predating the policy must still sign in.
const FIXTURE_PW: &str = "pw";

/// Distinctive, so finding it in the log is never a coincidental substring match.
const WRONG_PW: &str = "zzz-never-log-this-zzz";

async fn server_with_user(username: &str, disabled: bool) -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let phc = pingward::auth::hash_password(FIXTURE_PW).unwrap();
    let uid = store
        .create_user(username, Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    if disabled {
        store.set_user_disabled(uid, true).await.unwrap();
    }
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    (server, store)
}

/// Installs a capturing subscriber for the duration of `f`. Scoped rather than
/// global: it cannot clash with another test in this binary, and
/// `#[tokio::test]`'s current-thread runtime keeps every task on this thread.
async fn captured<F, Fut>(f: F) -> String
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    let buf = SharedBuf::default();
    let make_writer = {
        let buf = buf.clone();
        move || buf.clone()
    };
    let subscriber = tracing_subscriber::fmt()
        .with_writer(make_writer)
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    f().await;
    buf.text()
}

async fn attempt_login(server: &mut TestServer, username: &str, password: &str) {
    // `csrf_guard` has no path exemptions, so each attempt needs its own token
    // from a fresh anonymous session.
    let csrf = common::anonymous_csrf(server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", username),
            ("password", password),
        ])
        .await;
}

#[tokio::test]
async fn a_wrong_password_is_logged_without_the_password() {
    let (mut server, _store) = server_with_user("alice", false).await;
    let log = captured(|| async {
        attempt_login(&mut server, "alice", WRONG_PW).await;
    })
    .await;

    assert!(log.contains("login.failed"), "{log}");
    assert!(log.contains("reason=\"bad_credentials\""), "{log}");
    assert!(log.contains("alice"), "{log}");
    // A log carrying the attempted password would be a credential store of its
    // own; a near-miss guess is worth as much as the real thing.
    assert!(
        !log.contains(WRONG_PW),
        "the submitted password must never be logged: {log}"
    );
}

/// An unknown username logs the same event as a wrong password. `user_exists`
/// separates a typo from a spray and is safe to record because the log is not a
/// response — the *reply* stays identical, as does the response time
/// (`auth::verify_password_or_dummy`).
#[tokio::test]
async fn an_unknown_username_is_logged_and_marked_as_such() {
    let (mut server, _store) = server_with_user("alice", false).await;
    let log = captured(|| async {
        attempt_login(&mut server, "nobody", WRONG_PW).await;
    })
    .await;

    assert!(log.contains("login.failed"), "{log}");
    assert!(log.contains("nobody"), "{log}");
    assert!(!log.contains(WRONG_PW), "{log}");
}

#[tokio::test]
async fn a_disabled_account_logs_its_own_reason() {
    let (mut server, _store) = server_with_user("banned", true).await;
    // Correct credentials: the rejection is the account state, a different event.
    let log = captured(|| async {
        attempt_login(&mut server, "banned", FIXTURE_PW).await;
    })
    .await;

    assert!(log.contains("login.failed"), "{log}");
    assert!(log.contains("reason=\"account_disabled\""), "{log}");
}

/// The lockout itself is logged, not just the attempts leading to it — otherwise
/// the one event an operator most needs is the one the log stops at.
#[tokio::test]
async fn hitting_the_rate_limit_is_logged() {
    let (mut server, _store) = server_with_user("alice", false).await;
    for _ in 0..pingward::ratelimit::MAX_ATTEMPTS {
        attempt_login(&mut server, "alice", WRONG_PW).await;
    }
    let log = captured(|| async {
        attempt_login(&mut server, "alice", WRONG_PW).await;
    })
    .await;

    assert!(log.contains("reason=\"rate_limited\""), "{log}");
    // The bucket is named: behind a proxy it can differ from the attribution address.
    assert!(log.contains("bucket=127.0.0.1"), "{log}");
}

/// A username is attacker-chosen input. `auth::log_username` truncates it and the
/// call site renders it with `Debug`, so an embedded newline is escaped rather
/// than closing the line and opening a forged one.
#[tokio::test]
async fn a_forged_newline_in_the_username_cannot_open_a_second_log_line() {
    let (mut server, _store) = server_with_user("alice", false).await;
    let forged = "eve\nERROR pingward::auth: login.succeeded user_id=1";
    let log = captured(|| async {
        attempt_login(&mut server, forged, WRONG_PW).await;
    })
    .await;

    assert!(log.contains("login.failed"), "{log}");
    assert!(log.contains("eve"), "{log}");
    // The forged text is still *present* — it is the username that was tried.
    // What matters is that it stays inside one quoted field: a single entry.
    assert!(log.contains("\\n"), "the newline must be escaped: {log}");
    assert_eq!(
        log.trim_end().lines().count(),
        1,
        "the forged newline opened a second log line: {log}"
    );
}

/// A giant username cannot be turned into a giant log line.
#[tokio::test]
async fn an_oversized_username_is_truncated() {
    let (mut server, _store) = server_with_user("alice", false).await;
    let huge = "x".repeat(10_000);
    let log = captured(|| async {
        attempt_login(&mut server, &huge, WRONG_PW).await;
    })
    .await;

    assert!(log.contains("login.failed"), "{log}");
    assert!(
        log.len() < 1_000,
        "a 10k username produced a {}-byte log: {log}",
        log.len()
    );
    assert!(log.contains('…'), "truncation marker missing: {log}");
}

/// Guessing at the current password from an already-authenticated session is a
/// session takeover in progress, and `/account` is the only place it shows.
#[tokio::test]
async fn a_wrong_current_password_on_account_is_logged() {
    let (mut server, store) = server_with_user("alice", false).await;
    attempt_login(&mut server, "alice", FIXTURE_PW).await;
    let tok = common::newest_session_csrf(&store.pool).await;
    server.add_header("x-csrf-token", tok.as_str());

    let log = captured(|| async {
        server
            .post("/account/password")
            .form(&[
                ("current_password", WRONG_PW),
                ("new_password", "a brand new passphrase"),
                ("confirm_password", "a brand new passphrase"),
            ])
            .await;
    })
    .await;

    // One event for every re-auth gate, discriminated by `surface`, so one alert
    // rule need not know which forms exist.
    assert!(log.contains("reauth.failed"), "{log}");
    assert!(log.contains("surface=\"password_change\""), "{log}");
    assert!(log.contains("reason=\"bad_current_password\""), "{log}");
    assert!(!log.contains(WRONG_PW), "{log}");
}

/// An account lockout differs from an address throttle: somebody is working on
/// one *specific* account, possibly from many addresses. Its own `reason` lets an
/// operator alert on it separately.
#[tokio::test]
async fn locking_an_account_logs_its_own_reason() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let phc = pingward::auth::hash_password(FIXTURE_PW).unwrap();
    store
        .create_user("alice", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    let mut state = AppState::new(store, common::test_config());
    // Without this the per-address budget (5) runs out before the per-account
    // one (10) and the log would say `rate_limited` instead.
    state.login_limiter = std::sync::Arc::new(pingward::ratelimit::RateLimiter::new(u32::MAX, 60));
    let mut server = TestServer::new(app(state));
    server.save_cookies();

    for _ in 0..pingward::ratelimit::ACCOUNT_MAX_ATTEMPTS {
        attempt_login(&mut server, "alice", WRONG_PW).await;
    }
    let log = captured(|| async {
        attempt_login(&mut server, "alice", WRONG_PW).await;
    })
    .await;

    assert!(log.contains("reason=\"account_locked\""), "{log}");
    assert!(log.contains("alice"), "{log}");
    assert!(!log.contains(WRONG_PW), "{log}");
}

/// The API-key gate logs the same event as the password-change one,
/// distinguished by `surface` — two names would mean two alert rules for one thing.
#[tokio::test]
async fn a_refused_api_key_re_authentication_is_logged() {
    let (mut server, store) = server_with_user("alice", false).await;
    attempt_login(&mut server, "alice", FIXTURE_PW).await;
    let tok = common::newest_session_csrf(&store.pool).await;
    server.add_header("x-csrf-token", tok.as_str());

    let log = captured(|| async {
        server
            .post("/account/api-keys")
            .form(&[
                ("name", "ci"),
                ("expires_in", ""),
                ("current_password", WRONG_PW),
            ])
            .await;
    })
    .await;

    assert!(log.contains("reauth.failed"), "{log}");
    assert!(log.contains("surface=\"api_key_create\""), "{log}");
    assert!(log.contains("reason=\"bad_current_password\""), "{log}");
    assert!(!log.contains(WRONG_PW), "{log}");
}
