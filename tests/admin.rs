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
async fn non_admin_forbidden_on_admin_routes() {
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

#[tokio::test]
async fn admin_views_other_users_project_and_audits() {
    let (server, store, _admin_id) = admin_server().await;
    // A separate user owns a project + check.
    let owner = store
        .create_user("owner", Some("phc"), false, chrono::Utc::now())
        .await
        .unwrap();
    let pid = store
        .create_project(owner, "victim", None, None, chrono::Utc::now())
        .await
        .unwrap();
    // Admin can see it via /admin, owner-scoped route would 404.
    server.get("/projects").await; // (owner route is per-user; admin uses /admin)
    server
        .get(&format!("/admin/projects/{pid}"))
        .await
        .assert_status_ok();
    let audit = store.list_audit(10).await.unwrap();
    assert!(audit.iter().any(|a| a.action == "admin.access"
        && a.target_type.as_deref() == Some("project")
        && a.target_id == Some(pid)
        && a.target_owner_id == Some(owner)));
}
