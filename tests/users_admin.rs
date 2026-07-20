use axum_test::TestServer;
use pingward::{app, config::Config, db, state::AppState, store::Store};

/// After a session exists, send its CSRF token as a default `X-CSRF-Token`
/// header so protected POSTs pass `csrf_guard`. Call after every (re)login.
async fn set_csrf(server: &mut TestServer, store: &Store) {
    let tok = sqlx::query_scalar::<_, String>(
        "SELECT csrf_token FROM sessions ORDER BY expires_at DESC LIMIT 1",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap();
    server.add_header("x-csrf-token", tok.as_str());
}

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
    set_csrf(&mut server, &store).await;
    (server, store, admin_id)
}

#[tokio::test]
async fn creating_user_is_audited() {
    let (server, store, _admin) = admin_server().await;
    server
        .post("/admin/users")
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
    server.post(&format!("/admin/users/{dave}/delete")).await;
    let audit = store.list_audit(50).await.unwrap();
    assert!(audit.iter().any(|a| a.action == "user.delete"
        && a.target_type.as_deref() == Some("user")
        && a.target_id == Some(dave)));
}

#[tokio::test]
async fn deleting_nonexistent_user_writes_no_audit() {
    let (server, store, _admin) = admin_server().await;
    let before = store.list_audit(50).await.unwrap().len();
    server.post("/admin/users/99999/delete").await; // nonexistent id
    let after = store.list_audit(50).await.unwrap();
    assert!(
        !after
            .iter()
            .any(|a| a.action == "user.delete" && a.target_id == Some(99999))
    );
    assert_eq!(after.len(), before);
}

#[tokio::test]
async fn resetting_password_for_nonexistent_user_writes_no_audit() {
    let (server, store, _admin) = admin_server().await;
    server
        .post("/admin/users/99999/password")
        .form(&[("password", "whatever12")])
        .await;
    assert!(
        !store
            .list_audit(50)
            .await
            .unwrap()
            .iter()
            .any(|a| a.action == "user.password_reset" && a.target_id == Some(99999))
    );
}

#[tokio::test]
async fn promote_and_demote_admin() {
    let (server, store, _admin) = admin_server().await;
    let uid = store
        .create_user("erin", Some("p"), false, chrono::Utc::now())
        .await
        .unwrap();
    // promote
    server
        .post(&format!("/admin/users/{uid}/admin"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(uid).await.unwrap().unwrap().is_admin);
    // demote back
    server.post(&format!("/admin/users/{uid}/admin")).await;
    assert!(!store.find_user_by_id(uid).await.unwrap().unwrap().is_admin);
    assert!(
        store
            .list_audit(50)
            .await
            .unwrap()
            .iter()
            .any(|a| a.action == "user.set_admin")
    );
}

#[tokio::test]
async fn cannot_demote_self() {
    let (server, store, admin_id) = admin_server().await;
    // Only one admin exists here, so this alone can't distinguish the
    // self-guard from the (provably unreachable) last-admin guard; see
    // `demoting_self_is_refused_with_flash_even_with_a_second_admin` below
    // for a case that isolates the self-guard.
    server.post(&format!("/admin/users/{admin_id}/admin")).await;
    assert!(
        store
            .find_user_by_id(admin_id)
            .await
            .unwrap()
            .unwrap()
            .is_admin
    );
}

#[tokio::test]
async fn demoting_self_is_refused_with_flash_even_with_a_second_admin() {
    let (server, store, admin_id) = admin_server().await;
    // A second enabled admin exists, so if the self-guard were absent,
    // count_enabled_admins() would be >= 2 and the last-admin guard could
    // not explain a refusal either — isolating the self-guard as the cause.
    let phc = pingward::auth::hash_password("pw").unwrap();
    store
        .create_user("gale", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();

    let res = server.post(&format!("/admin/users/{admin_id}/admin")).await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(
        store
            .find_user_by_id(admin_id)
            .await
            .unwrap()
            .unwrap()
            .is_admin,
        "self-demote must be refused even though a second enabled admin exists"
    );
    let flash = res.maybe_cookie("pingward_flash");
    assert_eq!(
        flash.map(|c| c.value().to_string()),
        Some("users_blocked".to_string())
    );

    let body = server.get("/admin").await.text();
    assert!(
        body.contains("data-testid=\"users-flash\""),
        "flash should render once: {body}"
    );

    let body2 = server.get("/admin").await.text();
    assert!(
        !body2.contains("data-testid=\"users-flash\""),
        "flash must be one-shot: {body2}"
    );
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
        .post(&format!("/admin/users/{}/password", dave.id))
        .form(&[("password", "brandnew1")])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    let updated = store.find_user_by_id(dave.id).await.unwrap().unwrap();
    assert!(pingward::auth::verify_password(
        "brandnew1",
        updated.password_hash.as_deref().unwrap()
    ));
    assert!(
        store
            .list_audit(50)
            .await
            .unwrap()
            .iter()
            .any(|a| a.action == "user.password_reset" && a.target_id == Some(dave.id))
    );
}

#[tokio::test]
async fn disable_and_enable_member() {
    let (server, store, _admin) = admin_server().await;
    let uid = store
        .create_user("frank", Some("p"), false, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post(&format!("/admin/users/{uid}/disabled"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(uid).await.unwrap().unwrap().disabled);
    server.post(&format!("/admin/users/{uid}/disabled")).await;
    assert!(!store.find_user_by_id(uid).await.unwrap().unwrap().disabled);
    assert!(
        store
            .list_audit(50)
            .await
            .unwrap()
            .iter()
            .any(|a| a.action == "user.set_disabled")
    );
}

#[tokio::test]
async fn cannot_disable_self() {
    let (server, store, admin_id) = admin_server().await;
    // Only one admin exists here, so this alone can't distinguish the
    // self-guard from the (provably unreachable) last-admin guard — see the
    // comment on `cannot_demote_self` for the same caveat.
    server
        .post(&format!("/admin/users/{admin_id}/disabled"))
        .await;
    assert!(
        !store
            .find_user_by_id(admin_id)
            .await
            .unwrap()
            .unwrap()
            .disabled
    );
}
