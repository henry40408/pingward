//! The `csrf.rejected` log event. A rejection is a bodyless 403 that reaches no
//! handler, so this event is its only trace — yet the guard refuses before
//! `login_limiter`, so tokenless bot traffic must stay at `debug`, not `warn`.

use axum_test::TestServer;
use pingward::{app, db, secret, state::AppState, store::Store};
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
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// A server with one account, plus the buffer its logs land in.
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

/// Installs a JSON, `DEBUG`-level subscriber writing to `buf` for the guard's
/// lifetime. Thread-local suffices on `#[tokio::test]`'s current-thread runtime.
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

/// A presented-but-invalid token must warn, naming the session by its log
/// handle, never the raw session id.
#[tokio::test]
async fn a_mismatched_token_warns_and_names_the_session_by_handle() {
    let (mut server, buf) = server_with_logs().await;
    // `common::anonymous_csrf` inlined to keep the raw session id.
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
            // Well-formed hex, so this fails `verify_csrf`, not the hex decode.
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
    assert!(
        !text.contains(&raw_session_id),
        "the raw session id leaked into the log: {text}"
    );
}

/// Tokenless posts are what scanners send, unthrottled; warning on them would
/// drown the event.
#[tokio::test]
async fn a_tokenless_post_is_recorded_but_not_warned() {
    let (mut server, buf) = server_with_logs().await;
    let _ = common::anonymous_csrf(&mut server).await;

    let guard = capture(&buf);
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
    assert!(
        text.contains(r#""reason":"token_missing""#),
        "expected reason=token_missing, got: {text}"
    );
    assert!(
        !text.contains(r#""level":"WARN""#),
        "a tokenless POST must not warn — it is what scanners send: {text}"
    );
}
