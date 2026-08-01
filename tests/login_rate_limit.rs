//! `POST /login` per-client-IP rate limiting (`crate::ratelimit`).
//!
//! `axum-test` drives the router without `ConnectInfo` (see
//! `src/main.rs`'s `into_make_service_with_connect_info`, only wired up for
//! the real listener), so every request here has no socket peer and lands in
//! `ratelimit::rate_limit_key`'s loopback fallback bucket — that fallback
//! path is exactly what these tests exercise.

use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};

mod common;

async fn server() -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    (server, store)
}

async fn create_user(store: &Store, username: &str, password: &str) -> i64 {
    let phc = pingward::auth::hash_password(password).unwrap();
    store
        .create_user(username, Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap()
}

/// A wrong-password `POST /login`, with a fresh CSRF token — `csrf_guard` has
/// no path exemptions, so every attempt needs its own token from a fresh
/// anonymous session (`common::anonymous_csrf` clears cookies to guarantee a
/// mint).
async fn failed_login(server: &mut TestServer, username: &str) -> axum_test::TestResponse {
    let csrf = common::anonymous_csrf(server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", username),
            ("password", "wrong-password"),
        ])
        .await
}

#[tokio::test]
async fn sixth_failed_login_is_rate_limited() {
    let (mut server, store) = server().await;
    create_user(&store, "bob", "correct-password").await;

    for _ in 0..pingward::ratelimit::MAX_ATTEMPTS {
        let res = failed_login(&mut server, "bob").await;
        res.assert_status_ok(); // re-rendered login form with an error
    }

    let res = failed_login(&mut server, "bob").await;
    res.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        res.header("retry-after"),
        pingward::ratelimit::WINDOW_SECS.to_string()
    );
}

#[tokio::test]
async fn successful_logins_do_not_consume_the_budget() {
    let (mut server, store) = server().await;
    create_user(&store, "bob", "correct-password").await;

    // More than MAX_ATTEMPTS successful logins in a row: each one releases
    // its own reservation, so none of them should ever be throttled.
    for _ in 0..=pingward::ratelimit::MAX_ATTEMPTS {
        let csrf = common::anonymous_csrf(&mut server).await;
        let res = server
            .post("/login")
            .form(&[
                ("_csrf", csrf.as_str()),
                ("username", "bob"),
                ("password", "correct-password"),
            ])
            .await;
        res.assert_status(axum::http::StatusCode::SEE_OTHER);
    }
}

#[tokio::test]
async fn rate_limited_request_does_not_reach_the_password_check() {
    let (mut server, store) = server().await;
    create_user(&store, "bob", "correct-password").await;

    for _ in 0..pingward::ratelimit::MAX_ATTEMPTS {
        failed_login(&mut server, "bob").await;
    }
    let sessions_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&store.pool)
        .await
        .unwrap();

    // Now throttled — even the *correct* password must not create a session.
    let csrf = common::anonymous_csrf(&mut server).await;
    let res = server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "bob"),
            ("password", "correct-password"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);

    let sessions_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(
        sessions_before, sessions_after,
        "a throttled request must not create a session even with valid credentials"
    );
}

// --- the per-account limiter ---
//
// These need the per-address limiter out of the way. `axum-test` gives every
// request the same loopback bucket (see the module doc above), so with both
// limiters at their real settings the 5-per-minute address budget is spent
// before the 10-per-15-minutes account budget is, and the account one would
// never be reached. `AppState::account_limiter` and `login_limiter` are public
// fields, so swapping in a permissive address limiter needs no test-only seam
// in the production type.

use pingward::ratelimit::{ACCOUNT_MAX_ATTEMPTS, ACCOUNT_WINDOW_SECS, RateLimiter};
use std::sync::Arc;

/// A server whose per-address limiter is effectively disabled, leaving the
/// account limiter as the only thing that can refuse.
async fn server_without_ip_limiting() -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let mut state = AppState::new(store.clone(), common::test_config());
    state.login_limiter = Arc::new(RateLimiter::new(u32::MAX, 60));
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    (server, store)
}

