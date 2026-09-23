//! `POST /login` per-IP and per-account rate limiting (`crate::ratelimit`).
//!
//! `axum-test` sets no `ConnectInfo`, so every request shares
//! `ratelimit::rate_limit_key`'s loopback fallback bucket.

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

/// A wrong-password `POST /login`, each with its own anonymous session's CSRF token.
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

    // Throttled: even the correct password must not create a session.
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
// The shared loopback bucket's 5/min would trip before the account's 10/15min,
// so these swap in a permissive `AppState::login_limiter`.

use pingward::ratelimit::{ACCOUNT_MAX_ATTEMPTS, ACCOUNT_WINDOW_SECS, RateLimiter};
use std::sync::Arc;

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

/// Otherwise one sprayed username would lock out the whole instance.
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

/// Otherwise being throttled would reveal whether a username exists.
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

/// A success must `clear` the account bucket, not refund one attempt.
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

    // A refund of one would have run out after a single further failure.
    for _ in 0..ACCOUNT_MAX_ATTEMPTS {
        failed_login(&mut server, "bob").await.assert_status_ok();
    }
    failed_login(&mut server, "bob")
        .await
        .assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
}

/// Pins the accepted cost: a lockout is a `DoS` primitive for anyone who knows
/// the username.
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
