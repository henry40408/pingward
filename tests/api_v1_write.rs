//! Integration tests for the `/api/v1` write API (Stage C): create/update/
//! delete + the check actions (pause/resume/ack/regenerate) + channel binding,
//! plus admin cross-user write auditing and JSON error envelopes.

use axum::http::StatusCode;
use axum_test::TestServer;
use chrono::Utc;
use pingward::{apikey, app, config::Config, db, state::AppState, store::Store};
use serde_json::{Value, json};

async fn test_app() -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
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

/// Authorization header value for a bearer token.
fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

#[tokio::test]
async fn create_project_appears_in_list() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;

    let res = server
        .post("/api/v1/projects")
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "Backups" }))
        .await;
    res.assert_status(StatusCode::CREATED);
    let body = res.json::<Value>();
    assert_eq!(body["name"], "Backups");
    assert_eq!(body["owner_id"], uid);
    let pid = body["id"].as_i64().unwrap();

    let list = server
        .get("/api/v1/projects")
        .add_header("authorization", bearer(&token))
        .await;
    let arr = list.json::<Vec<Value>>();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], pid);
}

/// POST/PATCH accept `description`, GET returns it **unrendered** (the raw
/// markdown, not HTML — asserting the literal `**bold**` proves the server
/// never runs it through `markdown::render` for API responses), and omitting
/// the field on create yields `""`.
#[tokio::test]
async fn project_description_is_raw_markdown_and_defaults_to_empty() {
    let (server, store) = test_app().await;
    let (_uid, token) = user_with_key(&store, "alice", false).await;

    // Omitted on create → "".
    let created = server
        .post("/api/v1/projects")
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "Backups" }))
        .await;
    created.assert_status(StatusCode::CREATED);
    let body = created.json::<Value>();
    assert_eq!(body["description"], "");
    let pid = body["id"].as_i64().unwrap();

    // PATCH sets it; GET must return the raw markdown, not rendered HTML.
    let patched = server
        .patch(&format!("/api/v1/projects/{pid}"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "Backups", "description": "Nightly **bold** backups." }))
        .await;
    patched.assert_status_ok();
    assert_eq!(
        patched.json::<Value>()["description"],
        "Nightly **bold** backups."
    );

    let fetched = server
        .get(&format!("/api/v1/projects/{pid}"))
        .add_header("authorization", bearer(&token))
        .await;
    let text = fetched.text();
    assert!(
        text.contains("**bold**"),
        "GET must return the raw markdown: {text}"
    );
    assert!(
        !text.contains("<strong>"),
        "GET must NOT render markdown to HTML: {text}"
    );
}

#[tokio::test]
async fn create_project_rejects_blank_name() {
    let (server, store) = test_app().await;
    let (_uid, token) = user_with_key(&store, "alice", false).await;
    let res = server
        .post("/api/v1/projects")
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "   " }))
        .await;
    res.assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(res.json::<Value>()["error"]["code"], "bad_request");
}

#[tokio::test]
async fn duration_field_accepts_both_int_and_string() {
    let (server, store) = test_app().await;
    let (_uid, token) = user_with_key(&store, "alice", false).await;

    // Integer seconds.
    let a = server
        .post("/api/v1/projects")
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "ints", "scan_interval_secs": 90 }))
        .await;
    a.assert_status(StatusCode::CREATED);
    assert_eq!(a.json::<Value>()["scan_interval_secs"], 90);

    // Human-readable string is parsed to seconds.
    let b = server
        .post("/api/v1/projects")
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "strs", "scan_interval_secs": "5m" }))
        .await;
    b.assert_status(StatusCode::CREATED);
    assert_eq!(b.json::<Value>()["scan_interval_secs"], 300);
}

#[tokio::test]
async fn patch_project_replaces_fields() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "old", "", Some(60), None, Utc::now())
        .await
        .unwrap();

    let res = server
        .patch(&format!("/api/v1/projects/{pid}"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "new", "nag_interval_secs": "1h" }))
        .await;
    res.assert_status_ok();
    let body = res.json::<Value>();
    assert_eq!(body["name"], "new");
    assert_eq!(body["nag_interval_secs"], 3600);
    // scan override was omitted → cleared (full replacement, not partial).
    assert!(body["scan_interval_secs"].is_null());
}