#[tokio::test]
async fn an_account_is_locked_after_its_own_budget_regardless_of_source() {
    let (mut server, store) = server_without_ip_limiting().await;
    create_user(&store, "bob", "correct-password").await;

    for _ in 0..ACCOUNT_MAX_ATTEMPTS {
        failed_login(&mut server, "bob").await.assert_status_ok();
    }

    let res = failed_login(&mut server, "bob").await;
    res.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        res.header("retry-after"),
        ACCOUNT_WINDOW_SECS.to_string(),
        "the account window is the one that has to elapse, not the address one"
    );
}

/// The lockout is per-account, not a global kill switch: locking "bob" must
/// not keep everyone else out. This is what makes the control usable at all —
/// otherwise one sprayed username would deny the whole instance.
#[tokio::test]
async fn locking_one_account_leaves_another_signable_in() {
    let (mut server, store) = server_without_ip_limiting().await;
    create_user(&store, "bob", "correct-password").await;
    create_user(&store, "carol", "carol-password").await;

    for _ in 0..=ACCOUNT_MAX_ATTEMPTS {
        failed_login(&mut server, "bob").await;
    }

    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "carol"),
            ("password", "carol-password"),
        ])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
}

/// The enumeration property. A username that does not exist must exhaust and
/// trip the limiter exactly as a real one does — if the throttle only engaged
/// for real accounts, *being throttled* would answer "does this user exist?",
/// giving back at this layer what `auth::verify_password_or_dummy` protects at
/// the next.
#[tokio::test]
async fn an_unknown_username_is_locked_out_just_like_a_real_one() {
    let (mut server, store) = server_without_ip_limiting().await;
    create_user(&store, "bob", "correct-password").await;

    for _ in 0..ACCOUNT_MAX_ATTEMPTS {
        failed_login(&mut server, "no-such-user")
            .await
            .assert_status_ok();
    }

    let res = failed_login(&mut server, "no-such-user").await;
    res.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.header("retry-after"), ACCOUNT_WINDOW_SECS.to_string());
    // Byte-identical to the real account's refusal, body included.
    let mut real = server_without_ip_limiting().await;
    create_user(&real.1, "bob", "correct-password").await;
    for _ in 0..ACCOUNT_MAX_ATTEMPTS {
        failed_login(&mut real.0, "bob").await;
    }
    let real_res = failed_login(&mut real.0, "bob").await;
    assert_eq!(real_res.status_code(), res.status_code());
    assert_eq!(real_res.header("retry-after"), res.header("retry-after"));
}

/// A success clears the account bucket outright rather than refunding the one
/// attempt it cost (`RateLimiter::clear`). An owner who mistypes their way to
/// the edge of the lockout and then signs in correctly must get a *full*
/// budget back, not a single attempt — otherwise the availability cost of this
/// control lands on exactly the person it is meant to protect.
#[tokio::test]
async fn a_successful_login_clears_the_account_lockout_budget() {
    let (mut server, store) = server_without_ip_limiting().await;
    create_user(&store, "bob", "correct-password").await;

    for _ in 0..ACCOUNT_MAX_ATTEMPTS - 1 {
        failed_login(&mut server, "bob").await.assert_status_ok();
    }

    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "bob"),
            ("password", "correct-password"),
        ])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);

    // The whole window is available again: a refund of one would have run out
    // after a single further failure.
    for _ in 0..ACCOUNT_MAX_ATTEMPTS {
        failed_login(&mut server, "bob").await.assert_status_ok();
    }
    failed_login(&mut server, "bob")
        .await
        .assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
}

/// The accepted cost of the control, pinned so it is a decision rather than a
/// surprise: once the account budget is spent, the **correct** password is
/// refused too, and no session is created. This is the denial-of-service
/// primitive an account lockout necessarily hands to whoever knows a username
/// (see `ratelimit::ACCOUNT_MAX_ATTEMPTS`).
#[tokio::test]
async fn a_locked_account_refuses_even_the_correct_password() {
    let (mut server, store) = server_without_ip_limiting().await;
    create_user(&store, "bob", "correct-password").await;

    for _ in 0..ACCOUNT_MAX_ATTEMPTS {
        failed_login(&mut server, "bob").await;
    }

    let csrf = common::anonymous_csrf(&mut server).await;
    let res = server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "bob"),
            ("password", "correct-password"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);

    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(sessions, 0, "a throttled request must not create a session");
}
