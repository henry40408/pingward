use axum_test::TestServer;
use pingward::{
    app, db,
    models::ScheduleKind,
    state::AppState,
    store::{NewCheck, Store},
};

mod common;

/// Sends the current session's CSRF token as a default `X-CSRF-Token` header,
/// so protected POSTs are not rejected by `csrf_guard`. Call after every login.
async fn set_csrf(server: &mut TestServer, store: &Store) {
    let tok = common::newest_session_csrf(&store.pool).await;
    server.add_header("x-csrf-token", tok.as_str());
}

/// A `TestServer` on a fresh in-memory DB, signed in as a new user (id returned).
async fn server_as(username: &str, is_admin: bool) -> (TestServer, Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user(username, Some(&phc), is_admin, chrono::Utc::now())
        .await
        .unwrap();
    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", username),
            ("password", "pw"),
        ])
        .await;
    set_csrf(&mut server, &store).await;
    (server, store, uid)
}

async fn logged_in_server() -> (TestServer, Store, i64) {
    server_as("owner", false).await
}

async fn admin_server() -> (TestServer, Store, i64) {
    server_as("admin", true).await
}

/// `validate_project` only checks `trim().is_empty()`, so a handler could still
/// hand the raw, untrimmed name to the store.
#[tokio::test]
async fn project_create_stores_a_trimmed_name() {
    let (server, store, _uid) = logged_in_server().await;
    let res = server
        .post("/projects")
        .form(&[
            ("name", "  Nightly jobs  "),
            ("description", ""),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let location = res.header("location");
    let location = location.to_str().unwrap();
    let pid: i64 = location
        .rsplit('/')
        .next()
        .unwrap()
        .parse()
        .expect("redirect should point at /projects/{id}");
    let stored = store.find_project(pid).await.unwrap().unwrap();
    assert_eq!(stored.name, "Nightly jobs");
}

#[tokio::test]
async fn check_create_stores_a_trimmed_name() {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("name", "  backup  "),
            ("description", ""),
            ("schedule_kind", "period"),
            ("period_secs", "60"),
            ("cron_expr", ""),
            ("grace_secs", "300"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let location = res.header("location");
    let location = location.to_str().unwrap();
    let id: i64 = location
        .rsplit('/')
        .next()
        .unwrap()
        .parse()
        .expect("redirect should point at /checks/{id}");
    let stored = store.find_check(id).await.unwrap().unwrap();
    assert_eq!(stored.name, "backup");
}

/// `check_update_core` validates via `validate_check`, which trims the name — a
/// reverted handler could still hand the raw one to the store.
#[tokio::test]
async fn check_update_stores_a_trimmed_name() {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let id = store
        .create_check(&NewCheck {
            project_id: pid,
            name: "backup",
            ping_uuid: "uuid-test-check",
            kind: ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    let res = server
        .post(&format!("/checks/{id}"))
        .form(&[
            ("name", "  renamed  "),
            ("description", ""),
            ("schedule_kind", "period"),
            ("period_secs", "60"),
            ("cron_expr", ""),
            ("grace_secs", "300"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let stored = store.find_check(id).await.unwrap().unwrap();
    assert_eq!(stored.name, "renamed");
}

#[tokio::test]
async fn project_update_stores_a_trimmed_name() {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let res = server
        .post(&format!("/projects/{pid}"))
        .form(&[
            ("name", "  Renamed jobs  "),
            ("description", ""),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let stored = store.find_project(pid).await.unwrap().unwrap();
    assert_eq!(stored.name, "Renamed jobs");
}

/// `admin_project_update` is a separate, admin-only route that shares
/// `validate_project`.
#[tokio::test]
async fn admin_project_update_stores_a_trimmed_name() {
    let (server, store, admin_id) = admin_server().await;
    let pid = store
        .create_project(admin_id, "web", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let res = server
        .post(&format!("/admin/projects/{pid}"))
        .form(&[
            ("name", "  Admin renamed  "),
            ("description", ""),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let stored = store.find_project(pid).await.unwrap().unwrap();
    assert_eq!(stored.name, "Admin renamed");
}
