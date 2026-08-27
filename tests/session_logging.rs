//! Regression test for `pingward::session` audit logging
//! (`auth::session_log_handle`, used at every `tracing::info!(target:
//! "pingward::session", ...)` site in `src/web.rs`).
//!
//! Nothing else in the suite captures a `tracing` subscriber, so those call
//! sites could all be deleted with CI green. Only the helper is unit-tested,
//! never a call site: a refactor writing `handle = %id` would ship raw session
//! ids — the cookie's bearer secret — into the logs unnoticed. This drives
//! login -> request -> logout against a capturing subscriber and asserts both
//! halves: the events are emitted, and the raw id appears nowhere.

use axum_test::TestServer;
use pingward::{app, db, secret, state::AppState, store::Store};
use std::io::Write;
use std::sync::{Arc, Mutex};

mod common;

/// A `Write` sink appending into a shared buffer. Cloning shares the same
/// `Vec<u8>`, so `MakeWriter`'s clone-per-event closure still collects into one.
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
    // Scoped thread-local guard: it cannot clash with a subscriber another test
    // in this binary installs, and `#[tokio::test]`'s current-thread runtime keeps
    // every task this test drives on the thread the guard applies to.
    let guard = tracing::subscriber::set_default(subscriber);

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

    // Recover the raw session id the way production does, so the negative
    // assertion below checks the actual bearer secret rather than a stand-in.
    let cookie_name = pingward::auth::session_cookie_name(false);
    let cookie_value = res.cookie(cookie_name).value().to_string();
    let raw_session_id = secret::verify_session(common::TEST_SECRET.as_bytes(), &cookie_value)
        .expect("login sets a signed session cookie");
    let expected_handle = pingward::auth::session_log_handle(&raw_session_id);

    let csrf = common::newest_session_csrf(&store.pool).await;
    server.add_header("x-csrf-token", csrf.as_str());
    server.get("/").await.assert_status_ok();

    server.post("/logout").await;

    drop(guard);
    let output = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();

    // (a) `session.created` carries no `reason` field — only `session.destroyed`
    // does — and a password login is distinguished from forward-auth only by
    // `sso=false`.
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

    // (b) The half that fails if a call site logs `%id` instead of
    // `%session_log_handle(&id)`.
    assert!(
        !output.contains(&raw_session_id),
        "the raw session id must never appear in logged output — found it in:\n{output}"
    );
}
