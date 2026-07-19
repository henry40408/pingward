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
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "c",
            ping_uuid: "uuid-c",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    let res = server.get("/admin").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("Dashboard") || body.contains("Admin"));
    // scale figures present
    assert!(body.contains("proj") || body.contains('1'));
}

#[tokio::test]
async fn admin_dashboard_absolute_times_wrapped_for_local_tz() {
    let (server, store, _admin) = admin_server().await;
    let now = chrono::Utc::now();
    // Scheduler heartbeat timestamps.
    store
        .set_setting("last_scan_at", &now.to_rfc3339())
        .await
        .unwrap();
    store
        .set_setting("last_prune_at", &now.to_rfc3339())
        .await
        .unwrap();
    // A failed notification to populate the "Recent failures" table.
    let uid = store
        .create_user("o2", Some("p"), false, now)
        .await
        .unwrap();
    let pid = store
        .create_project(uid, "p2", None, None, now)
        .await
        .unwrap();
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "c2",
            ping_uuid: "uuid-c2",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    let chid = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook",
            "{\"url\":\"http://x\"}",
            now,
        )
        .await
        .unwrap();
    store
        .record_notification(
            cid,
            chid,
            pingward::notify::EventKind::Down,
            pingward::models::NotifyStatus::Error,
            Some("boom"),
            now,
        )
        .await
        .unwrap();

    let body = server.get("/admin").await.text();
    // Last scan, last prune, and the recent-failure "When" cell each carry the
    // `.localtime` class with a `data-ts` so the shared base.html script converts
    // them to the viewer's time zone (raw UTC text is only the no-JS fallback).
    // The scheduler heartbeats render the class on a <div> (not a <span>), so
    // match the class+attr pair rather than a specific tag; the trailing quote
    // after `localtime` also excludes the script's `.localtime[data-ts]` selector.
    let spans = body.matches(r#"localtime" data-ts=""#).count();
    assert!(
        spans >= 3,
        "expected >=3 localtime elements (scan, prune, failure), got {spans}"
    );
    // The scheduler heartbeat is embedded as its stored RFC3339 data-ts.
    assert!(
        body.contains(&format!(r#"data-ts="{}""#, now.to_rfc3339())),
        "last_scan_at must appear as an RFC3339 data-ts attribute"
    );
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
