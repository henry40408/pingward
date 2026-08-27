//! Regression tests for the `csrf.rejected` audit event (`web::csrf_guard`, via
//! `web::log_csrf_rejection`). Two properties pulling in opposite directions.
//!
//! A rejection must leave a trace: `csrf_guard` answers with a bodyless 403 that
//! reaches no handler, so a refusal was otherwise indistinguishable from any
//! other 403 (henry40408/rdrs#477: a token drifting out of step with its session
//! took a browser down for a week).
//!
//! But the guard sits outside every handler, so it refuses before `login_submit`
//! reaches `login_limiter` — an unauthenticated bot's `POST /login` is rejected
//! unthrottled. Warning on that drowns the event in scanner traffic, so the
//! tokenless case stays quieter and the second test locks that in.

use axum_test::TestServer;
use pingward::{app, db, secret, state::AppState, store::Store};
use std::io::Write;
use std::sync::{Arc, Mutex};

mod common;

/// A `Write` sink appending into a shared buffer, as in
/// `tests/session_logging.rs` — cloning shares the same `Vec<u8>`, which
/// `tracing_subscriber::fmt`'s clone-per-event `MakeWriter` contract needs.
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
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// A server with one account, plus the buffer its logs land in.
///
/// The subscriber is JSON and admits `DEBUG`, so a test can assert on the *level*
/// an event was emitted at and match fields as `"name":"value"`.
async fn server_with_logs() -> (TestServer, SharedBuf) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let phc = pingward::auth::hash_password("correct-horse-battery-staple").unwrap();
    store
        .create_user("alice", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    let state = AppState::new(store, common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    (server, SharedBuf::default())
}

/// Installs `buf` as the default subscriber for the guard's lifetime.
/// Thread-local rather than global: `#[tokio::test]`'s current-thread runtime
/// keeps everything `TestServer` drives here, and it cannot clash with another
/// test in the binary.
fn capture(buf: &SharedBuf) -> tracing::subscriber::DefaultGuard {
    let make_writer = {
        let buf = buf.clone();
        move || buf.clone()
    };
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_writer(make_writer)
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    tracing::subscriber::set_default(subscriber)
}

/// A token that was presented and did not verify is the event worth alerting on:
/// it must warn, and must name the session by its `/account` handle rather than
/// the bearer secret behind it.
#[tokio::test]
async fn a_mismatched_token_warns_and_names_the_session_by_handle() {
    let (mut server, buf) = server_with_logs().await;
    // `common::anonymous_csrf` inlined, to keep the raw session id the negative
    // assertion needs. A second GET would not do: with `save_cookies()` the
    // client already holds the cookie, so no second `Set-Cookie` is emitted.
    server.clear_cookies();
    let res = server.get("/login").await;
    let cookie_value = res
        .cookie(pingward::auth::session_cookie_name(false))
        .value()
        .to_string();
    let raw_session_id = secret::verify_session(common::TEST_SECRET.as_bytes(), &cookie_value)
        .expect("the anonymous-session layer signs its cookie");
    let expected_handle = pingward::auth::session_log_handle(&raw_session_id);

    let guard = capture(&buf);
    let res = server
        .post("/login")
        .form(&[
            // Well-formed hex of the right shape, so this fails `verify_csrf`
            // rather than the hex decode in front of it.
            ("_csrf", &"00".repeat(32)),
            ("username", &"alice".to_string()),
            ("password", &"correct-horse-battery-staple".to_string()),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::FORBIDDEN);
    drop(guard);

    let text = buf.text();
    assert!(
        text.contains(r#""message":"csrf.rejected""#),
        "expected a csrf.rejected event, got: {text}"
    );
    assert!(
        text.contains(r#""reason":"token_mismatch""#),
        "expected reason=token_mismatch, got: {text}"
    );
    assert!(
        text.contains(r#""level":"WARN""#),
        "a presented-but-invalid token must warn, got: {text}"
    );
    assert!(
        text.contains(&format!(r#""handle":"{expected_handle}""#)),
        "expected the /account handle {expected_handle} in: {text}"
    );
    // The bearer secret must never reach a log line (as in `tests/session_logging.rs`).
    assert!(
        !text.contains(&raw_session_id),
        "the raw session id leaked into the log: {text}"
    );
}

/// The anti-spam lock: `csrf_guard` refuses ahead of `login_limiter`, so a bot's
/// `POST /login` is rejected unthrottled. Warning there would make the event
/// unreadable on any internet-facing deployment.
#[tokio::test]
async fn a_tokenless_post_is_recorded_but_not_warned() {
    let (mut server, buf) = server_with_logs().await;
    let _ = common::anonymous_csrf(&mut server).await;

    let guard = capture(&buf);
    // No `_csrf` at all — what every scanner that finds the login path sends.
    let res = server
        .post("/login")
        .form(&[
            ("username", "alice"),
            ("password", "correct-horse-battery-staple"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::FORBIDDEN);
    drop(guard);

    let text = buf.text();
    // Still recorded — the reason is diagnosable, just not at alert volume.
    assert!(
        text.contains(r#""reason":"token_missing""#),
        "expected reason=token_missing, got: {text}"
    );
    assert!(
        !text.contains(r#""level":"WARN""#),
        "a tokenless POST must not warn — it is what scanners send: {text}"
    );
}
