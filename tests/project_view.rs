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
        .create_project(uid, "web", "", None, None, chrono::Utc::now())
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

/// XSS regression: a project description carrying `<img onerror=...>` and a
/// `javascript:` link must never reach the rendered page as live markup —
/// `markdown::render`'s escape-first design must have turned the `<` into
/// `&lt;` before any markdown transform ran, and the `javascript:` scheme must
/// never have become an `<a href>`.
#[tokio::test]
async fn project_description_neutralizes_xss_payloads() {
    let (server, store, pid) = server_with_project().await;
    store
        .update_project(
            pid,
            "web",
            "<img src=x onerror=alert(1)> and [x](javascript:alert(1))",
            None,
            None,
        )
        .await
        .unwrap();

    let res = server.get(&format!("/projects/{pid}")).await;
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
        body.contains("data-testid=\"project-description\""),
        "description block missing: {body}"
    );
}

/// A project description's markdown renders on the project page (`**bold**`
/// becomes `<strong>`), and its truncated plain-text form (no markdown
/// markers, no HTML tags) shows on each check row via `ProjectCheckRow`.
#[tokio::test]
async fn project_and_check_descriptions_render_on_project_page() {
    let (server, store, pid) = server_with_project().await;
    store
        .update_project(
            pid,
            "web",
            "A **bold** project description.\n\n- alpha item\n- beta item",
            None,
            None,
        )
        .await
        .unwrap();
    store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            description: "Runs *nightly* backups.",
            ping_uuid: "cu-desc",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();

    let res = server.get(&format!("/projects/{pid}")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains("<strong>bold</strong>"),
        "project description markdown not rendered: {body}"
    );
    // A `- ` bullet list block in the description must render through the
    // real pipeline (`markdown::render`'s `<ul>/<li>` path), not just be
    // proven by `src/markdown.rs`'s own unit tests.
    assert!(body.contains("<ul>"), "list block missing <ul>: {body}");
    assert!(
        body.contains("<li>alpha item</li>"),
        "list item 'alpha item' missing: {body}"
    );
    assert!(
        body.contains("<li>beta item</li>"),
        "list item 'beta item' missing: {body}"
    );
    // `ProjectCheckRow.description` is `markdown::truncate_plain`: markers
    // stripped (no `*`), no HTML tags emitted.
    assert!(
        body.contains(
            "class=\"cdesc\" data-testid=\"check-description-summary\">Runs nightly backups.</div>"
        ),
        "check row must show the plain-text truncated description: {body}"
    );
    assert!(
        !body.contains("Runs *nightly*"),
        "check row description must have its markdown markers stripped: {body}"
    );
}
