//! Web-surface twin of
//! `tests/api_v1.rs::member_cannot_reach_another_users_resource_on_any_api_route`:
//! exhaustively proves `owned_project`/`owned_check` (`src/web.rs`) hide
//! another user's project/check/channel from a signed-in non-admin caller
//! behind a `404`, across every parameterised owner-scoped browser route.

use axum::http::StatusCode;
use axum_test::TestServer;
use chrono::Utc;
use pingward::models::ChannelKind;
use pingward::{app, config::Config, db, state::AppState, store::Store};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// Uniform per-request timeout for the ownership loop below. `/checks/{id}/events`
/// is a Server-Sent Events route whose body never ends, so `axum_test`'s
/// request helpers (which await the *entire* body) would hang forever on it.
/// Rather than special-casing that one route, EVERY request in this test —
/// non-owner and owner alike — goes through this same timeout, with opposite
/// pass/fail meanings (see the two call sites below). This keeps the test's
/// hard "no exception lists" convention (PRs #77-#79): one rule, applied
/// uniformly, instead of a per-route carve-out.
///
/// Deliberately generous (seconds, not milliseconds): this value's only job
/// is to distinguish "streams forever" (the SSE route) from "completes" —
/// there is nothing to gain from cutting it close, and a tight bound just
/// risks a false failure on a loaded CI runner. Only one request per test
/// run — the owner's SSE positive control — actually waits the full
/// duration; every other request still returns almost immediately.
const ROUTE_TIMEOUT: Duration = Duration::from_secs(5);

mod common;

/// A fresh, empty, migrated in-memory-SQLite store.
async fn test_store() -> Store {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    Store::new(pool)
}

/// Log a fresh `TestServer` (its own cookie jar) into `store` as `username`,
/// with its own session's CSRF synchronizer token attached as a default
/// `X-CSRF-Token` header so protected POSTs pass `csrf_guard`
/// (`tests/admin.rs::set_csrf` does the same thing; duplicated here because
/// integration-test binaries only share code through `tests/common/`). A
/// missing/invalid CSRF token would also come back as a non-2xx status
/// (`403`, distinguishable from `404` but still a rejection the caller didn't
/// intend to test), so attaching a *valid* token is what proves every `404`
/// asserted below comes from owner scoping and not from an incidental CSRF
/// failure. Ordered by `rowid` (not `created_at`) so the just-inserted
/// session is unambiguous even when another user's session already exists in
/// the same store — this test logs in two different users against one shared
/// store.
async fn login_server(store: &Store, username: &str, password: &str) -> TestServer {
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    server
        .post("/login")
        .form(&[("username", username), ("password", password)])
        .await;
    let tok = sqlx::query_scalar::<_, String>(
        "SELECT csrf_token FROM sessions ORDER BY rowid DESC LIMIT 1",
    )
    .fetch_one(&store.pool)
    .await
    .unwrap();
    server.add_header("x-csrf-token", tok.as_str());
    server
}

/// Send one web request with an optional url-encoded form body. Factored out
/// so the (method, non-owner) and (method, owner) requests in the loop below
/// share one code path instead of duplicating the method dispatch `match`.
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
// `owned_project`/`owned_check` in `src/web.rs` are the single choke point
// every parameterised owner-scoped browser handler routes an id through:
// owner-scope or `404` (not `403`) — existence is hidden from a caller who
// doesn't own the resource. `/admin*` routes reuse the same owner templates
// but resolve ids through `admin_project`/`admin_check`/`admin_channel`
// instead (no owner filter, audited) — admins are *allowed* cross-user
// access, a different invariant covered by
// `tests/admin.rs::non_admin_forbidden_on_every_admin_route`, so `/admin*` is
// excluded here. `/account/*` routes are owner-scoped by yet another
// mechanism (an API key belongs directly to a user; a session is found by a
// SHA-256 handle via a linear scan of the caller's own sessions, not an id
// lookup at all) and are covered separately in `tests/account_web.rs`
// (`unknown_or_foreign_handle_revokes_nothing`, `keys_are_caller_scoped`), so
// they're excluded here too.
//
// The test below derives every non-admin, non-account `web::routes()` route
// that carries a path parameter, substitutes another user's resource id into
// it, and asserts every single one 404s for a non-admin non-owner caller —
// AND that the owner, hitting the exact same route against the exact same
// id, gets anything other than 404. That second half is what stops the test
// from passing vacuously: without it, a 404 from the non-owner is
// indistinguishable from "that id never existed at all" (broken seeding, an
// off-by-one id, a future refactor), and every route would still show green
// even though the test would no longer be exercising ownership scoping.

