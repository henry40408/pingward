//! What still works with JavaScript switched off.
//!
//! `app.js` is progressive enhancement everywhere *except* where these tests
//! are pointed. Two things had quietly stopped being optional:
//!
//! 1. A check row's only route to the check page was the delegated `data-href`
//!    click handler, so with no JS the dashboard led nowhere at all.
//! 2. The history sections' pager and Clear controls are real `<a href>`s
//!    aimed at fragment endpoints, which answered a plain navigation with a
//!    bare partial — no `<head>`, so no stylesheet, no nav, no way back.
//!
//! Both failure modes are invisible to the browser suite, which always runs
//! with JS on. These assertions are the guard instead.

use axum::http::StatusCode;
use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};

mod common;

async fn logged_in_server() -> (TestServer, Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
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

async fn check_for(store: &Store, owner: i64, uuid: &str) -> (i64, i64) {
    let pid = store
        .create_project(owner, "web", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            ping_uuid: uuid,
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    (pid, cid)
}

// --- rows reach their page without a click handler ---

/// The dashboard's whole job is getting you to a check. The row is a `div`
/// (a flex container three templates share), so the link has to be inside it.
#[tokio::test]
async fn dashboard_check_rows_carry_a_real_link() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let body = server.get("/").await.text();
    assert!(
        body.contains(&format!("href=\"/checks/{cid}\"")),
        "dashboard row has no anchor to its check: {body}"
    );
}

#[tokio::test]
async fn project_check_rows_carry_a_real_link() {
    let (server, store, uid) = logged_in_server().await;
    let (pid, cid) = check_for(&store, uid, "cu").await;

    let body = server.get(&format!("/projects/{pid}")).await.text();
    assert!(
        body.contains(&format!("href=\"/checks/{cid}\"")),
        "project row has no anchor to its check: {body}"
    );
}

#[tokio::test]
async fn admin_project_rows_carry_a_real_link() {
    let (server, store, uid) = logged_in_server().await;
    let (pid, _cid) = check_for(&store, uid, "cu").await;

    let body = server.get("/admin").await.text();
    assert!(
        body.contains(&format!("href=\"/admin/projects/{pid}\"")),
        "admin project row has no anchor to its project: {body}"
    );
}

/// The row must not go back to simulating a link with ARIA: `role="link"` plus
/// `tabindex` buys a focus ring and Enter, and still leaves the row dead with
/// JS off — which is exactly the state this file exists to prevent.
#[tokio::test]
async fn rows_do_not_simulate_a_link_with_aria() {
    let (server, store, uid) = logged_in_server().await;
    check_for(&store, uid, "cu").await;

    for path in ["/", "/admin"] {
        let body = server.get(path).await.text();
        assert!(
            !body.contains("role=\"link\""),
            "{path} still fakes a link instead of rendering one: {body}"
        );
    }
}

// --- fragment endpoints answer a navigation with a page ---

#[tokio::test]
async fn pings_fragment_redirects_a_real_navigation_to_the_check_page() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let res = server.get(&format!("/checks/{cid}/pings")).await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(
        res.header("location"),
        format!("/checks/{cid}#pings-section")
    );
}

/// The pager cursor and the active filter live entirely in the query string,
/// and the full check page parses the same `CheckPageQuery` — so carrying it
/// across is what makes an unscripted "Older →" actually page.
#[tokio::test]
async fn the_redirect_carries_the_cursor_and_filter() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let res = server
        .get(&format!("/checks/{cid}/pings?pb=42&pk=fail"))
        .await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(
        res.header("location"),
        format!("/checks/{cid}?pb=42&pk=fail#pings-section")
    );
}

#[tokio::test]
async fn notifications_fragment_redirects_a_real_navigation() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let res = server.get(&format!("/checks/{cid}/notifications")).await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(
        res.header("location"),
        format!("/checks/{cid}#notifs-section")
    );
}

#[tokio::test]
async fn admin_check_fragments_redirect_within_the_admin_prefix() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let res = server.get(&format!("/admin/checks/{cid}/pings")).await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(
        res.header("location"),
        format!("/admin/checks/{cid}#pings-section")
    );

    let res = server
        .get(&format!("/admin/checks/{cid}/notifications"))
        .await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(
        res.header("location"),
        format!("/admin/checks/{cid}#notifs-section")
    );
}

#[tokio::test]
async fn audit_fragment_redirects_a_real_navigation_to_admin() {
    let (server, store, uid) = logged_in_server().await;
    check_for(&store, uid, "cu").await;

    let res = server.get("/admin/audit?aaction=admin.access").await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(
        res.header("location"),
        "/admin?aaction=admin.access#audit-section"
    );
}

/// The redirect is presentation only: `app.js` still gets its partial, which
/// is what keeps the in-place swap a swap.
#[tokio::test]
async fn a_fetch_caller_still_gets_the_bare_fragment() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let res = server
        .get(&format!("/checks/{cid}/pings"))
        .add_header("x-requested-with", "fetch")
        .await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains("data-testid=\"pings-filters\""),
        "fetch caller did not get the fragment: {body}"
    );
    assert!(
        !body.contains("<title>"),
        "fetch caller got a whole page, not a fragment: {body}"
    );
}

/// Ownership is resolved *before* the redirect decision, so the fallback
/// cannot become a cheap way to confirm that someone else's check exists.
#[tokio::test]
async fn the_redirect_never_answers_for_another_users_check() {
    let (server, store, _uid) = logged_in_server().await;
    let other = store
        .create_user("other", Some("x"), false, chrono::Utc::now())
        .await
        .unwrap();
    let (_pid, cid) = check_for(&store, other, "cu2").await;

    server
        .get(&format!("/checks/{cid}/pings"))
        .await
        .assert_status_not_found();
    server
        .get(&format!("/checks/{cid}/notifications"))
        .await
        .assert_status_not_found();
}