#[tokio::test]
async fn delete_project_then_gone() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();

    server
        .delete(&format!("/api/v1/projects/{pid}"))
        .add_header("authorization", bearer(&token))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .get(&format!("/api/v1/projects/{pid}"))
        .add_header("authorization", bearer(&token))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn member_cannot_create_check_in_another_users_project() {
    let (server, store) = test_app().await;
    let (_alice, token) = user_with_key(&store, "alice", false).await;
    let (bob, _) = user_with_key(&store, "bob", false).await;
    let pid = store
        .create_project(bob, "bobs", "", None, None, Utc::now())
        .await
        .unwrap();

    // 404 (existence hidden), and no check is created.
    server
        .post(&format!("/api/v1/projects/{pid}/checks"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "x", "period_secs": 60 }))
        .await
        .assert_status(StatusCode::NOT_FOUND);
    assert!(store.list_checks_for_project(pid).await.unwrap().is_empty());
}

#[tokio::test]
async fn admin_cross_user_write_is_audited_with_the_verb() {
    let (server, store) = test_app().await;
    let (_admin, token) = user_with_key(&store, "root", true).await;
    let (bob, _) = user_with_key(&store, "bob", false).await;
    let pid = store
        .create_project(bob, "bobs", "", None, None, Utc::now())
        .await
        .unwrap();

    let res = server
        .post(&format!("/api/v1/projects/{pid}/checks"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "cross", "period_secs": "1h" }))
        .await;
    res.assert_status(StatusCode::CREATED);

    let audits = store.list_audit(10).await.unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action, "admin.api.access");
    assert_eq!(audits[0].method.as_deref(), Some("POST"));
    assert_eq!(audits[0].target_type.as_deref(), Some("project"));
    assert_eq!(audits[0].target_owner_id, Some(bob));
}

#[tokio::test]
async fn create_check_period_and_reject_bad_schedule() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();

    let ok = server
        .post(&format!("/api/v1/projects/{pid}/checks"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "job", "period_secs": "1h", "grace_secs": "5m" }))
        .await;
    ok.assert_status(StatusCode::CREATED);
    let body = ok.json::<Value>();
    assert_eq!(body["period_secs"], 3600);
    assert_eq!(body["grace_secs"], 300);
    assert_eq!(body["status"], "new");
    assert!(!body["ping_uuid"].as_str().unwrap().is_empty());

    // Cron kind without an expression is rejected as a 400 envelope.
    let bad = server
        .post(&format!("/api/v1/projects/{pid}/checks"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "bad", "schedule_kind": "cron" }))
        .await;
    bad.assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(bad.json::<Value>()["error"]["code"], "bad_request");
}

/// Same coverage as `project_description_is_raw_markdown_and_defaults_to_empty`,
/// for checks: POST/PATCH accept `description`, GET returns the raw markdown
/// (not rendered), and omitting it on create yields `""`.
#[tokio::test]
async fn check_description_is_raw_markdown_and_defaults_to_empty() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();

    // Omitted on create → "".
    let created = server
        .post(&format!("/api/v1/projects/{pid}/checks"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "job", "period_secs": "1h", "grace_secs": "5m" }))
        .await;
    created.assert_status(StatusCode::CREATED);
    let body = created.json::<Value>();
    assert_eq!(body["description"], "");
    let cid = body["id"].as_i64().unwrap();

    // PATCH sets it; GET must return the raw markdown, not rendered HTML.
    let patched = server
        .patch(&format!("/api/v1/checks/{cid}"))
        .add_header("authorization", bearer(&token))
        .json(&json!({
            "name": "job",
            "description": "Runs **nightly**.",
            "period_secs": "1h",
            "grace_secs": "5m"
        }))
        .await;
    patched.assert_status_ok();
    assert_eq!(patched.json::<Value>()["description"], "Runs **nightly**.");

    let fetched = server
        .get(&format!("/api/v1/checks/{cid}"))
        .add_header("authorization", bearer(&token))
        .await;
    let text = fetched.text();
    assert!(
        text.contains("**nightly**"),
        "GET must return the raw markdown: {text}"
    );
    assert!(
        !text.contains("<strong>"),
        "GET must NOT render markdown to HTML: {text}"
    );
}

