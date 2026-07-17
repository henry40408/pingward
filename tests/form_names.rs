use axum_test::TestServer;
use pingward::{app, config::Config, db, models::ScheduleKind, state::AppState, store::Store};

/// After a session exists, configure the `TestServer` to send that session's
/// CSRF synchronizer token as a default `X-CSRF-Token` header so protected
/// POSTs are not rejected by `csrf_guard`. Call after every (re)login.
async fn set_csrf(server: &mut TestServer, store: &Store) {
    let tok = sqlx::query_scalar::<_, String>(
        "SELECT csrf_token FROM sessions ORDER BY expires_at DESC LIMIT 1",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap();
    server.add_header("x-csrf-token", tok.as_str());
}

/// A `TestServer` on a fresh in-memory DB, signed in as a newly created user
/// and ready to POST. Returns the user's id.
async fn server_as(username: &str, is_admin: bool) -> (TestServer, Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user(username, Some(&phc), is_admin, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post("/login")
        .form(&[("username", username), ("password", "pw")])
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

/// Creating a project with a padded name must store it trimmed — the form
/// validation in `validate_project` checks `trim().is_empty()`, but a
/// reverted handler could still hand the raw, untrimmed name to the store.
#[tokio::test]
async fn project_create_stores_a_trimmed_name() {
    let (server, store, _uid) = logged_in_server().await;
    let res = server
        .post("/projects")
        .form(&[
            ("name", "  Nightly jobs  "),
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

/// Creating a check with a padded name must store it trimmed, mirroring the
/// project test above.
#[tokio::test]
async fn check_create_stores_a_trimmed_name() {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("name", "  backup  "),
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

/// Updating a check with a padded name must store it trimmed. The
/// `check_update_core` handler uses `validate_check`, which trims the name —
/// but a reverted handler could still hand the raw, untrimmed name to the store.
#[tokio::test]
async fn check_update_stores_a_trimmed_name() {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let id = store
        .create_check(
            pid,
            "backup",
            "uuid-test-check",
            ScheduleKind::Period,
            Some(60),
            300,
            None,
            "UTC",
        )
        .await
        .unwrap();
    let res = server
        .post(&format!("/checks/{id}"))
        .form(&[
            ("name", "  renamed  "),
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

/// Updating a project with a padded name must store it trimmed, mirroring the
/// check update test above.
#[tokio::test]
async fn project_update_stores_a_trimmed_name() {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let res = server
        .post(&format!("/projects/{pid}"))
        .form(&[
            ("name", "  Renamed jobs  "),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let stored = store.find_project(pid).await.unwrap().unwrap();
    assert_eq!(stored.name, "Renamed jobs");
}

/// Admin updating a project with a padded name must store it trimmed. The
/// `admin_project_update` handler is a separate route (`POST /admin/projects/{id}`)
/// and requires an admin user, but uses the same `validate_project` logic.
#[tokio::test]
async fn admin_project_update_stores_a_trimmed_name() {
    let (server, store, admin_id) = admin_server().await;
    let pid = store
        .create_project(admin_id, "web", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let res = server
        .post(&format!("/admin/projects/{pid}"))
        .form(&[
            ("name", "  Admin renamed  "),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let stored = store.find_project(pid).await.unwrap().unwrap();
    assert_eq!(stored.name, "Admin renamed");
}
