//! Regression test for the `pingward::auth` failure-logging control
//! (`web::log_login_failure` and the `password_change.failed` call site in
//! `web::account_password`).
//!
//! OWASP's Authentication Cheat Sheet asks for every authentication failure and
//! lockout to be logged and reviewed. For a self-hosted pingward this log is
//! the *only* signal that the login page is being sprayed: the audit table
//! records what succeeded, the rate limiter keeps its counters in memory and
//! tells nobody, and nothing else observes a rejected attempt. Delete the
//! `tracing::warn!` calls and every other test in the suite still passes — the
//! same blind spot `tests/session_logging.rs` was written to close for the
//! session events, so this borrows that file's capturing-subscriber harness.
//!
//! Two halves are asserted, as there: the events are actually emitted with the
//! `reason` that discriminates them, and the submitted **password never
//! appears** in the captured output.

use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};
use std::io::Write;
use std::sync::{Arc, Mutex};

mod common;

/// See `tests/session_logging.rs` — a `Write` sink whose clones all append into
/// one shared buffer, which is what `tracing_subscriber::fmt`'s `MakeWriter`
/// contract (a clone per event) requires.
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

/// The password every fixture account is created with. Short on purpose: the
/// length policy governs the surfaces that *set* a password, never `/login`,
/// so an account whose hash predates the policy must still be able to sign in.
const FIXTURE_PW: &str = "pw";

/// A wrong password, distinctive enough that finding it in the log output is
/// unambiguous rather than a coincidental substring match.
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

/// Install a capturing subscriber for the duration of `f`. Scoped rather than
/// global for the reason `tests/session_logging.rs` gives: it cannot clash with
/// a subscriber another test in this binary installs, and `#[tokio::test]`'s
/// current-thread runtime keeps every task this drives on the guarded thread.
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
    // The whole point of the control is that it is safe to keep: a log that
    // carried the attempted password would be a credential store of its own,
    // and a near-miss guess in it is worth as much as the real thing.
    assert!(
        !log.contains(WRONG_PW),
        "the submitted password must never be logged: {log}"
    );
}

/// An unknown username logs the same event as a wrong password. `user_exists`
/// is what separates a typo from a spray, and it is safe to record because the
/// log is not a response — the *reply* to both stays identical, which is what
/// `auth::verify_password_or_dummy` exists to keep true of the response time
/// as well.
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
    // Correct credentials — the rejection is the account state, and that is a
    // materially different event for whoever reads the log.
    let log = captured(|| async {
        attempt_login(&mut server, "banned", FIXTURE_PW).await;
    })
    .await;

    assert!(log.contains("login.failed"), "{log}");
    assert!(log.contains("reason=\"account_disabled\""), "{log}");
}

/// The lockout itself is logged, not just the attempts leading to it —
/// otherwise the one event an operator most needs to see is the one the log
/// stops at.
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
    // The bucket the limiter counted against is named, since it can legitimately
    // differ from the attribution address behind a proxy.
    assert!(log.contains("bucket=127.0.0.1"), "{log}");
}

/// A username is attacker-chosen input. `auth::log_username` truncates it and
/// the call site renders it with `Debug`, so an embedded newline is escaped
/// rather than closing the line and opening a forged one — which in `text`
/// format would otherwise let an attacker write arbitrary entries.
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
    // The forged text is still *present* — it is the username that was tried,
    // and suppressing it would defeat the point of logging the attempt. What
    // matters is that it stays inside one quoted field: a single entry, so a
    // reader (or a log parser) can never mistake it for a second record.
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

    assert!(log.contains("password_change.failed"), "{log}");
    assert!(log.contains("reason=\"bad_current_password\""), "{log}");
    assert!(!log.contains(WRONG_PW), "{log}");
}

/// An account lockout is a materially different event from an address
/// throttle: it means somebody is working on one *specific* account, quite
/// possibly from many addresses. It gets its own `reason` so an operator can
/// alert on it separately, rather than being folded into `rate_limited`.
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
