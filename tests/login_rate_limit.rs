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
