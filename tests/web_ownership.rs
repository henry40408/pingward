//! Web-surface twin of
//! `tests/api_v1.rs::member_cannot_reach_another_users_resource_on_any_api_route`:
//! every parameterised owner-scoped browser route hides another user's
//! project/check/channel from a signed-in non-admin caller behind a `404`
//! (`owned_project`/`owned_check` in `src/web.rs`).

use axum::http::StatusCode;
use axum_test::TestServer;
use chrono::Utc;
use pingward::models::ChannelKind;
use pingward::{app, db, state::AppState, store::Store};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// Uniform per-request timeout for the ownership loop below.
/// `/checks/{id}/events` is a Server-Sent Events route whose body never ends,
/// so `axum_test`'s request helpers (which await the *entire* body) would hang
/// forever on it. Every request goes through this same timeout rather than a
/// per-route carve-out, with opposite pass/fail meanings at the two call sites
/// below.
///
/// Generous (seconds) because its only job is to tell "streams forever" from
/// "completes"; a tight bound risks a false failure on a loaded CI runner.
/// Only the owner's SSE positive control ever waits it out.
const ROUTE_TIMEOUT: Duration = Duration::from_secs(5);

mod common;

/// A fresh, empty, migrated in-memory-SQLite store.
async fn test_store() -> Store {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    Store::new(pool)
}

/// Log a fresh `TestServer` (its own cookie jar) into `store` as `username`,
/// with that session's CSRF token attached as a default `X-CSRF-Token` header
/// so protected POSTs pass `csrf_guard` (duplicated from
/// `tests/admin.rs::set_csrf`; test binaries share code only through
/// `tests/common/`). Without a valid token the rejection would be a `403`, so
/// the token is what proves every `404` below comes from owner scoping. The
/// session is found by `rowid`, since this test logs two users into one store
/// within the same second.
async fn login_server(store: &Store, username: &str, password: &str) -> TestServer {
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", username),
            ("password", password),
        ])
        .await;
    let tok = common::newest_session_csrf(&store.pool).await;
    server.add_header("x-csrf-token", tok.as_str());
    server
}

/// Send one web request with an optional url-encoded form body, shared by the
/// non-owner and owner requests in the loop below.
async fn build_request(
    server: &TestServer,
    method: &str,
    path: &str,
    body: Option<&[(&str, &str)]>,
) -> axum_test::TestResponse {
    let mut req = match method {
        "GET" => server.get(path),
        "POST" => server.post(path),
        other => panic!("unsupported method {other} for route {path}"),
    };
    if let Some(fields) = body {
        req = req.form(fields);
    }
    req.await
}

// --- web-surface cross-user ownership scoping -------------------------------
//
// `owned_project`/`owned_check` in `src/web.rs` are the choke point every
// parameterised owner-scoped browser handler routes an id through: owner-scope
// or `404` (not `403`), so existence is hidden. Excluded here: `/admin*`, which
// resolves ids through `admin_project`/`admin_check`/`admin_channel` because
// admins are *allowed* cross-user access (covered by
// `tests/admin.rs::non_admin_forbidden_on_every_admin_route`), and `/account/*`,
// owner-scoped by a different mechanism (a key belongs to a user; a session is
// found by SHA-256 handle) and covered in `tests/account_web.rs`.
//
// The test below derives every such route, substitutes another user's resource
// id, and asserts the non-owner 404s AND that the owner does not. Without that
// second half a 404 is indistinguishable from "that id never existed at all"
// (broken seeding, an off-by-one id, a future refactor) and the test passes
// vacuously.

