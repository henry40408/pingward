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

/// Pull the value of the first `name="_csrf"` hidden input out of a rendered
/// HTML body — the token a real browser would echo back on form submission.
fn extract_csrf(html: &str) -> String {
    let marker = "name=\"_csrf\" value=\"";
    let start = html
        .find(marker)
        .expect("rendered form must embed a _csrf field")
        + marker.len();
    let end = html[start..].find('"').expect("unterminated _csrf value");
    html[start..start + end].to_string()
}

// End-to-end form path: the token embedded in a rendered form authorizes a real
// browser-style POST (no `X-CSRF-Token` header), and omitting it is rejected.
#[tokio::test]
async fn form_includes_csrf_and_form_post_succeeds() {
    let (server, _store) = logged_in_server().await;
    // GET a page carrying a protected form and read the embedded token from the HTML.
    let body = server.get("/users").await.text();
    let token = extract_csrf(&body);
    assert!(
        !token.is_empty(),
        "rendered form must embed a non-empty _csrf token"
    );

    // The embedded token alone (no header) authorizes the form submission.
    server
        .post("/users")
        .form(&[
            ("_csrf", token.as_str()),
            ("username", "bob"),
            ("password", "pw"),
        ])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);

    // The same submission with the `_csrf` field omitted is rejected.
    server
        .post("/users")
        .form(&[("username", "carol"), ("password", "pw")])
        .await
        .assert_status(axum::http::StatusCode::FORBIDDEN);
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
