use axum_test::TestServer;
use pingward::{app, state::AppState, store::Store};

mod common;

async fn server() -> (TestServer, Store) {
    let pool = pingward::db::connect("sqlite::memory:").await.unwrap();
    pingward::db::migrate(&pool, "sqlite::memory:")
        .await
        .unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    (server, store)
}

async fn logged_in_server() -> (TestServer, Store, i64) {
    let (mut server, store) = server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
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

/// XSS regression, mirroring
/// `project_view.rs::project_description_neutralizes_xss_payloads`.
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
    // The escaped payload still appears as inert page *content*, so a bare
    // `!contains("onerror=alert(1)")` would be wrong and `!contains("onerror")`
    // would false-positive on base.html's own `liveSource.onerror`.
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

#[tokio::test]
async fn check_detail_shows_when_the_next_ping_is_due() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            ping_uuid: "cu-due",
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
        .await
        .assert_status_ok();

    let res = server.get(&format!("/checks/{cid}")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains("data-testid=\"check-next-due\""),
        "next-due element missing: {body}"
    );
    // 1h period + 5m grace, pinged just now.
    assert!(
        body.contains("due in 1h"),
        "next deadline not counted down: {body}"
    );
    assert!(
        body.contains("check-next-due\" title=\""),
        "next-due tooltip carrying the absolute instant missing: {body}"
    );
}

/// A never-pinged check still has a real deadline (`scan_once` will down it),
/// but the label must not read as a report about a run that happened.
#[tokio::test]
async fn check_detail_next_due_names_the_first_ping_when_none_has_arrived() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            ping_uuid: "cu-due-new",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    // Precondition: `next_due_at` is unstamped, so the page cannot be reading it.
    assert!(
        store
            .find_check(cid)
            .await
            .unwrap()
            .unwrap()
            .next_due_at
            .is_none()
    );

    let res = server.get(&format!("/checks/{cid}")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains("first ping due in 1h"),
        "never-pinged check should name the first ping: {body}"
    );
}

/// A paused check is excluded from monitoring, so no deadline may be shown.
#[tokio::test]
async fn check_detail_paused_shows_no_deadline() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            ping_uuid: "cu-due-paused",
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
        .await
        .assert_status_ok();
    store
        .set_status(cid, pingward::models::CheckStatus::Paused)
        .await
        .unwrap();

    let res = server.get(&format!("/checks/{cid}")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains("not scheduled while paused"),
        "paused check should name the state: {body}"
    );
    assert!(
        !body.contains("due in"),
        "paused check must not count down to a deadline nothing enforces: {body}"
    );
}

/// The strip renders past the widest viewport: `assets/app.css` clips the
/// overflow from the left, so the server cap decides whether a wide screen can
/// fill its width. Locked here so "why render bars nobody sees?" cannot shrink it.
#[tokio::test]
async fn the_heartbeat_renders_more_bars_than_a_viewport_fits() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "busy",
            ping_uuid: "cu-busy",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    let check = store.find_check(cid).await.unwrap().unwrap();

    // Start/success pairs, the shape the window is sized for: 150 runs exceeds
    // the cap, so the page must clamp.
    for _ in 0..150 {
        server
            .post(&format!("/ping/{}/start", check.ping_uuid))
            .await
            .assert_status_ok();
        server
            .post(&format!("/ping/{}", check.ping_uuid))
            .await
            .assert_status_ok();
    }

    let body = server.get(&format!("/checks/{cid}")).await.text();
    let strip = body
        .split_once("class=\"beat\"")
        .expect("heartbeat strip")
        .1
        .split_once("</div>")
        .expect("strip closes")
        .0;
    let bars = strip.matches("<i ").count();
    assert_eq!(bars, 120, "expected the full {} bars, got {bars}", 120);

    // The caption must not name a count: only the browser knows how many show.
    assert!(!body.contains("30 runs ago"), "stale fixed-count caption");
}
