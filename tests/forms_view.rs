use axum_test::TestServer;
use pingward::{app, config::Config, state::AppState, store::Store};

async fn server() -> (TestServer, Store) {
    let pool = pingward::db::connect("sqlite::memory:").await.unwrap();
    pingward::db::migrate(&pool, "sqlite::memory:")
        .await
        .unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    (server, store)
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

async fn server_with_project() -> (TestServer, Store, i64) {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", None, None, chrono::Utc::now())
        .await
        .unwrap();
    (server, store, pid)
}

/// The restyled channel form must keep the `.field` form-control class from
/// `assets/app.css` (Task 1) while preserving every existing input name that
/// the (unchanged) handler in `src/web.rs` depends on.
#[tokio::test]
async fn channel_form_is_restyled_and_keeps_fields() {
    let (server, _store, pid) = server_with_project().await;
    let res = server.get(&format!("/projects/{pid}/channels/new")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("class=\"field\""), "form not restyled");
    assert!(body.contains("name=\"webhook_url\""), "webhook field lost");
}

/// The restyled check form must keep the `.field` class and every field name
/// the handler in `src/web.rs` reads via `CheckForm`.
#[tokio::test]
async fn check_form_is_restyled_and_keeps_fields() {
    let (server, _store, pid) = server_with_project().await;
    let res = server.get(&format!("/projects/{pid}/checks/new")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("class=\"field\""), "form not restyled");
    for name in [
        "name",
        "schedule_kind",
        "period_secs",
        "cron_expr",
        "grace_secs",
        "timezone",
        "scan_interval_secs",
        "max_runtime_secs",
        "nag_interval_secs",
    ] {
        assert!(
            body.contains(&format!("name=\"{name}\"")),
            "check form lost field {name}"
        );
    }
}

/// The restyled project form must keep the `.field` class and every field
/// name the handler in `src/web.rs` reads via `ProjectForm`.
#[tokio::test]
async fn project_form_is_restyled_and_keeps_fields() {
    let (server, _store, _uid) = logged_in_server().await;
    let res = server.get("/projects/new").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("class=\"field\""), "form not restyled");
    for name in ["name", "scan_interval_secs", "nag_interval_secs"] {
        assert!(
            body.contains(&format!("name=\"{name}\"")),
            "project form lost field {name}"
        );
    }
}