/// Every parameterised, non-admin, non-account `web::routes()` route is
/// checked BOTH ways: a signed-in non-admin non-owner caller ("B") must get
/// `404 Not Found` (not `403`), and the owner ("A"), hitting the exact same
/// route against the exact same resource id, must get anything *other than*
/// `404`. The route list is derived from the router's own source
/// (`common::routes_in_router_source`) rather than hand-maintained, so a
/// newly added owner-scoped route that resolves an id without going through
/// `owned_project`/`owned_check` fails this test.
#[tokio::test]
async fn member_cannot_reach_another_users_resource_on_any_web_route() {
    let store = test_store().await;
    let phc = pingward::auth::hash_password("pw").unwrap();

    // User A: the owner whose resources B will try (and fail) to reach.
    let owner = store
        .create_user("alice", Some(&phc), false, Utc::now())
        .await
        .unwrap();
    let owner_server = login_server(&store, "alice", "pw").await;

    // User B: a different NON-admin caller. An admin is *allowed* cross-user
    // access (and it's audited) — that's a separate invariant, tested
    // exhaustively in tests/admin.rs, not this one.
    store
        .create_user("mallory", Some(&phc), false, Utc::now())
        .await
        .unwrap();
    let member_server = login_server(&store, "mallory", "pw").await;

    // Derived with a single empty prefix rather than three separate prefix
    // calls (one each for `/projects`, `/checks`, `/channels`): a future
    // owner-scoped resource type introduced under a brand-new path prefix is
    // then automatically in scope for this test instead of being silently
    // missed because nobody remembered to add a fourth prefix call.
    let routes = common::routes_in_router_source(include_str!("../src/web.rs"), "");
    let param_routes: Vec<(&str, String)> = routes
        .into_iter()
        .filter(|(_, raw_path)| {
            // No path parameter ⇒ no cross-user surface to test (e.g.
            // `POST /projects`, `GET /projects/new`).
            raw_path.contains('{')
                // `/admin*` is a different invariant (see module doc above).
                && !raw_path.starts_with("/admin")
                // `/account/*` is owner-scoped by a different mechanism
                // entirely (see module doc above).
                && !raw_path.starts_with("/account")
        })
        .collect();
    // A parser bug (or an accidental over-broad filter) that returns nothing
    // would make the loop below pass vacuously. Guard against that
    // explicitly.
    assert!(
        param_routes.len() >= 15,
        "parsed only {} parameterised non-admin, non-account web routes from \
         src/web.rs — the source parser is probably broken, or the filter is \
         too aggressive; this test would otherwise pass vacuously",
        param_routes.len()
    );

    // (method, raw path) -> request form body, verified against each
    // handler's actual `Form<...>`/`HtmlForm<...>` struct in `src/web.rs`.
    // `Form`/`HtmlForm` are `FromRequest` extractors that run as part of
    // parameter binding, i.e. *before* the handler body ever calls
    // `owned_project`/`owned_check` — a route that needs a body and gets an
    // incomplete one (missing a required, non-`#[serde(default)]` field)
    // fails extraction (400/422) before ownership is ever checked, which
    // would make that route's "B" request 400 instead of 404 and break the
    // assertion below for a reason that has nothing to do with ownership.
    // Every parameterised route must appear here exactly once, whether or
    // not it takes a body — see the exhaustiveness assertion below.
    let project_form: Vec<(&str, &str)> = vec![
        ("name", "x"),
        ("scan_interval_secs", ""),
        ("nag_interval_secs", ""),
    ];
    let check_form: Vec<(&str, &str)> = vec![
        ("name", "x"),
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
    // valid (empty) selection — mirrors `create_channel_and_bind_to_check`'s
    // unbind step in tests/auth_web.rs.
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
        (("POST", "/channels/{id}/delete"), None),
        (("POST", "/channels/{id}/test"), None),
        (("POST", "/checks/{id}/channels"), Some(bind_form.clone())),
    ]);

    // The table's keys must exactly match the derived routes — a new
    // parameterised, non-admin, non-account route missing from the table (or
    // a stale entry for a removed one) fails here rather than silently
    // skipping the invariant.
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
        // Seed a fresh project + check + channel for the owner on *every*
        // iteration rather than once before the loop. Several routes are
        // destructive (`POST /projects/{id}/delete`, `.../checks/{id}/delete`,
        // `.../channels/{id}/delete`); the owner's positive-control request
        // below would consume/delete a shared resource and poison later
        // iterations, so each iteration gets its own. Names/uuids are
        // suffixed with the loop index — `ping_uuid` has a UNIQUE constraint,
        // so it in particular must not repeat.
        let pid = store
            .create_project(owner, &format!("alice-project-{i}"), None, None, Utc::now())
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

        // B's request MUST run before A's. B's request always 404s (that's
        // the assertion), so it never mutates the seeded resource; A's
        // request may be a delete that consumes it. Running A first would let
        // a destructive owner request remove the row before B ever asks,
        // which would make B's 404 pass vacuously again — exactly what this
        // whole ordering exists to rule out.
        //
        // Both requests below go through the same `ROUTE_TIMEOUT`, but a
        // timeout means opposite things for each: for the non-owner, a
        // request that never resolves is itself a failure (every non-owner
        // request must resolve quickly, to 404 — a route whose body streams
        // forever before ownership is even checked is a bug, not a pass);
        // for the owner, a timeout counts as "not 404" and satisfies the
        // positive control — a response that streams instead of completing
        // (e.g. `/checks/{id}/events`'s SSE body, which never ends) proves
        // the id resolved and the handler was entered.
        let member_res = tokio::time::timeout(
            ROUTE_TIMEOUT,
            build_request(&member_server, method, &path, body.as_deref()),
        )
        .await;
        // 404, not 403: `owned_project`/`owned_check` hide existence from a
        // caller who isn't the owner and isn't an admin, rather than
        // revealing "it exists but you can't touch it".
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

        // Positive control: the SAME request, as the owner, against the SAME
        // resource id. This proves the id was live and reachable, so B's 404
        // above is genuinely ownership-driven rather than "that id doesn't
        // exist at all". We assert merely "not 404", not an exact success
        // status: several routes legitimately redirect (303) while others
        // (e.g. a minimal `POST /projects/{pid}/channels` body that fails
        // `validate_channel`'s per-kind required-field check) legitimately
        // re-render the form with a validation error (200). Either still
        // proves the id resolved to a real, owned resource, which is the
        // only thing this control needs to establish.
        let owner_res = tokio::time::timeout(
            ROUTE_TIMEOUT,
            build_request(&owner_server, method, &path, body.as_deref()),
        )
        .await;
        // A timeout here means the response is still streaming (e.g. the
        // `/checks/{id}/events` SSE body, which never ends) rather than a
        // completed 404 — that alone satisfies the positive control, so only
        // the `Ok` case needs an assertion.
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
