use axum_test::TestServer;
use pingward::{app, config::Config, db, state::AppState, store::Store};

async fn logged_in_server() -> (TestServer, Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let mut server = TestServer::new(app(state));
    server.save_cookies();
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

/// Create a project and a single (never-pinged, "new") check inside it.
async fn server_with_project_and_check() -> (TestServer, Store, i64, i64) {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            ping_uuid: "cu",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    (server, store, pid, cid)
}

#[tokio::test]
async fn dashboard_renders_tiles_and_badges() {
    let (server, _store, _pid, _cid) = server_with_project_and_check().await;

    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("class=\"tiles\""), "summary tiles missing");
    assert!(body.contains("class=\"badge"), "status badge missing");
}

#[tokio::test]
async fn dashboard_shows_project_group_and_check_row() {
    let (server, _store, pid, cid) = server_with_project_and_check().await;

    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("web"), "project group name missing");
    assert!(body.contains("1 checks"), "group check count missing");
    assert!(
        body.contains(&format!("/projects/{pid}")),
        "manage link missing"
    );
    assert!(
        body.contains(&format!("/checks/{cid}")),
        "check row link missing"
    );
    assert!(body.contains("class=\"badge new\""), "new badge missing");
    assert!(
        body.contains("class=\"status-dot new\""),
        "new status dot missing"
    );
}

#[tokio::test]
async fn dashboard_empty_state_when_no_projects() {
    let (server, _store, _uid) = logged_in_server().await;
    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains("No projects yet"),
        "empty-state message missing"
    );
}
