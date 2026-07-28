use axum::http::StatusCode;
use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};

mod common;

/// After a session exists, configure the `TestServer` to send that session's
/// CSRF synchronizer token as a default `X-CSRF-Token` header so protected POSTs
/// are not rejected by `csrf_guard`. Call after every (re)login.
async fn set_csrf(server: &mut TestServer, store: &Store) {
    let tok = common::newest_session_csrf(&store.pool).await;
    server.add_header("x-csrf-token", tok.as_str());
}

async fn admin_server() -> (TestServer, Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    let admin_id = store
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
    set_csrf(&mut server, &store).await;
    (server, store, admin_id)
}

// --- admin route guard exhaustiveness --------------------------------------
//
// `web::routes()` guards every `/admin*` handler individually via the
// `AdminUser` extractor — there is no router-level layer enforcing it.
// `non_admin_forbidden_on_every_admin_route` below parses `src/web.rs` to
// recover the exact list of `/admin*` (method, path) pairs the router
// registers — `axum::Router` does not expose its route table at runtime, so
// source-parsing is the only way to derive it — and asserts every single one
// returns 403 for a signed-in non-admin. There is no per-route exception
// list: a new `/admin` route that forgets its `AdminUser` guard fails this
// test, and the only way to make it pass again is to add the guard.

/// Every `/admin*` route registered by `web::routes()` must 403 for a
/// signed-in non-admin, with no exceptions. The route list is derived from
/// the router's own source (`common::routes_in_router_source`) rather than
/// hand-maintained, so a newly added `/admin` route that forgets its
/// `AdminUser` guard fails this test and there is no way to silence it
/// short of actually adding the guard.
#[tokio::test]
async fn non_admin_forbidden_on_every_admin_route() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    store
        .create_user("member", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "member"),
            ("password", "pw"),
        ])
        .await;
    // A valid session + CSRF token proves every 403 below comes from the
    // `AdminUser` guard, not a missing/invalid CSRF token.
    set_csrf(&mut server, &store).await;

    let routes = common::routes_in_router_source(include_str!("../src/web.rs"), "/admin");
    // A parser that (due to a bug) returns nothing would make the loop below
    // pass vacuously. Guard against that explicitly.
    assert!(
        routes.len() >= 25,
        "parsed only {} /admin routes from web.rs — the source parser is \
         probably broken; this test would otherwise pass vacuously",
        routes.len()
    );

    for (method, raw_path) in &routes {
        let path = common::normalise_route_path(raw_path);
        let status = match *method {
            "GET" => server.get(&path).await.status_code(),
            // `AdminUser` is extracted before `Form`/`HtmlForm` in every
            // handler, so the guard rejects before the body is parsed — an
            // empty form is fine here.
            "POST" => server.post(&path).form(&[("_", "")]).await.status_code(),
            other => panic!("unsupported method {other} for route {path}"),
        };
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {path}: expected 403 Forbidden, got {status}"
        );
    }
}

