//! Web twin of `tests/api_v1.rs::member_cannot_reach_another_users_resource_on_any_api_route`:
//! every parameterised owner-scoped route 404s for a non-admin non-owner.

use axum::http::StatusCode;
use axum_test::TestServer;
use chrono::Utc;
use pingward::models::ChannelKind;
use pingward::{app, db, state::AppState, store::Store};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// `/checks/{id}/events` streams forever and `axum_test` awaits the whole body,
/// so every request is bounded. Generous, to avoid flakes on a loaded runner;
/// only the owner's SSE request ever waits it out.
const ROUTE_TIMEOUT: Duration = Duration::from_secs(5);

mod common;

async fn test_store() -> Store {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    Store::new(pool)
}

/// A separate cookie jar with a valid CSRF header, so every `404` below comes
/// from owner scoping rather than a CSRF `403`.
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
// Excluded: `/admin*` (admins may cross users; see `tests/admin.rs`) and
// `/account/*` (scoped differently; see `tests/account_web.rs`).

/// Routes are derived from `src/web.rs`, so a new one that skips
/// `owned_project`/`owned_check` fails. The non-owner ("B") must get `404`, and
/// the owner ("A") must not, or B's 404 could mean the id never existed.
#[tokio::test]
async fn member_cannot_reach_another_users_resource_on_any_web_route() {
    let store = test_store().await;
    let phc = pingward::auth::hash_password("pw").unwrap();

    let owner = store
        .create_user("alice", Some(&phc), false, Utc::now())
        .await
        .unwrap();
    let owner_server = login_server(&store, "alice", "pw").await;

    store
        .create_user("mallory", Some(&phc), false, Utc::now())
        .await
        .unwrap();
    let member_server = login_server(&store, "mallory", "pw").await;

    // Empty prefix, so routes under a future new prefix are included too.
    let routes = common::routes_in_router_source(include_str!("../src/web.rs"), "");
    let param_routes: Vec<(&str, String)> = routes
        .into_iter()
        .filter(|(_, raw_path)| {
            raw_path.contains('{')
                && !raw_path.starts_with("/admin")
                && !raw_path.starts_with("/account")
        })
        .collect();
    assert!(
        param_routes.len() >= 15,
        "parsed only {} parameterised non-admin, non-account web routes from \
         src/web.rs — the source parser is probably broken, or the filter is \
         too aggressive; this test would otherwise pass vacuously",
        param_routes.len()
    );

    // Form extractors run before `owned_project`/`owned_check`, so an
    // incomplete body would fail extraction instead of 404ing.
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
    // `BindForm.channel_ids` is `#[serde(default)]`.
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
        // Every `ChannelForm` field is `#[serde(default)]`.
        (("POST", "/channels/{id}"), Some(vec![("_", "")])),
        (("POST", "/channels/{id}/delete"), None),
        (("POST", "/channels/{id}/test"), None),
        (("POST", "/checks/{id}/channels"), Some(bind_form.clone())),
    ]);

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
        // Seeded per iteration: the owner's request may delete it.
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

        // B before A: A's request may delete the resource.
        let member_res = tokio::time::timeout(
            ROUTE_TIMEOUT,
            build_request(&member_server, method, &path, body.as_deref()),
        )
        .await;
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

        // Positive control: only "not 404", since routes variously redirect or
        // re-render a form.
        let owner_res = tokio::time::timeout(
            ROUTE_TIMEOUT,
            build_request(&owner_server, method, &path, body.as_deref()),
        )
        .await;
        // A timeout is the SSE body still streaming, which is also "not 404".
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