/// Every parameterised, non-admin, non-account `web::routes()` route is checked
/// both ways: a non-owner caller ("B") gets `404` (not `403`), and the owner
/// ("A") gets anything *other than* `404` for the same route and id. The route
/// list is derived from the router's own source, so a new owner-scoped route
/// that resolves an id without `owned_project`/`owned_check` fails this test.
#[tokio::test]
async fn member_cannot_reach_another_users_resource_on_any_web_route() {
    let store = test_store().await;
    let phc = pingward::auth::hash_password("pw").unwrap();

    let owner = store
        .create_user("alice", Some(&phc), false, Utc::now())
        .await
        .unwrap();
    let owner_server = login_server(&store, "alice", "pw").await;

    // B is a *non-admin*: an admin is allowed cross-user access, a separate
    // invariant tested in tests/admin.rs.
    store
        .create_user("mallory", Some(&phc), false, Utc::now())
        .await
        .unwrap();
    let member_server = login_server(&store, "mallory", "pw").await;

    // A single empty prefix, not one call per known prefix: a future
    // owner-scoped resource type under a new path prefix is then in scope
    // automatically instead of being silently missed.
    let routes = common::routes_in_router_source(include_str!("../src/web.rs"), "");
    let param_routes: Vec<(&str, String)> = routes
        .into_iter()
        .filter(|(_, raw_path)| {
            // No path parameter ⇒ no cross-user surface to test.
            raw_path.contains('{')
                // `/admin*` is a different invariant (see module doc above).
                && !raw_path.starts_with("/admin")
                // `/account/*` is owner-scoped differently (see above).
                && !raw_path.starts_with("/account")
        })
        .collect();
    // A parser bug or an over-broad filter returning nothing would make the
    // loop below pass vacuously.
    assert!(
        param_routes.len() >= 15,
        "parsed only {} parameterised non-admin, non-account web routes from \
         src/web.rs — the source parser is probably broken, or the filter is \
         too aggressive; this test would otherwise pass vacuously",
        param_routes.len()
    );

    // (method, raw path) -> request form body, verified against each handler's
    // `Form<...>`/`HtmlForm<...>` struct in `src/web.rs`. Those extractors run
    // during parameter binding, *before* the handler calls
    // `owned_project`/`owned_check`, so a route given an incomplete body fails
    // extraction (400/422) and its "B" request would be 400 rather than 404.
    // Every parameterised route must appear here exactly once, body or not —
    // see the exhaustiveness assertion below.
    let project_form: Vec<(&str, &str)> = vec![
        ("name", "x"),
        ("description", ""),
        ("scan_interval_secs", ""),
        ("nag_interval_secs", ""),
    ];
    let check_form: Vec<(&str, &str)> = vec![
        ("name", "x"),
        ("description", ""),
        ("schedule_kind", "period"),
        ("period_secs", "60"),
        ("cron_expr", ""),
        ("grace_secs", "30"),
        ("timezone", "UTC"),
        ("scan_interval_secs", ""),
        ("max_runtime_secs", ""),
        ("nag_interval_secs", ""),
    ];
    let channel_form: Vec<(&str, &str)> = vec![("name", "x"), ("kind", "webhook")];
    // `BindForm.channel_ids` is `#[serde(default)]`, so an empty form is a
    // valid (empty) selection.
    let bind_form: Vec<(&str, &str)> = vec![("_", "")];

    type FormBody<'a> = Option<Vec<(&'a str, &'a str)>>;
    let body_table: HashMap<(&str, &str), FormBody> = HashMap::from([
        (("GET", "/projects/{id}"), None),
        (("POST", "/projects/{id}"), Some(project_form.clone())),
        (("GET", "/projects/{id}/edit"), None),
        (("POST", "/projects/{id}/delete"), None),
        (("GET", "/projects/{pid}/checks/new"), None),
        (("POST", "/projects/{pid}/checks"), Some(check_form.clone())),
        (("GET", "/checks/{id}"), None),
        (("POST", "/checks/{id}"), Some(check_form.clone())),
        (("GET", "/checks/{id}/pings"), None),
        (("GET", "/checks/{id}/events"), None),
        (("GET", "/checks/{id}/notifications"), None),
        (("GET", "/checks/{id}/edit"), None),
        (("POST", "/checks/{id}/pause"), None),
        (("POST", "/checks/{id}/resume"), None),
        (("POST", "/checks/{id}/ack"), None),
        (("POST", "/checks/{id}/regenerate"), None),
        (("POST", "/checks/{id}/delete"), None),
        (("GET", "/projects/{pid}/channels/new"), None),
        (
            ("POST", "/projects/{pid}/channels"),
            Some(channel_form.clone()),
        ),
        (("GET", "/channels/{id}/edit"), None),
        // An edit merges over the stored config: every `ChannelForm` field is
        // `#[serde(default)]` and a blank one keeps its stored value.
        (("POST", "/channels/{id}"), Some(vec![("_", "")])),
        (("POST", "/channels/{id}/delete"), None),
        (("POST", "/channels/{id}/test"), None),
        (("POST", "/checks/{id}/channels"), Some(bind_form.clone())),
    ]);

    // The table's keys must exactly match the derived routes, so a new route
    // missing from the table (or a stale entry for a removed one) fails here
    // rather than silently skipping the invariant.
    let derived_keys: HashSet<(&str, &str)> = param_routes
        .iter()
        .map(|(method, path)| (*method, path.as_str()))
        .collect();
    let table_keys: HashSet<(&str, &str)> = body_table.keys().copied().collect();
    assert_eq!(
        derived_keys, table_keys,
        "body_table's keys don't exactly match the derived parameterised, \
         non-admin, non-account web routes — add or remove an entry so the \
         two match"
    );

    for (i, (method, raw_path)) in param_routes.iter().enumerate() {
        // Seed per iteration, not once before the loop: several routes are
        // destructive, so the owner's positive control below would consume a
        // shared resource and poison later iterations. Names/uuids carry the
        // loop index because `ping_uuid` is UNIQUE.
        let pid = store
            .create_project(
                owner,
                &format!("alice-project-{i}"),
                "",
                None,
                None,
                Utc::now(),
            )
            .await
            .unwrap();
        let cid = store
            .create_check(&pingward::store::NewCheck {
                project_id: pid,
                name: &format!("alice-check-{i}"),
                ping_uuid: &format!("alice-check-uuid-{i}"),
                kind: pingward::models::ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let chid = store
            .create_channel(
                pid,
                ChannelKind::Webhook,
                &format!("alice-channel-{i}"),
                "{}",
                Utc::now(),
            )
            .await
            .unwrap();

        let path = common::substitute_owner_id(raw_path, pid, cid, chid);
        let body = body_table
            .get(&(*method, raw_path.as_str()))
            .unwrap_or_else(|| panic!("no body mapping for {method} {raw_path} — add one"));

        // B's request must run before A's: B always 404s and so never mutates
        // the seeded resource, while A's may be a delete that consumes it —
        // running A first would make B's 404 vacuous again.
        //
        // Both go through `ROUTE_TIMEOUT`, which means opposite things for
        // each: for the non-owner, never resolving is itself a failure; for
        // the owner, a timeout counts as "not 404" and satisfies the positive
        // control, since a body that streams instead of completing (the SSE
        // route) proves the id resolved and the handler was entered.
        let member_res = tokio::time::timeout(
            ROUTE_TIMEOUT,
            build_request(&member_server, method, &path, body.as_deref()),
        )
        .await;
        // 404, not 403: existence is hidden from a non-owner non-admin.
        let Ok(member_res) = member_res else {
            panic!(
                "{method} {raw_path} (requested as {path}): non-owner request did not \
                 resolve within {ROUTE_TIMEOUT:?} — every non-owner request must resolve \
                 promptly to 404, not hang"
            );
        };
        assert_eq!(
            member_res.status_code(),
            StatusCode::NOT_FOUND,
            "{method} {raw_path} (requested as {path}): expected 404 Not Found \
             for a non-owner non-admin caller, got {}",
            member_res.status_code()
        );

        // Positive control: the same request as the owner against the same id,
        // proving the id was live so B's 404 is ownership-driven. Only "not
        // 404" is asserted — several routes redirect (303), and a minimal
        // channel-create body re-renders the form with a validation error
        // (200); either proves the id resolved to a real, owned resource.
        let owner_res = tokio::time::timeout(
            ROUTE_TIMEOUT,
            build_request(&owner_server, method, &path, body.as_deref()),
        )
        .await;
        // A timeout here means the response is still streaming (the SSE body)
        // rather than a completed 404, which already satisfies the positive
        // control, so only the `Ok` case needs an assertion.
        if let Ok(owner_res) = owner_res {
            assert_ne!(
                owner_res.status_code(),
                StatusCode::NOT_FOUND,
                "{method} {raw_path} (requested as {path}): the owner got 404 too, so the \
                 non-owner's 404 proves nothing about ownership scoping — the seeded \
                 resource is not reachable and this test would pass vacuously"
            );
        }
    }
}