#[tokio::test]
async fn admin_sees_admin_nav_link_on_dashboard() {
    let (server, store, _admin_id) = admin_server().await;
    let body = server.get("/").await.text();
    assert!(
        body.contains(r#"href="/admin""#),
        "admin's own dashboard should show the Admin nav link"
    );

    // A separate, non-admin member must NOT see the Admin nav link on their
    // own dashboard, proving the link reflects the viewer, not the route.
    let state = AppState::new(store.clone(), common::test_config());
    let mut member_server = TestServer::new(app(state));
    member_server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    store
        .create_user("member", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    member_server
        .post("/login")
        .form(&[("username", "member"), ("password", "pw")])
        .await;
    let member_body = member_server.get("/").await.text();
    assert!(
        !member_body.contains(r#"href="/admin""#),
        "non-admin member should not see the Admin nav link"
    );
}

#[tokio::test]
async fn admin_views_other_users_project_and_audits() {
    let (server, store, _admin_id) = admin_server().await;
    // A separate user owns a project + check.
    let owner = store
        .create_user("owner", Some("phc"), false, chrono::Utc::now())
        .await
        .unwrap();
    let pid = store
        .create_project(owner, "victim", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    // Admin can see it via /admin, owner-scoped route would 404.
    server.get("/projects").await; // (owner route is per-user; admin uses /admin)
    server
        .get(&format!("/admin/projects/{pid}"))
        .await
        .assert_status_ok();
    let audit = store.list_audit(10).await.unwrap();
    assert!(audit.iter().any(|a| a.action == "admin.access"
        && a.target_type.as_deref() == Some("project")
        && a.target_id == Some(pid)
        && a.target_owner_id == Some(owner)));
}

/// Deleting another user's project sends the admin back to `/admin`. The
/// `location` assertion is the regression guard: it used to point at
/// `/admin/projects`, a route that no longer exists and would now 404.
#[tokio::test]
async fn admin_deletes_other_users_project_and_lands_on_admin() {
    let (server, store, _admin_id) = admin_server().await;
    // A separate user owns a project.
    let owner = store
        .create_user("owner", Some("phc"), false, chrono::Utc::now())
        .await
        .unwrap();
    let pid = store
        .create_project(owner, "victim", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    // Admin deletes the project and should land on /admin, not /admin/projects.
    let res = server.post(&format!("/admin/projects/{pid}/delete")).await;
    assert_eq!(res.status_code(), StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/admin");
    // Verify the project is actually deleted.
    let projects = store.list_projects_for_user(owner).await.unwrap();
    assert!(!projects.iter().any(|p| p.id == pid));
}

#[tokio::test]
async fn admin_mutation_on_other_project_is_audited() {
    let (server, store, _admin_id) = admin_server().await;
    let owner = store
        .create_user("owner2", Some("phc"), false, chrono::Utc::now())
        .await
        .unwrap();
    let pid = store
        .create_project(owner, "p", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let cid = store
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
    server
        .post(&format!("/admin/checks/{cid}/pause"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    // Check is paused and the access was audited.
    assert_eq!(
        store.find_check(cid).await.unwrap().unwrap().status,
        pingward::models::CheckStatus::Paused
    );
    let audit = store.list_audit(50).await.unwrap();
    assert!(
        audit
            .iter()
            .any(|a| a.target_type.as_deref() == Some("check")
                && a.target_id == Some(cid)
                && a.method.as_deref() == Some("POST"))
    );
}

#[tokio::test]
async fn admin_keeps_nav_link_on_owner_form_validation_error() {
    let (server, store, admin_id) = admin_server().await;
    let pid = store
        .create_project(admin_id, "p", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    // Invalid: blank name is allowed, but blank period_secs with schedule_kind
    // "period" fails `validate_check`, triggering the error re-render branch.
    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("name", "c"),
            ("description", ""),
            ("schedule_kind", "period"),
            ("period_secs", ""),
            ("cron_expr", ""),
            ("grace_secs", "30"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    // Error re-render is 200 with the form; it must still show the Admin nav
    // link since the viewer is an admin (even though this is the owner route).
    res.assert_status_ok();
    assert!(res.text().contains("href=\"/admin\""));
}

// --- audit trail on /admin -------------------------------------------------

/// Record `n` audit rows directly, one second apart, alternating action so the
/// filter has something to narrow. Returns the seeded rows' actions in order.
async fn seed_audit(store: &Store, actor_id: i64, n: i64) {
    let base = chrono::Utc::now() - chrono::Duration::hours(1);
    for i in 0..n {
        store
            .record_audit(
                &pingward::store::NewAudit {
                    actor_user_id: actor_id,
                    actor_username: "admin",
                    action: if i % 2 == 0 {
                        "admin.access"
                    } else {
                        "user.create"
                    },
                    target_type: Some("project"),
                    target_id: Some(i),
                    target_owner_id: Some(9),
                    method: Some("GET"),
                    path: Some("/admin/projects/1"),
                    detail: Some(format!("row {i}")).as_deref(),
                },
                base + chrono::Duration::seconds(i),
            )
            .await
            .unwrap();
    }
}

/// The trail is readable from `/admin` itself, and every column of
/// `models::AuditLog` reaches the page — including the `method`/`path`/
/// `detail`/`target_owner_id` carried in the expandable row.
#[tokio::test]
async fn admin_page_shows_the_audit_trail() {
    let (server, store, admin_id) = admin_server().await;
    seed_audit(&store, admin_id, 3).await;

    let res = server.get("/admin").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains("Audit trail"),
        "audit card heading missing: {body}"
    );
    assert!(
        body.contains("data-testid=\"audit-row\""),
        "no audit rows rendered: {body}"
    );
    assert!(body.contains("admin.access"), "action missing: {body}");
    assert!(
        body.contains("project #1"),
        "target type/id column missing: {body}"
    );
    // The rest of AuditLog, in the expandable row.
    assert!(
        body.contains("GET /admin/projects/1"),
        "method/path missing: {body}"
    );
    assert!(body.contains("row 1"), "detail missing: {body}");
    assert!(body.contains("user #9"), "target_owner_id missing: {body}");
}

/// An admin with an empty trail gets the empty state, not a broken table.
#[tokio::test]
async fn admin_audit_empty_state() {
    let (server, _store, _admin_id) = admin_server().await;
    let res = server.get("/admin").await;
    res.assert_status_ok();
    assert!(
        res.text().contains("No audit entries yet."),
        "empty state missing: {}",
        res.text()
    );
}

/// The fragment endpoint serves the same table on its own, and honours the
/// action filter. The card body and the fragment are one template, so this
/// also pins what `/admin` inlines.
#[tokio::test]
async fn admin_audit_fragment_filters_by_action() {
    let (server, store, admin_id) = admin_server().await;
    seed_audit(&store, admin_id, 6).await;

    let res = server.get("/admin/audit?aaction=user.create").await;
    res.assert_status_ok();
    let body = res.text();
    assert_eq!(
        body.matches("data-testid=\"audit-row\"").count(),
        3,
        "expected the 3 user.create rows: {body}"
    );
    // Only the rows are filtered — the Action select still has to offer every
    // action present in the trail, so assert on the cell, not the page.
    assert!(
        !body.contains("<td class=\"mono\">admin.access</td>"),
        "filtered-out action still present as a row: {body}"
    );
    // A filter in force offers a way out of it.
    assert!(
        body.contains("data-testid=\"audit-clear\""),
        "Clear link missing while filtered: {body}"
    );
}

/// A filter that matches nothing says so rather than reading as "no audit
/// entries exist".
#[tokio::test]
async fn admin_audit_fragment_filtered_empty_state_differs() {
    let (server, store, admin_id) = admin_server().await;
    seed_audit(&store, admin_id, 2).await;

    let res = server.get("/admin/audit?aactor=nobody").await;
    res.assert_status_ok();
    assert!(
        res.text().contains("No audit entries match the filter."),
        "filtered empty state missing: {}",
        res.text()
    );
}

/// Keyset paging over the fragment: the Older link carries a `ab=` cursor plus
/// the active filter, and following it yields strictly older rows.
#[tokio::test]
async fn admin_audit_pages_and_carries_the_filter() {
    let (server, store, admin_id) = admin_server().await;
    // 20 rows per page, so 24 rows means a second page exists.
    seed_audit(&store, admin_id, 24).await;

    let res = server.get("/admin/audit?aaction=admin.access").await;
    res.assert_status_ok();
    let body = res.text();
    // 12 of the 24 rows are admin.access — one page's worth, no Older link.
    assert_eq!(body.matches("data-testid=\"audit-row\"").count(), 12);

    // Unfiltered, 24 rows page at 20.
    let res = server.get("/admin/audit").await;
    let body = res.text();
    assert_eq!(body.matches("data-testid=\"audit-row\"").count(), 20);
    let older = body
        .split("data-testid=\"audit-older\"")
        .next()
        .and_then(|s| s.rfind("href=\"").map(|i| (s, i)))
        .map(|(s, i)| {
            let rest = &s[i + 6..];
            rest[..rest.find('"').unwrap()].to_string()
        })
        .expect("an Older link with 24 rows");
    assert!(older.starts_with("/admin/audit?ab="), "unexpected: {older}");

    let res = server.get(&older).await;
    res.assert_status_ok();
    assert_eq!(res.text().matches("data-testid=\"audit-row\"").count(), 4);
}

/// Paging preserves an active filter rather than silently widening it.
#[tokio::test]
async fn admin_audit_pager_href_carries_the_active_filter() {
    let (server, store, admin_id) = admin_server().await;
    // 48 rows: 24 of each action, so a filtered view still has two pages.
    seed_audit(&store, admin_id, 48).await;

    let res = server.get("/admin/audit?aaction=admin.access").await;
    let body = res.text();
    assert!(
        body.contains("aaction=admin.access") && body.contains("/admin/audit?ab="),
        "pager href dropped the filter: {body}"
    );
}
