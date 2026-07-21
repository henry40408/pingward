//! Renders `/projects/{id}`'s check list. `tests/auth_web.rs` already hits
//! this URL, but only to assert ownership/authorization — nothing asserts
//! what the page actually renders. A dedicated file (mirroring the
//! one-surface-per-file convention of `dashboard_view.rs` /
//! `admin_dashboard.rs`) keeps that separate from the dashboard's own tests.
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

async fn server_with_project() -> (TestServer, Store, i64) {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", None, None, chrono::Utc::now())
        .await
        .unwrap();
    (server, store, pid)
}

#[tokio::test]
async fn project_page_shows_running_badge_for_in_flight_check() {
    let (server, store, pid) = server_with_project().await;
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
    // In-flight start, no finish: stored `new`, display-status `running`.
    store
        .mark_ping(
            cid,
            pingward::models::CheckStatus::New,
            None,
            Some(chrono::Utc::now()),
            None,
        )
        .await
        .unwrap();

    let res = server.get(&format!("/projects/{pid}")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains("class=\"badge running\""),
        "running badge missing on project page"
    );
    assert!(
        body.contains("class=\"status-dot running\""),
        "running status dot missing on project page"
    );
}

/// Regression guard for this branch's behaviour change: `project.html`
/// previously rendered the raw stored status (so a stored-`up` check could
/// never show anything but "up") and now goes through `view::display_status`,
/// which surfaces "late" for a stored-`up` check inside its grace window.
#[tokio::test]
async fn project_page_shows_late_for_stored_up_check_in_grace_window() {
    let (server, store, pid) = server_with_project().await;
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
    let now = chrono::Utc::now();
    // due in 2m, grace 300s -> expected run time was 3m ago: `now` sits in
    // (expected, due], the definition of "late".
    store
        .mark_ping(
            cid,
            pingward::models::CheckStatus::Up,
            Some(now - chrono::Duration::seconds(4000)),
            None,
            Some(now + chrono::Duration::seconds(120)),
        )
        .await
        .unwrap();

    let res = server.get(&format!("/projects/{pid}")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains("class=\"badge late\""),
        "late badge missing on project page (raw stored status would show 'up')"
    );
    assert!(
        body.contains("class=\"status-dot late\""),
        "late status dot missing on project page"
    );
}
