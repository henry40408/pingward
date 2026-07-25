//! Regression test for the `pingward::session` audit-logging control
//! (`auth::session_log_handle`, wired up at every `tracing::info!(target:
//! "pingward::session", ...)` call site in `src/web.rs`).
//!
//! Nothing else in the suite captures a `tracing` subscriber, so every one of
//! those call sites could be deleted and the suite would stay green. Worse,
//! the doc comment on `auth::session_log_handle` says **"Never log the
//! session id itself"** — it is the cookie's bearer secret — but only the
//! helper itself is unit-tested (`auth::tests::session_log_handle_is_never_the_raw_id`),
//! never a real call site. A refactor that wrote `handle = %id` instead of
//! `handle = %session_log_handle(&id)` would ship raw session ids into
//! whatever `PINGWARD_LOG_FORMAT=json` feeds a log aggregator, with CI green.
//!
//! This test installs a real capturing subscriber, drives a realistic
//! session lifecycle (login -> one authenticated request -> logout), and
//! asserts both halves of the invariant: the documented events are actually
//! emitted, and the raw session id string appears nowhere in the captured
//! output.

use axum_test::TestServer;
use pingward::{app, db, secret, state::AppState, store::Store};
use std::io::Write;
use std::sync::{Arc, Mutex};

mod common;

/// A `Write` sink that appends into a shared buffer. Cloning shares the same
/// underlying `Vec<u8>` via the `Arc`, which is what lets a clone-per-event
/// closure (`tracing_subscriber::fmt`'s `MakeWriter` contract) still collect
/// everything into one place the test can inspect afterwards.
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

#[tokio::test]
async fn session_lifecycle_is_logged_without_leaking_the_raw_id() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let phc = pingward::auth::hash_password("pw12345").unwrap();
    store
        .create_user("alice", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    let state = AppState::new(store.clone(), common::test_config());
    // axum-test 21's `TestServer::new` returns `Self` directly, not a
    // `Result` (see `tests/auth_web.rs`'s note).
    let mut server = TestServer::new(app(state));
    server.save_cookies();

    let buf = SharedBuf::default();
    let make_writer = {
        let buf = buf.clone();
        move || buf.clone()
    };
    let subscriber = tracing_subscriber::fmt()
        .with_writer(make_writer)
        .with_ansi(false)
        .finish();
    // A scoped, thread-local guard rather than `set_global_default`. Nextest
    // runs each test in its own process, so a global subscriber would also be
    // safe here — this form is used because it cannot clash with a
    // subscriber another test in this binary installs, and `#[tokio::test]`
    // defaults to a current-thread runtime, so every task this test drives
    // (including whatever `TestServer` spawns) stays on the one thread this
    // guard applies to.
    let guard = tracing::subscriber::set_default(subscriber);

    // Login.
    let csrf = common::anonymous_csrf(&mut server).await;
    let res = server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "alice"),
            ("password", "pw12345"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);

    // Recover the raw session id the same way production code does
    // (`secret::session_id_from_jar`, here applied directly to the Set-Cookie
    // value) so the negative assertion below checks the actual bearer secret,
    // not a stand-in for it.
    let cookie_name = pingward::auth::session_cookie_name(false);
    let cookie_value = res.cookie(cookie_name).value().to_string();
    let raw_session_id = secret::verify_session(common::TEST_SECRET.as_bytes(), &cookie_value)
        .expect("login sets a signed session cookie");
    let expected_handle = pingward::auth::session_log_handle(&raw_session_id);

    // One authenticated request.
    let csrf = common::newest_session_csrf(&store.pool).await;
    server.add_header("x-csrf-token", csrf.as_str());
    server.get("/").await.assert_status_ok();

    // Logout.
    server.post("/logout").await;

    drop(guard);
    let output = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();

    // (a) The documented events are actually emitted.
    //
    // `session.created` (`web::open_session`) carries no `reason` field —
    // only `session.destroyed` events do (see `ARCHITECTURE.md`'s "Session
    // and CSRF secret" event-field list); a password login is distinguished
    // from forward-auth only by `sso=false`. So this checks for the event
    // name plus `sso=false`, and for `session.destroyed` with the one reason
    // this lifecycle actually produces: `logout`.
    assert!(
        output.contains("session.created"),
        "expected a session.created event in:\n{output}"
    );
    assert!(
        output.contains("sso=false"),
        "expected the password-login session.created event to carry sso=false in:\n{output}"
    );
    assert!(
        output.contains("session.destroyed"),
        "expected a session.destroyed event in:\n{output}"
    );
    assert!(
        output.contains(r#"reason="logout""#),
        "expected the logout teardown to be tagged reason=\"logout\" in:\n{output}"
    );
    assert!(
        output.contains(&expected_handle),
        "expected the session_log_handle to appear in:\n{output}"
    );

    // (b) The raw session id — the cookie's bearer secret — appears nowhere
    // in the captured output. This is the half that would genuinely fail if
    // a call site logged `%id` instead of `%session_log_handle(&id)`.
    assert!(
        !output.contains(&raw_session_id),
        "the raw session id must never appear in logged output — found it in:\n{output}"
    );
}
