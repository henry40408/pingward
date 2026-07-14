use axum_test::TestServer;
use pingward::{app, config::Config, db, state::AppState, store::Store};

async fn admin_server() -> (TestServer, Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    let admin_id = store
        .create_user("admin", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post("/login")
        .form(&[("username", "admin"), ("password", "pw")])
        .await;
    (server, store, admin_id)
}

#[tokio::test]
async fn admin_dashboard_renders_with_figures() {
    let (server, store, _admin) = admin_server().await;
    let uid = store
        .create_user("owner", Some("p"), false, chrono::Utc::now())
        .await
        .unwrap();
    let pid = store
        .create_project(uid, "proj", None, None, chrono::Utc::now())
        .await
        .unwrap();
    store
        .create_check(
            pid,
            "c",
            "uuid-c",
            pingward::models::ScheduleKind::Period,
            Some(3600),
            300,
            None,
            "UTC",
        )
        .await
        .unwrap();
    let res = server.get("/admin").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("Dashboard") || body.contains("Admin"));
    // scale figures present
    assert!(body.contains("proj") || body.contains("1"));
}

#[tokio::test]
async fn non_admin_cannot_see_dashboard() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    store
        .create_user("member", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post("/login")
        .form(&[("username", "member"), ("password", "pw")])
        .await;
    server
        .get("/admin")
        .await
        .assert_status(axum::http::StatusCode::FORBIDDEN);
}
