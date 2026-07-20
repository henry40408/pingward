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
    let body = server.get("/admin").await.text();
    let token = extract_csrf(&body);
    assert!(
        !token.is_empty(),
        "rendered form must embed a non-empty _csrf token"
    );

    // The embedded token alone (no header) authorizes the form submission.
    server
        .post("/admin/users")
        .form(&[
            ("_csrf", token.as_str()),
            ("username", "bob"),
            ("password", "pw"),
        ])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);

    // The same submission with the `_csrf` field omitted is rejected.
    server
        .post("/admin/users")
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

// (b'') The multi-value "Save channels" form authorizes via the `_csrf` form
// field even though it also carries repeated `channel_ids` keys — i.e. the guard
// finds `_csrf` regardless of its position among the urlencoded pairs.
#[tokio::test]
async fn multi_value_channels_form_authorizes_via_csrf_field() {
    let (server, store) = logged_in_server().await;
    let admin = store.find_user_by_username("admin").await.unwrap().unwrap();
    let pid = store
        .create_project(admin.id, "p", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "c",
            ping_uuid: "uuid-csrf-chan",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    let ch1 = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "w1",
            "{\"url\":\"http://x\"}",
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    let ch2 = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "w2",
            "{\"url\":\"http://y\"}",
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    let tok = csrf_token(&store).await;
    // Repeated `channel_ids` keys plus a trailing `_csrf` field (no header).
    server
        .post(&format!("/checks/{cid}/channels"))
        .form(&[
            ("channel_ids", ch1.to_string().as_str()),
            ("channel_ids", ch2.to_string().as_str()),
            ("_csrf", tok.as_str()),
        ])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    // Both channels were actually bound (the rebuilt body reached the handler).
    let bound = store.bound_channel_ids(cid).await.unwrap();
    assert!(bound.contains(&ch1) && bound.contains(&ch2));
    // The same submission with `_csrf` omitted is rejected.
    server
        .post(&format!("/checks/{cid}/channels"))
        .form(&[("channel_ids", ch1.to_string().as_str())])
        .await
        .assert_status(axum::http::StatusCode::FORBIDDEN);
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