#[tokio::test]
async fn check_actions_change_state() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "job",
            ping_uuid: "uuid-orig",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 30,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();

    // pause → paused
    let paused = server
        .post(&format!("/api/v1/checks/{cid}/pause"))
        .add_header("authorization", bearer(&token))
        .await;
    paused.assert_status_ok();
    assert_eq!(paused.json::<Value>()["status"], "paused");

    // resume → new
    let resumed = server
        .post(&format!("/api/v1/checks/{cid}/resume"))
        .add_header("authorization", bearer(&token))
        .await;
    assert_eq!(resumed.json::<Value>()["status"], "new");

    // ack → acknowledged
    let acked = server
        .post(&format!("/api/v1/checks/{cid}/ack"))
        .add_header("authorization", bearer(&token))
        .await;
    assert_eq!(acked.json::<Value>()["acknowledged"], true);

    // regenerate → new ping uuid
    let regen = server
        .post(&format!("/api/v1/checks/{cid}/regenerate"))
        .add_header("authorization", bearer(&token))
        .await;
    assert_ne!(regen.json::<Value>()["ping_uuid"], "uuid-orig");
}

#[tokio::test]
async fn patch_check_replaces_schedule() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "job",
            ping_uuid: "uuid-x",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 30,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();

    let res = server
        .patch(&format!("/api/v1/checks/{cid}"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "renamed", "period_secs": "2h", "grace_secs": 45 }))
        .await;
    res.assert_status_ok();
    let body = res.json::<Value>();
    assert_eq!(body["name"], "renamed");
    assert_eq!(body["period_secs"], 7200);
    assert_eq!(body["grace_secs"], 45);
    // The ping UUID is preserved across a schedule update.
    assert_eq!(body["ping_uuid"], "uuid-x");
}

#[tokio::test]
async fn set_check_channels_honors_only_same_project_channels() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "job",
            ping_uuid: "uuid-ch",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 30,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook",
            "{\"url\":\"https://e.example\"}",
            Utc::now(),
        )
        .await
        .unwrap();

    // Bind the valid channel plus a bogus foreign id (9999) → only the valid one sticks.
    let res = server
        .put(&format!("/api/v1/checks/{cid}/channels"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "channel_ids": [ch, 9999] }))
        .await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["channel_ids"], json!([ch]));

    // Sending an empty set unbinds everything.
    let cleared = server
        .put(&format!("/api/v1/checks/{cid}/channels"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "channel_ids": [] }))
        .await;
    assert_eq!(cleared.json::<Value>()["channel_ids"], json!([]));
}

#[tokio::test]
async fn create_channel_hides_secrets_then_delete() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();

    let secret = "https://hooks.example.com/SECRET";
    let res = server
        .post(&format!("/api/v1/projects/{pid}/channels"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "hook", "kind": "webhook", "webhook_url": secret }))
        .await;
    res.assert_status(StatusCode::CREATED);
    let body = res.text();
    assert!(
        !body.contains(secret),
        "config secret must never be returned"
    );
    assert!(!body.contains("config_json"));
    let chid = res.json::<Value>()["id"].as_i64().unwrap();
    // But the config was stored (the channel is usable).
    let stored = store.find_channel(chid).await.unwrap().unwrap();
    assert!(stored.config_json.contains(secret));

    server
        .delete(&format!("/api/v1/channels/{chid}"))
        .add_header("authorization", bearer(&token))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert!(store.find_channel(chid).await.unwrap().is_none());
}

#[tokio::test]
async fn create_channel_rejects_missing_required_field() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    // webhook kind without a URL.
    let res = server
        .post(&format!("/api/v1/projects/{pid}/channels"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "hook", "kind": "webhook" }))
        .await;
    res.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn malformed_json_body_is_a_400_envelope() {
    let (server, store) = test_app().await;
    let (_uid, token) = user_with_key(&store, "alice", false).await;
    let res = server
        .post("/api/v1/projects")
        .add_header("authorization", bearer(&token))
        .add_header("content-type", "application/json")
        .text("{ not valid json ")
        .await;
    res.assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(res.json::<Value>()["error"]["code"], "bad_request");
}
