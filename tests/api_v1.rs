//! Integration tests for the `/api/v1` bearer API.

use axum::http::StatusCode;
use axum_test::TestServer;
use chrono::Utc;
use pingward::models::{ChannelKind, ScheduleKind};
use pingward::notify::EventKind;
use pingward::store::NewCheck;
use pingward::{apikey, app, db, state::AppState, store::Store};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

mod common;

async fn test_app() -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    (TestServer::new(app(state)), store)
}

/// Create a user and mint an API key for them; returns `(user_id, bearer_token)`.
async fn user_with_key(store: &Store, username: &str, is_admin: bool) -> (i64, String) {
    let now = Utc::now();
    let uid = store
        .create_user(username, Some("x"), is_admin, now)
        .await
        .unwrap();
    let (full, prefix, hash) = apikey::generate_api_key();
    store
        .insert_api_key(uid, "k", &hash, &prefix, None, now)
        .await
        .unwrap();
    (uid, full)
}

/// A period check under `project_id`, returning its id.
async fn make_check(store: &Store, project_id: i64, name: &str, uuid: &str) -> i64 {
    store
        .create_check(&NewCheck {
            project_id,
            name,
            ping_uuid: uuid,
            kind: ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 30,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn missing_bearer_is_401_json() {
    let (server, _store) = test_app().await;
    let res = server.get("/api/v1/projects").await;
    res.assert_status(StatusCode::UNAUTHORIZED);
    assert_eq!(res.json::<Value>()["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn bogus_bearer_is_401() {
    let (server, _store) = test_app().await;
    let res = server
        .get("/api/v1/projects")
        .add_header("authorization", "Bearer pw_not_a_real_key")
        .await;
    res.assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn lists_only_callers_own_projects() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let now = Utc::now();
    store
        .create_project(uid, "mine", "", None, None, now)
        .await
        .unwrap();
    let (other, _) = user_with_key(&store, "bob", false).await;
    store
        .create_project(other, "theirs", "", None, None, now)
        .await
        .unwrap();

    let res = server
        .get("/api/v1/projects")
        .add_header("authorization", format!("Bearer {token}"))
        .await;
    res.assert_status_ok();
    let arr = res.json::<Vec<Value>>();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "mine");
    assert_eq!(arr[0]["owner_id"], uid);
}

#[tokio::test]
async fn member_cannot_read_another_users_project() {
    let (server, store) = test_app().await;
    let (_uid, token) = user_with_key(&store, "alice", false).await;
    let (other, _) = user_with_key(&store, "bob", false).await;
    let pid = store
        .create_project(other, "theirs", "", None, None, Utc::now())
        .await
        .unwrap();

    // 404 (existence hidden), not 403.
    server
        .get(&format!("/api/v1/projects/{pid}"))
        .add_header("authorization", format!("Bearer {token}"))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_key_reads_cross_user_and_writes_audit() {
    let (server, store) = test_app().await;
    let (_admin, token) = user_with_key(&store, "root", true).await;
    let (other, _) = user_with_key(&store, "bob", false).await;
    let pid = store
        .create_project(other, "theirs", "", None, None, Utc::now())
        .await
        .unwrap();

    let res = server
        .get(&format!("/api/v1/projects/{pid}"))
        .add_header("authorization", format!("Bearer {token}"))
        .await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["name"], "theirs");

    let audits = store.list_audit(10).await.unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action, "admin.api.access");
    assert_eq!(audits[0].target_type.as_deref(), Some("project"));
    assert_eq!(audits[0].target_id, Some(pid));
    assert_eq!(audits[0].target_owner_id, Some(other));
}

#[tokio::test]
async fn admin_reading_own_project_does_not_audit() {
    let (server, store) = test_app().await;
    let (admin, token) = user_with_key(&store, "root", true).await;
    let pid = store
        .create_project(admin, "mine", "", None, None, Utc::now())
        .await
        .unwrap();

    server
        .get(&format!("/api/v1/projects/{pid}"))
        .add_header("authorization", format!("Bearer {token}"))
        .await
        .assert_status_ok();
    assert!(store.list_audit(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn check_pings_are_paginated_newest_first() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    let cid = make_check(&store, pid, "job", "uuid-pg").await;
    for _ in 0..25 {
        store
            .insert_ping(
                cid,
                pingward::models::PingKind::Success,
                None,
                "",
                None,
                Utc::now(),
            )
            .await
            .unwrap();
    }

    // First page: default 20, more older exist.
    let res = server
        .get(&format!("/api/v1/checks/{cid}/pings"))
        .add_header("authorization", format!("Bearer {token}"))
        .await;
    res.assert_status_ok();
    let page = res.json::<Value>();
    assert_eq!(page["items"].as_array().unwrap().len(), 20);
    assert_eq!(page["has_older"], true);
    assert_eq!(page["has_newer"], false);
    // `next_before` points at the last (oldest) item on the page; there is no
    // newer page, so `next_after` is null.
    let last_id = page["items"].as_array().unwrap()[19]["id"]
        .as_i64()
        .unwrap();
    assert_eq!(page["next_before"], last_id);
    assert!(page["next_after"].is_null());

    // Follow next_before to fetch the older page → remaining 5.
    let res2 = server
        .get(&format!("/api/v1/checks/{cid}/pings?before={last_id}"))
        .add_header("authorization", format!("Bearer {token}"))
        .await;
    let page2 = res2.json::<Value>();
    assert_eq!(page2["items"].as_array().unwrap().len(), 5);
    assert_eq!(page2["has_newer"], true);
    assert_eq!(page2["has_older"], false);
    // Now the newer direction is populated and the older one is exhausted.
    assert_eq!(
        page2["next_after"],
        page2["items"].as_array().unwrap()[0]["id"]
            .as_i64()
            .unwrap()
    );
    assert!(page2["next_before"].is_null());
}

#[tokio::test]
async fn limit_is_clamped() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    let cid = make_check(&store, pid, "job", "uuid-lim").await;
    for _ in 0..3 {
        store
            .insert_ping(
                cid,
                pingward::models::PingKind::Success,
                None,
                "",
                None,
                Utc::now(),
            )
            .await
            .unwrap();
    }
    // limit=0 clamps up to 1.
    let res = server
        .get(&format!("/api/v1/checks/{cid}/pings?limit=0"))
        .add_header("authorization", format!("Bearer {token}"))
        .await;
    assert_eq!(res.json::<Value>()["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn channel_dto_never_leaks_config_secrets() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    let secret = "https://hooks.example.com/SECRET-TOKEN";
    let cid = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook",
            &format!("{{\"url\":\"{secret}\"}}"),
            Utc::now(),
        )
        .await
        .unwrap();

    let res = server
        .get(&format!("/api/v1/channels/{cid}"))
        .add_header("authorization", format!("Bearer {token}"))
        .await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        !body.contains(secret),
        "config secret must never be serialized"
    );
    assert!(!body.contains("config_json"));
    assert_eq!(res.json::<Value>()["kind"], "webhook");
}

#[tokio::test]
async fn notifications_endpoint_returns_events() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    let cid = make_check(&store, pid, "job", "uuid-nt").await;
    let chid = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook",
            "{}",
            Utc::now(),
        )
        .await
        .unwrap();
    store
        .record_notification(
            cid,
            chid,
            EventKind::Down,
            pingward::models::NotifyStatus::Ok,
            None,
            Utc::now(),
        )
        .await
        .unwrap();

    let res = server
        .get(&format!("/api/v1/checks/{cid}/notifications"))
        .add_header("authorization", format!("Bearer {token}"))
        .await;
    res.assert_status_ok();
    let items = res.json::<Value>()["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["event"], "down");
    assert_eq!(items[0]["status"], "ok");
}

#[tokio::test]
async fn disabled_users_key_is_rejected() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    store.set_user_disabled(uid, true).await.unwrap();
    server
        .get("/api/v1/projects")
        .add_header("authorization", format!("Bearer {token}"))
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn expired_key_is_rejected() {
    let (server, store) = test_app().await;
    let now = Utc::now();
    let uid = store
        .create_user("alice", Some("x"), false, now)
        .await
        .unwrap();
    let (full, prefix, hash) = apikey::generate_api_key();
    store
        .insert_api_key(
            uid,
            "old",
            &hash,
            &prefix,
            Some(now - chrono::Duration::hours(1)),
            now,
        )
        .await
        .unwrap();
    server
        .get("/api/v1/projects")
        .add_header("authorization", format!("Bearer {full}"))
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn keys_endpoint_is_self_scoped() {
    let (server, store) = test_app().await;
    let (_uid, token) = user_with_key(&store, "alice", false).await;
    let (_bob, _) = user_with_key(&store, "bob", false).await;

    let res = server
        .get("/api/v1/keys")
        .add_header("authorization", format!("Bearer {token}"))
        .await;
    res.assert_status_ok();
    let arr = res.json::<Vec<Value>>();
    // Alice sees only her own single key; no `token_hash` field is ever present.
    assert_eq!(arr.len(), 1);
    assert!(arr[0].get("token_hash").is_none());
    assert!(arr[0]["prefix"].as_str().unwrap().starts_with("pw_"));
}

#[tokio::test]
async fn docs_require_a_logged_in_session() {
    let (server, _store) = test_app().await;
    // Both are gated behind a web session, so an unauthenticated request
    // redirects to /login.
    server
        .get("/api/openapi.json")
        .await
        .assert_status(StatusCode::SEE_OTHER);
    server
        .get("/api/docs")
        .await
        .assert_status(StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn docs_are_served_to_a_logged_in_user() {
    let (mut server, store) = test_app().await;
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    store
        .create_user("member", Some(&phc), false, Utc::now())
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

    let spec = server.get("/api/openapi.json").await;
    spec.assert_status_ok();
    assert_eq!(spec.json::<Value>()["info"]["title"], "pingward API");

    let docs = server.get("/api/docs").await;
    docs.assert_status_ok();
    assert!(docs.text().to_lowercase().contains("scalar"));
}

// --- /api/v1 route guard exhaustiveness ------------------------------------
//
// `api::routes()` guards every `/api/v1` handler individually via the `ApiUser`
// bearer extractor; there is no router-level layer enforcing it.
//
// `/api/openapi.json` and `/api/docs` are excluded: they are gated behind a
// logged-in web session (`CurrentUser`), not a bearer key, and the `/api/v1`
// prefix filter drops them automatically.

/// Every `/api/v1` route registered by `api::routes()` must reject an
/// unauthenticated caller with 401, with no exceptions. The route list is
/// derived from the router's own source rather than hand-maintained
/// (`axum::Router` exposes no route table at runtime), so a new `/api/v1` route
/// that forgets its `ApiUser` extractor fails this test.
#[tokio::test]
async fn every_api_v1_route_requires_a_bearer_key() {
    let (server, _store) = test_app().await;

    let routes = common::routes_in_router_source(include_str!("../src/api/mod.rs"), "/api/v1");
    // A parser bug returning nothing would make the loop below pass vacuously.
    assert!(
        routes.len() >= 20,
        "parsed only {} /api/v1 routes from src/api/mod.rs — the source parser \
         is probably broken; this test would otherwise pass vacuously",
        routes.len()
    );

    for (method, raw_path) in &routes {
        let path = common::normalise_route_path(raw_path);
        // No body is sent, even for POST/PUT/PATCH: `ApiUser` is a
        // `FromRequestParts` extractor and runs *before* the `ApiJson` body
        // extractor, so a handler that took the body first would surface
        // `400 bad_request` here instead of `401`.
        let status = match *method {
            "GET" => server.get(&path).await.status_code(),
            "POST" => server.post(&path).await.status_code(),
            "PUT" => server.put(&path).await.status_code(),
            "PATCH" => server.patch(&path).await.status_code(),
            "DELETE" => server.delete(&path).await.status_code(),
            other => panic!("unsupported method {other} for route {path}"),
        };
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {path}: expected 401 Unauthorized, got {status}"
        );
    }
}

// --- /api/v1 cross-user ownership scoping -----------------------------------
//
// `resolve_project`/`resolve_check`/`resolve_channel` in `src/api/v1.rs` are
// the choke point every parameterised `/api/v1` handler routes an id through:
// owner-scope first, else an audited admin cross-user access, else `404` (not
// `403`), so existence is hidden from a caller who neither owns the resource
// nor is an admin.

/// Every parameterised `/api/v1` route is checked both ways: a non-admin
/// non-owner caller ("B") gets `404` (not `403`), and the owner ("A") gets
/// anything *other than* 404 for the same route and id. Without that second
/// half a 404 from B is indistinguishable from "that id never existed" (broken
/// seeding, an off-by-one id, a future refactor) and the test passes vacuously.
/// The route list is derived from the router's own source, so a new route that
/// resolves an id outside those three choke points fails this test.
#[tokio::test]
async fn member_cannot_reach_another_users_resource_on_any_api_route() {
    let (server, store) = test_app().await;

    let (owner, owner_token) = user_with_key(&store, "alice", false).await;

    // B is a *non-admin*: an admin is allowed cross-user access, a separate
    // invariant.
    let (_member, token) = user_with_key(&store, "mallory", false).await;

    let routes = common::routes_in_router_source(include_str!("../src/api/mod.rs"), "/api/v1");
    // A route with no path parameter has no cross-user surface to test.
    let param_routes: Vec<(&str, String)> = routes
        .into_iter()
        .filter(|(_, raw_path)| raw_path.contains('{'))
        .collect();
    // A parser bug returning nothing would make the loop below pass vacuously.
    assert!(
        param_routes.len() >= 15,
        "parsed only {} parameterised /api/v1 routes from src/api/mod.rs — the \
         source parser is probably broken; this test would otherwise pass \
         vacuously",
        param_routes.len()
    );

    // (method, raw path) -> request body, verified against each DTO's
    // `Deserialize` impl in `src/api/input.rs`. `ApiJson` runs *before* the
    // handler calls `resolve_*`, so an absent or schema-invalid body would 400
    // before the ownership check is reached. Every parameterised route must
    // appear here exactly once, body or not — see the exhaustiveness assertion
    // below.
    let body_table: HashMap<(&str, &str), Option<Value>> = HashMap::from([
        (("GET", "/api/v1/projects/{id}"), None),
        (
            ("PATCH", "/api/v1/projects/{id}"),
            Some(json!({ "name": "x" })),
        ),
        (("DELETE", "/api/v1/projects/{id}"), None),
        (("GET", "/api/v1/projects/{id}/checks"), None),
        (
            ("POST", "/api/v1/projects/{id}/checks"),
            Some(json!({ "name": "x" })),
        ),
        (("GET", "/api/v1/projects/{id}/channels"), None),
        (
            ("POST", "/api/v1/projects/{id}/channels"),
            Some(json!({ "name": "x", "kind": "webhook" })),
        ),
        (("GET", "/api/v1/checks/{id}"), None),
        (
            ("PATCH", "/api/v1/checks/{id}"),
            Some(json!({ "name": "x" })),
        ),
        (("DELETE", "/api/v1/checks/{id}"), None),
        (("GET", "/api/v1/checks/{id}/pings"), None),
        (("GET", "/api/v1/checks/{id}/notifications"), None),
        (("POST", "/api/v1/checks/{id}/pause"), None),
        (("POST", "/api/v1/checks/{id}/resume"), None),
        (("POST", "/api/v1/checks/{id}/ack"), None),
        (("POST", "/api/v1/checks/{id}/regenerate"), None),
        (
            ("PUT", "/api/v1/checks/{id}/channels"),
            Some(json!({ "channel_ids": [] })),
        ),
        (("GET", "/api/v1/channels/{id}"), None),
        // A merge patch, so an empty object is a valid (no-op) body — every
        // field of `ChannelInput` is `#[serde(default)]`.
        (("PATCH", "/api/v1/channels/{id}"), Some(json!({}))),
        (("DELETE", "/api/v1/channels/{id}"), None),
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
        "body_table's keys don't exactly match the derived parameterised /api/v1 \
         routes — add or remove an entry so the two match"
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
        let cid = make_check(
            &store,
            pid,
            &format!("alice-check-{i}"),
            &format!("alice-check-uuid-{i}"),
        )
        .await;
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
        // the seeded resource, while A's may be a DELETE that consumes it —
        // running A first would make B's 404 vacuous again.
        let member_res = build_request(&server, method, &path, &token, body.as_ref()).await;
        // 404, not 403: existence is hidden from a non-owner non-admin.
        assert_eq!(
            member_res.status_code(),
            StatusCode::NOT_FOUND,
            "{method} {raw_path} (requested as {path}): expected 404 Not Found \
             for a non-owner non-admin caller, got {}",
            member_res.status_code()
        );

        // Positive control: the same request as the owner against the same id,
        // proving the id was live so B's 404 is ownership-driven. Only "not
        // 404" is asserted — the minimal bodies in `body_table` satisfy
        // `serde`'s `Deserialize` but not the later `validate_*` calls, so
        // several routes return `400 bad_request` for the owner too; a 400
        // still proves the id resolved to a real, owned resource.
        let owner_res = build_request(&server, method, &path, &owner_token, body.as_ref()).await;
        assert_ne!(
            owner_res.status_code(),
            StatusCode::NOT_FOUND,
            "{method} {raw_path} (requested as {path}): the owner got 404 too, so the \
             non-owner's 404 proves nothing about ownership scoping — the seeded \
             resource is not reachable and this test would pass vacuously"
        );
    }
}

/// Builds and sends one `/api/v1` request with the given bearer token and
/// optional JSON body, shared by the non-owner and owner requests in the loop
/// above.
async fn build_request(
    server: &TestServer,
    method: &str,
    path: &str,
    token: &str,
    body: Option<&Value>,
) -> axum_test::TestResponse {
    let mut req = match method {
        "GET" => server.get(path),
        "POST" => server.post(path),
        "PUT" => server.put(path),
        "PATCH" => server.patch(path),
        "DELETE" => server.delete(path),
        other => panic!("unsupported method {other} for route {path}"),
    }
    .add_header("authorization", format!("Bearer {token}"));
    if let Some(json_body) = body {
        req = req.json(json_body);
    }
    req.await
}
