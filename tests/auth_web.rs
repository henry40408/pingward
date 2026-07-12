use axum_test::TestServer;
use pingward::{app, config::Config, db, state::AppState, store::Store};

async fn server() -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    // axum-test 21's `TestServer::new` returns `Self` directly (it panics
    // internally on failure rather than returning a `Result`), matching the
    // note in `tests/ping_api.rs`.
    let mut server = TestServer::new(app(state));
    // axum-test 21 names this `save_cookies` (the brief's `do_save_cookies`
    // does not exist on `TestServer` — that name is used by `TestRequest`
    // instead). Persists Set-Cookie between requests.
    server.save_cookies();
    (server, store)
}

#[tokio::test]
async fn setup_creates_admin_then_dashboard_loads() {
    let (server, store) = server().await;

    // With no users, root redirects to /setup.
    let res = server.get("/").await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/setup");

    // Create the first admin.
    let res = server
        .post("/setup")
        .form(&[("username", "admin"), ("password", "pw12345")])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(store.count_users().await.unwrap(), 1);
    let admin = store.find_user_by_username("admin").await.unwrap().unwrap();
    assert!(admin.is_admin);

    // Now authenticated (cookie saved) — dashboard renders 200.
    server.get("/").await.assert_status_ok();
}

async fn logged_in_server() -> (TestServer, Store, i64) {
    let (server, store) = server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user("admin", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post("/login")
        .form(&[("username", "admin"), ("password", "pw")])
        .await;
    (server, store, uid)
}

#[tokio::test]
async fn create_and_delete_project() {
    let (server, store, uid) = logged_in_server().await;

    let res = server
        .post("/projects")
        .form(&[("name", "web"), ("scan_interval_secs", "")])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let projects = store.list_projects_for_user(uid).await.unwrap();
    assert_eq!(projects.len(), 1);
    let pid = projects[0].id;

    server
        .get(&format!("/projects/{pid}"))
        .await
        .assert_status_ok();

    server
        .post(&format!("/projects/{pid}/delete"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.list_projects_for_user(uid).await.unwrap().is_empty());
}

#[tokio::test]
async fn cannot_view_another_users_project() {
    let (server, store, _uid) = logged_in_server().await;
    // project owned by a different user
    let other = store
        .create_user("other", Some("x"), false, chrono::Utc::now())
        .await
        .unwrap();
    let pid = store
        .create_project(other, "secret", None, chrono::Utc::now())
        .await
        .unwrap();
    server
        .get(&format!("/projects/{pid}"))
        .await
        .assert_status(axum::http::StatusCode::NOT_FOUND);
}

async fn server_with_project() -> (TestServer, Store, i64) {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", None, chrono::Utc::now())
        .await
        .unwrap();
    (server, store, pid)
}

#[tokio::test]
async fn create_check_and_pause_resume() {
    let (server, store, pid) = server_with_project().await;

    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("name", "backup"),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("grace_secs", "300"),
            ("cron_expr", ""),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let checks = store.list_checks_for_project(pid).await.unwrap();
    assert_eq!(checks.len(), 1);
    let cid = checks[0].id;

    server
        .post(&format!("/checks/{cid}/pause"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        store.find_check(cid).await.unwrap().unwrap().status,
        pingward::models::CheckStatus::Paused
    );

    server
        .post(&format!("/checks/{cid}/resume"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        store.find_check(cid).await.unwrap().unwrap().status,
        pingward::models::CheckStatus::New
    );
}

#[tokio::test]
async fn invalid_cron_is_rejected() {
    let (server, store, pid) = server_with_project().await;
    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("name", "bad"),
            ("schedule_kind", "cron"),
            ("period_secs", ""),
            ("grace_secs", "60"),
            ("cron_expr", "not a cron"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
        ])
        .await;
    res.assert_status_ok(); // re-rendered form, not a redirect
    assert!(store.list_checks_for_project(pid).await.unwrap().is_empty());
}

#[tokio::test]
async fn regenerate_uuid_changes_ping_url() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(
            pid,
            "job",
            "old-uuid",
            pingward::models::ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
        .await
        .unwrap();
    server
        .post(&format!("/checks/{cid}/regenerate"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_ne!(
        store.find_check(cid).await.unwrap().unwrap().ping_uuid,
        "old-uuid"
    );
}

#[tokio::test]
async fn login_logout_cycle() {
    let (server, store) = server().await;
    let phc = pingward::auth::hash_password("secret1").unwrap();
    store
        .create_user("bob", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();

    // wrong password → back to login with 200 + error
    server
        .post("/login")
        .form(&[("username", "bob"), ("password", "nope")])
        .await
        .assert_status_ok();

    // right password → redirect, cookie set
    let res = server
        .post("/login")
        .form(&[("username", "bob"), ("password", "secret1")])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    server.get("/").await.assert_status_ok();

    // logout → redirect, then root bounces to /login
    server
        .post("/logout")
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    let res = server.get("/").await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/login");
}

#[tokio::test]
async fn create_channel_and_bind_to_check() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(
            pid,
            "job",
            "cu",
            pingward::models::ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
        .await
        .unwrap();

    // create a webhook channel
    let res = server
        .post(&format!("/projects/{pid}/channels"))
        .form(&[
            ("name", "hook"),
            ("kind", "webhook"),
            ("url", "http://example.test/h"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let channels = store.list_channels_for_project(pid).await.unwrap();
    assert_eq!(channels.len(), 1);
    let chid = channels[0].id;
    assert!(channels[0].config_json.contains("example.test"));

    // bind it to the check
    server
        .post(&format!("/checks/{cid}/channels"))
        .form(&[("channel_ids", chid.to_string().as_str())])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(store.bound_channel_ids(cid).await.unwrap(), vec![chid]);

    // unbind by submitting no channel_ids
    server
        .post(&format!("/checks/{cid}/channels"))
        .form(&[("_", "")])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.bound_channel_ids(cid).await.unwrap().is_empty());
}
