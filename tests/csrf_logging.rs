//! Regression tests for the `csrf.rejected` audit event (`web::csrf_guard`,
//! via `web::log_csrf_rejection`).
//!
//! Two properties, and they pull in opposite directions.
//!
//! A rejection must leave a trace at all. `csrf_guard` answers with a bodyless
//! 403 that reaches no handler, so before this event a refusal was
//! indistinguishable from any other 403 the app can return — which is exactly
//! the position rdrs was in when a CSRF token drifting out of step with its
//! session took a browser down for a week (henry40408/rdrs#477).
//!
//! But the guard is layered *outside* every handler, so it also refuses before
//! `login_submit` ever reaches `login_limiter`. An unauthenticated bot sending
//! `POST /login` is rejected here with nothing throttling it. If that warns,
//! the event drowns in scanner traffic and stops being read at all — so the
//! tokenless case is deliberately quieter, and the test below locks that in.
//! Without it, "promote everything to warn!" is a one-word change that looks
//! like a tidy-up and silently costs the signal.

use axum_test::TestServer;
use pingward::{app, db, secret, state::AppState, store::Store};
use std::io::Write;
use std::sync::{Arc, Mutex};

mod common;

/// A `Write` sink that appends into a shared buffer, as in
/// `tests/session_logging.rs` — cloning shares the same `Vec<u8>` through the
/// `Arc`, which is what `tracing_subscriber::fmt`'s clone-per-event
/// `MakeWriter` contract needs.
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
/// The subscriber is JSON and admits `DEBUG`, so a test can assert on the
/// *level* an event was emitted at — the whole point of the second test — and
/// match a field as `"name":"value"` rather than by groping through prose.
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

/// Install `buf` as the default subscriber for as long as the guard lives.
/// Thread-local rather than global: `#[tokio::test]` defaults to a
/// current-thread runtime, so everything `TestServer` drives stays on the one
/// thread this applies to, and it cannot clash with another test in the binary.
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

/// A token that was presented and did not verify is the event worth alerting
/// on: it is what a token drifting out of step with its session looks like
/// from the server side. It must warn, and it must name the session by its
/// `/account` handle rather than by the bearer secret behind it.
#[tokio::test]
async fn a_mismatched_token_warns_and_names_the_session_by_handle() {
    let (mut server, buf) = server_with_logs().await;
    // What `common::anonymous_csrf` does, inlined to keep the raw session id
    // the negative assertion below needs. It cannot simply GET twice: with
    // `save_cookies()` the client already holds the cookie by then, so
    // `anonymous_session` finds it valid and emits no second `Set-Cookie`.
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
    // The cookie's bearer secret must never reach a log line — the same
    // invariant `tests/session_logging.rs` locks for the session events.
    assert!(
        !text.contains(&raw_session_id),
        "the raw session id leaked into the log: {text}"
    );
}

/// The anti-spam lock. `csrf_guard` refuses ahead of `login_limiter`, so a bot
/// sending `POST /login` is rejected here with nothing throttling it. That
/// path must not warn, or the event is unreadable on any internet-facing
/// deployment and the mismatch above is lost inside it.
#[tokio::test]
async fn a_tokenless_post_is_recorded_but_not_warned() {
    let (mut server, buf) = server_with_logs().await;
    let _ = common::anonymous_csrf(&mut server).await;

    let guard = capture(&buf);
    // No `_csrf` field at all — what a client that never rendered the form
    // sends, which is every scanner that finds the login path.
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
