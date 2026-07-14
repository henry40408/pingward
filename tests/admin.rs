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
async fn admin_sees_admin_nav_link_on_dashboard() {
    let (server, store, _admin_id) = admin_server().await;
    let body = server.get("/").await.text();
    assert!(
        body.contains(r#"href="/admin""#),
        "admin's own dashboard should show the Admin nav link"
    );

    // A separate, non-admin member must NOT see the Admin nav link on their
    // own dashboard, proving the link reflects the viewer, not the route.
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let mut member_server = TestServer::new(app(state));
    member_server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    store
        .create_user("member", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    member_server
        .post("/login")
        .form(&[("username", "member"), ("password", "pw")])
        .await;
    let member_body = member_server.get("/").await.text();
    assert!(
        !member_body.contains(r#"href="/admin""#),
        "non-admin member should not see the Admin nav link"
    );
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

#[tokio::test]
async fn admin_mutation_on_other_project_is_audited() {
    let (server, store, _admin_id) = admin_server().await;
    let owner = store
        .create_user("owner2", Some("phc"), false, chrono::Utc::now())
        .await
        .unwrap();
    let pid = store
        .create_project(owner, "p", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let cid = store
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
    server
        .post(&format!("/admin/checks/{cid}/pause"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    // Check is paused and the access was audited.
    assert_eq!(
        store.find_check(cid).await.unwrap().unwrap().status,
        pingward::models::CheckStatus::Paused
    );
    let audit = store.list_audit(50).await.unwrap();
    assert!(audit
        .iter()
        .any(|a| a.target_type.as_deref() == Some("check")
            && a.target_id == Some(cid)
            && a.method.as_deref() == Some("POST")));
}
