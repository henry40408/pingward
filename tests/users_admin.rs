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
async fn creating_user_is_audited() {
    let (server, store, _admin) = admin_server().await;
    server
        .post("/users")
        .form(&[("username", "carol"), ("password", "pw123456")])
        .await;
    let carol = store.find_user_by_username("carol").await.unwrap().unwrap();
    let audit = store.list_audit(50).await.unwrap();
    assert!(audit.iter().any(|a| a.action == "user.create"
        && a.target_type.as_deref() == Some("user")
        && a.target_id == Some(carol.id)));
}

#[tokio::test]
async fn deleting_user_is_audited() {
    let (server, store, _admin) = admin_server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let dave = store
        .create_user("dave", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    server.post(&format!("/users/{dave}/delete")).await;
    let audit = store.list_audit(50).await.unwrap();
    assert!(audit.iter().any(|a| a.action == "user.delete"
        && a.target_type.as_deref() == Some("user")
        && a.target_id == Some(dave)));
}

#[tokio::test]
async fn admin_resets_password_and_target_can_login() {
    let (server, store, _admin) = admin_server().await;
    let phc = pingward::auth::hash_password("original").unwrap();
    store
        .create_user("dave", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    let dave = store.find_user_by_username("dave").await.unwrap().unwrap();
    server
        .post(&format!("/users/{}/password", dave.id))
        .form(&[("password", "brandnew1")])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    let updated = store.find_user_by_id(dave.id).await.unwrap().unwrap();
    assert!(pingward::auth::verify_password(
        "brandnew1",
        updated.password_hash.as_deref().unwrap()
    ));
    assert!(store
        .list_audit(50)
        .await
        .unwrap()
        .iter()
        .any(|a| a.action == "user.password_reset" && a.target_id == Some(dave.id)));
}
