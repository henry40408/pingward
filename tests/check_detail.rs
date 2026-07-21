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
        .create_project(uid, "web", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    (server, store, pid)
}

#[tokio::test]
async fn check_detail_shows_heartbeat_body_and_source() {
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
    let check = store.find_check(cid).await.unwrap().unwrap();

    let res = server
        .post(&format!("/ping/{}/fail", check.ping_uuid))
        .text("boom trace")
        .await;
    res.assert_status_ok();

    let res = server.get(&format!("/checks/{cid}")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("class=\"beat\""), "heartbeat missing: {body}");
    assert!(
        body.contains("boom trace"),
        "captured ping body not surfaced: {body}"
    );
    assert!(body.contains("Source"), "source column missing: {body}");
}

#[tokio::test]
async fn ping_timestamps_are_localizable_with_utc_fallback() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            ping_uuid: "cu2",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    let check = store.find_check(cid).await.unwrap().unwrap();

    server
        .post(&format!("/ping/{}", check.ping_uuid))
        .text("ok")
        .await
        .assert_status_ok();

    let res = server.get(&format!("/checks/{cid}")).await;
    res.assert_status_ok();
    let body = res.text();
    // Absolute timestamps are emitted as RFC3339 UTC the client localizes.
    assert!(
        body.contains("class=\"localtime\" data-ts=\""),
        "no localizable timestamp emitted: {body}"
    );
    assert!(
        body.contains("+00:00"),
        "data-ts should be RFC3339 UTC: {body}"
    );
    // The no-JS fallback shows a full date labeled UTC (not a bare HH:MM:SS).
    assert!(
        body.contains(" UTC</span>"),
        "fallback should show a UTC date-time: {body}"
    );
}

/// XSS regression: a check description carrying `<img onerror=...>` and a
/// `javascript:` link must never reach the rendered check page as live markup.
/// Mirrors `project_view.rs::project_description_neutralizes_xss_payloads`.
#[tokio::test]
async fn check_description_neutralizes_xss_payloads() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            description: "<img src=x onerror=alert(1)> and [x](javascript:alert(1))",
            ping_uuid: "cu-xss",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();

    let res = server.get(&format!("/checks/{cid}")).await;
    res.assert_status_ok();
    let body = res.text();
    // The escaped `onerror=alert(1)` text is expected to still appear as
    // inert page *content* (the whole `<img ...>` became literal text) — a
    // bare `!contains("onerror=alert(1)")` would be wrong, and `!contains
    // ("onerror")` alone would false-positive on base.html's own unrelated
    // `liveSource.onerror = ...` JS. What must never appear is a *live* tag
    // or attribute built from the payload.
    assert!(
        !body.contains("<img "),
        "a raw <img> tag leaked into rendered page: {body}"
    );
    assert!(
        !body.contains("href=\"javascript:"),
        "a live javascript: href leaked into rendered page: {body}"
    );
    assert!(
        body.contains("&lt;img"),
        "escaped <img must be present as literal text: {body}"
    );
    assert!(
        body.contains("data-testid=\"check-description\""),
        "description block missing: {body}"
    );
}

/// A check description's markdown renders on the check detail page
/// (`**bold**` becomes `<strong>`).
#[tokio::test]
async fn check_description_markdown_renders_on_check_page() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            description: "Runs **nightly** at 2am.",
            ping_uuid: "cu-md",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();

    let res = server.get(&format!("/checks/{cid}")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains("<strong>nightly</strong>"),
        "check description markdown not rendered: {body}"
    );
}
