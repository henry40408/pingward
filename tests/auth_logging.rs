//! The `pingward::auth` failure log (`web::log_login_failure`,
//! `web::log_reauth_failure`): the only spray signal, and nothing else in the
//! suite notices if it goes missing. Events carry their `reason`/`surface`, and
//! the submitted password never appears.

use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};
use std::io::Write;
use std::sync::{Arc, Mutex};

mod common;

/// A `Write` sink whose clones share one buffer, as `MakeWriter` requires.
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

/// Short on purpose: `/login` never applies the length policy.
const FIXTURE_PW: &str = "pw";

/// Distinctive, so a log match is never coincidental.
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

/// Captures log output for the duration of `f`. Thread-local suffices on
/// `#[tokio::test]`'s current-thread runtime.
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
    assert!(
        !log.contains(WRONG_PW),
        "the submitted password must never be logged: {log}"
    );
}

/// An unknown username logs the same `login.failed` event as a wrong password.
#[tokio::test]
async fn an_unknown_username_is_logged_like_a_wrong_password() {
    let (mut server, _store) = server_with_user("alice", false).await;
    let log = captured(|| async {
        attempt_login(&mut server, "nobody", WRONG_PW).await;
    })
    .await;

    assert!(log.contains("login.failed"), "{log}");
    assert!(log.contains("nobody"), "{log}");
    assert!(log.contains("reason=\"bad_credentials\""), "{log}");
    assert!(!log.contains(WRONG_PW), "{log}");
}

#[tokio::test]
async fn a_disabled_account_logs_its_own_reason() {
    let (mut server, _store) = server_with_user("banned", true).await;
    // Correct credentials: the rejection is the account state.
    let log = captured(|| async {
        attempt_login(&mut server, "banned", FIXTURE_PW).await;
    })
    .await;

    assert!(log.contains("login.failed"), "{log}");
    assert!(log.contains("reason=\"account_disabled\""), "{log}");
}

/// The throttled attempt itself is logged, not just those leading to it.
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
    // Named because behind a proxy it can differ from the attributed `ip`.
    assert!(log.contains("bucket=127.0.0.1"), "{log}");
}

/// The username is rendered with `Debug`, so an embedded newline is escaped
/// instead of forging a second log line.
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
    // The text is still present, but inside one quoted field.
    assert!(log.contains("\\n"), "the newline must be escaped: {log}");
    assert_eq!(
        log.trim_end().lines().count(),
        1,
        "the forged newline opened a second log line: {log}"
    );
}

/// `auth::log_username` truncates a giant username.
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

/// A wrong current password on `/account` signals a hijacked session.
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

    // One event for every re-auth gate, discriminated by `surface`.
    assert!(log.contains("reauth.failed"), "{log}");
    assert!(log.contains("surface=\"password_change\""), "{log}");
    assert!(log.contains("reason=\"bad_current_password\""), "{log}");
    assert!(!log.contains(WRONG_PW), "{log}");
}

/// An account lockout (one account, possibly many addresses) gets its own
/// `reason`, distinct from the per-address `rate_limited`.
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
    // Otherwise the per-address budget (5) runs out before the per-account (10).
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

/// The API-key gate logs the same `reauth.failed` event, with its own `surface`.
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
