use axum_test::TestServer;
use pingward::{app, config::Config, db, state::AppState, store::Store};

mod common;

#[tokio::test]
async fn admin_env_card_never_leaks_secrets() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let config = Config::from_map(|k| match k {
        "DATABASE_URL" => Some("postgres://pguser:pgsecret@db.internal:5432/pingward".into()),
        "PINGWARD_SMTP_HOST" => Some("smtp.example.com".into()),
        "PINGWARD_SMTP_FROM" => Some("alerts@example.com".into()),
        "PINGWARD_SMTP_PASSWORD" => Some("hunter2-smtp".into()),
        // Pinned so `common::anonymous_csrf` derives a token this server
        // accepts — an unpinned secret is random per `Config`.
        "PINGWARD_SECRET" => Some(common::TEST_SECRET.into()),
        _ => None,
    });
    let state = AppState::new(store.clone(), config);
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    store
        .create_user("admin", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();
    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "admin"),
            ("password", "pw"),
        ])
        .await;

    let res = server.get("/admin").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(!body.contains("hunter2-smtp"));
    assert!(!body.contains("pgsecret"));
    assert!(!body.contains("pguser"));
    assert!(body.contains("configured"));
    assert!(body.contains("db.internal"));
    assert!(body.contains("smtp.example.com"));
    // The username row is an identity (not a credential) and stays unset here
    // since PINGWARD_SMTP_USERNAME was never provided.
    assert!(body.contains(r#"data-testid="env-smtp-password""#));
}

#[tokio::test]
async fn admin_env_card_shows_unset_smtp_when_not_configured() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let config = common::test_config();
    let state = AppState::new(store.clone(), config);
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    store
        .create_user("admin", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();
    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "admin"),
            ("password", "pw"),
        ])
        .await;

    let res = server.get("/admin").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("DATABASE_URL"));
    assert!(body.contains("PINGWARD_SCAN_INTERVAL"));
    // No SMTP configured anywhere: the "configured" pill must never appear.
    assert!(body.contains(r#"data-testid="env-smtp-password""#));
    assert!(!body.contains(r#"<span class="pill ok">configured</span>"#));
}
