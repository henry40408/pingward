use axum_test::TestServer;
use pingward::{app, config::Config, db, state::AppState, store::Store};

async fn logged_in_server() -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    store
        .create_user("admin", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post("/login")
        .form(&[("username", "admin"), ("password", "pw")])
        .await;
    (server, store)
}

/// Read the current session's CSRF synchronizer token straight from the DB.
async fn csrf_token(store: &Store) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT csrf_token FROM sessions ORDER BY expires_at DESC LIMIT 1",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap()
}

// (a) A protected POST with a valid session cookie but NO token → 403.
#[tokio::test]
async fn protected_post_without_token_is_forbidden() {
    let (server, _store) = logged_in_server().await;
    server
        .post("/projects")
        .form(&[
            ("name", "web"),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await
        .assert_status(axum::http::StatusCode::FORBIDDEN);
}

// (b) Same POST WITH the header token → not 403 (303 redirect).
#[tokio::test]
async fn protected_post_with_header_token_succeeds() {
    let (mut server, store) = logged_in_server().await;
    let tok = csrf_token(&store).await;
    server.add_header("x-csrf-token", tok.as_str());
    server
        .post("/projects")
        .form(&[
            ("name", "web"),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
}

// (b') The `_csrf` form-field path also authorizes the request (Task 5's path).
#[tokio::test]
async fn protected_post_with_form_field_token_succeeds() {
    let (server, store) = logged_in_server().await;
    let tok = csrf_token(&store).await;
    server
        .post("/projects")
        .form(&[
            ("name", "web"),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
            ("_csrf", tok.as_str()),
        ])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
}

// A wrong token is rejected even with a valid session.
#[tokio::test]
async fn protected_post_with_wrong_token_is_forbidden() {
    let (mut server, _store) = logged_in_server().await;
    server.add_header("x-csrf-token", "not-the-real-token");
    server
        .post("/projects")
        .form(&[
            ("name", "web"),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await
        .assert_status(axum::http::StatusCode::FORBIDDEN);
}

// (c) POST /ping/{uuid} carries no token and lives in the exempt ping router.
#[tokio::test]
async fn ping_post_needs_no_csrf() {
    let (server, _store) = logged_in_server().await;
    let res = server
        .post(&format!("/ping/{}", uuid::Uuid::new_v4()))
        .await;
    assert_ne!(res.status_code(), axum::http::StatusCode::FORBIDDEN);
}

// (d) POST /login is exempt (pre-session) and needs no token.
#[tokio::test]
async fn login_post_needs_no_csrf() {
    let (server, _store) = logged_in_server().await;
    // Re-authenticating (valid creds, no CSRF header) still succeeds.
    server
        .post("/login")
        .form(&[("username", "admin"), ("password", "pw")])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
}
