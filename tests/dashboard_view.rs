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

/// Create a project and a single (never-pinged, "new") check inside it.
async fn server_with_project_and_check() -> (TestServer, Store, i64, i64) {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
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
    (server, store, pid, cid)
}

#[tokio::test]
async fn dashboard_renders_tiles_and_badges() {
    let (server, _store, _pid, _cid) = server_with_project_and_check().await;

    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("class=\"tiles\""), "summary tiles missing");
    assert!(body.contains("class=\"badge"), "status badge missing");
}

#[tokio::test]
async fn dashboard_shows_project_group_and_check_row() {
    let (server, _store, pid, cid) = server_with_project_and_check().await;

    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("web"), "project group name missing");
    assert!(body.contains("1 checks"), "group check count missing");
    assert!(
        body.contains(&format!("/projects/{pid}")),
        "manage link missing"
    );
    assert!(
        body.contains(&format!("/checks/{cid}")),
        "check row link missing"
    );
    assert!(body.contains("class=\"badge new\""), "new badge missing");
    assert!(
        body.contains("class=\"status-dot new\""),
        "new status dot missing"
    );
}

#[tokio::test]
async fn dashboard_shows_running_badge_and_count() {
    let (server, store, pid, cid) = server_with_project_and_check().await;
    // Give `cid` an in-flight start (no finish) so `display_status` resolves
    // it to Running.
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
    // A second, untouched check must NOT render as running — proves the
    // running tile/badge aren't rendered unconditionally.
    store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "idle",
            ping_uuid: "idle-uuid",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();

    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains(
            "<div class=\"tile running\"><span class=\"edge\"></span><div class=\"n\">1</div><div class=\"l\">Running</div></div>"
        ),
        "running tile must show a count of 1 (not 0 or 2)"
    );
    assert_eq!(
        body.matches("class=\"badge running\"").count(),
        1,
        "exactly one check row should render the running badge"
    );
}

/// Dashboard descriptions: a project's and a check's `markdown::truncate_plain`
/// output must actually reach the rendered page, inside the `gdesc`/`cdesc`
/// elements respectively, with markdown markers stripped (not raw) and the
/// check's long description genuinely truncated (not the full string).
#[tokio::test]
async fn dashboard_shows_truncated_descriptions_with_markdown_stripped() {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(
            uid,
            "web",
            "**Web** services project",
            None,
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    let long_desc = "**Nightly** backups of the primary database run every day and verify \
        checksum integrity end to end, catching silent corruption early before it can spread \
        further into downstream systems and backups.";
    assert!(
        pingward::markdown::to_plain(long_desc).chars().count() > 120,
        "test fixture must actually be long enough to exercise truncation"
    );
    store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            description: long_desc,
            ping_uuid: "cu-long-desc",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    // Negative control: a second check with an EMPTY description must not
    // render a `cdesc` element at all — proves the
    // `{% if !c.description.is_empty() %}` guard works, and that the
    // assertions below aren't matching something incidental.
    store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "idle",
            ping_uuid: "cu-empty-desc",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();

    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();

    // Project description: markers stripped, rendered inside `gdesc`.
    assert!(
        body.contains("class=\"gdesc\">Web services project</span>"),
        "project description missing/not stripped in gdesc: {body}"
    );
    assert!(
        !body.contains("**Web**"),
        "raw markdown markers leaked into gdesc, truncate_plain did not run: {body}"
    );

    // Check description: truncated (ellipsis present, tail of the original
    // string absent) and markers stripped, rendered inside `cdesc`.
    // 120 characters of content plus the ellipsis. Asserted against a literal
    // rather than a `truncate_plain` call, so a broken truncation cannot make
    // this test agree with itself.
    let expected = "Nightly backups of the primary database run every day and verify checksum integrity end to end, catching silent corrupti…";
    assert!(
        body.contains(&format!(
            "class=\"cdesc\" data-testid=\"check-description-summary\">{expected}</div>"
        )),
        "check description missing/not truncated-and-stripped in cdesc: {body}"
    );
    assert_eq!(expected.chars().count(), 121);
    assert!(
        body.contains('…'),
        "truncated check description must contain an ellipsis: {body}"
    );
    assert!(
        !body.contains("downstream systems and backups"),
        "the tail of the untruncated description leaked into the page: {body}"
    );
    assert!(
        !body.contains("**Nightly**"),
        "raw markdown markers leaked into cdesc, truncate_plain did not run: {body}"
    );

    // Negative control: exactly one check row (the one with a non-empty
    // description) should render a `cdesc` element.
    assert_eq!(
        body.matches("data-testid=\"check-description-summary\"")
            .count(),
        1,
        "the empty-description check must not render a cdesc element"
    );
}

#[tokio::test]
async fn dashboard_empty_state_when_no_projects() {
    let (server, _store, _uid) = logged_in_server().await;
    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains("No projects yet"),
        "empty-state message missing"
    );
}
