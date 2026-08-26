//! Integration tests for the `/api/v1` write API: create/update/delete, the
//! check actions (pause/resume/ack/regenerate), channel binding, admin
//! cross-user write auditing and JSON error envelopes.

use axum::http::StatusCode;
use axum_test::TestServer;
use chrono::Utc;
use pingward::{
    apikey, app, config::Config, db, models::ChannelKind, state::AppState, store::Store,
};
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

/// GET returns `description` as raw markdown, never through `markdown::render`;
/// omitting it on create yields `""`.
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

/// `Store::bind_all_project_channels` via `api::v1::create_check` — the same
/// guarantee as the web form.
#[tokio::test]
async fn create_check_is_bound_to_existing_project_channels() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    let c1 = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook1",
            r#"{"url":"http://x"}"#,
            Utc::now(),
        )
        .await
        .unwrap();
    let c2 = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook2",
            r#"{"url":"http://y"}"#,
            Utc::now(),
        )
        .await
        .unwrap();

    let res = server
        .post(&format!("/api/v1/projects/{pid}/checks"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "job", "period_secs": "1h", "grace_secs": "5m" }))
        .await;
    res.assert_status(StatusCode::CREATED);
    let cid = res.json::<Value>()["id"].as_i64().unwrap();

    let mut bound = store.bound_channel_ids(cid).await.unwrap();
    bound.sort_unstable();
    let mut expected = vec![c1, c2];
    expected.sort_unstable();
    assert_eq!(
        bound, expected,
        "a check created via the API in a project with existing channels must come out bound to all of them"
    );
}

/// As `project_description_is_raw_markdown_and_defaults_to_empty`, for checks.
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

    let paused = server
        .post(&format!("/api/v1/checks/{cid}/pause"))
        .add_header("authorization", bearer(&token))
        .await;
    paused.assert_status_ok();
    assert_eq!(paused.json::<Value>()["status"], "paused");

    let resumed = server
        .post(&format!("/api/v1/checks/{cid}/resume"))
        .add_header("authorization", bearer(&token))
        .await;
    assert_eq!(resumed.json::<Value>()["status"], "new");

    let acked = server
        .post(&format!("/api/v1/checks/{cid}/ack"))
        .add_header("authorization", bearer(&token))
        .await;
    assert_eq!(acked.json::<Value>()["acknowledged"], true);

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

/// The per-check overrides are accepted on write and returned on read —
/// `CheckDto` once omitted them, so read-modify-write was impossible. Asserted
/// on POST, GET and PATCH, since the DTO is what each renders.
#[tokio::test]
async fn check_override_fields_round_trip() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();

    let created = server
        .post(&format!("/api/v1/projects/{pid}/checks"))
        .add_header("authorization", bearer(&token))
        .json(&json!({
            "name": "job",
            "period_secs": "1h",
            "scan_interval_secs": "30s",
            "max_runtime_secs": "10m",
            "nag_interval_secs": "2h",
        }))
        .await;
    created.assert_status(StatusCode::CREATED);
    let body = created.json::<Value>();
    assert_eq!(body["scan_interval_secs"], 30);
    assert_eq!(body["max_runtime_secs"], 600);
    assert_eq!(body["nag_interval_secs"], 7200);
    let cid = body["id"].as_i64().unwrap();

    // Read back: the same values must survive a round-trip through the store.
    let fetched = server
        .get(&format!("/api/v1/checks/{cid}"))
        .add_header("authorization", bearer(&token))
        .await;
    fetched.assert_status_ok();
    let body = fetched.json::<Value>();
    assert_eq!(body["scan_interval_secs"], 30);
    assert_eq!(body["max_runtime_secs"], 600);
    assert_eq!(body["nag_interval_secs"], 7200);

    // PATCH replaces the whole check (see `patch_project_replaces_fields`), so an
    // override left out of the body comes back null rather than retained.
    let patched = server
        .patch(&format!("/api/v1/checks/{cid}"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "job", "period_secs": 60, "max_runtime_secs": 120 }))
        .await;
    patched.assert_status_ok();
    let body = patched.json::<Value>();
    assert_eq!(body["max_runtime_secs"], 120);
    assert!(body["scan_interval_secs"].is_null());
    assert!(body["nag_interval_secs"].is_null());
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

/// `PATCH /channels/{id}` merges rather than replaces: a field the caller does
/// not send keeps its stored value. A client cannot re-send the secrets — no API
/// response contains them — so a replacing patch would make a rename impossible.
#[tokio::test]
async fn patch_channel_renames_without_touching_the_stored_secret() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    let secret = "https://hooks.example.com/SECRET";
    let chid = store
        .create_channel(
            pid,
            ChannelKind::Webhook,
            "hook",
            &json!({ "url": secret }).to_string(),
            Utc::now(),
        )
        .await
        .unwrap();

    let res = server
        .patch(&format!("/api/v1/channels/{chid}"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "renamed" }))
        .await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        !body.contains(secret),
        "a patch response must not echo the stored secret back"
    );
    assert!(!body.contains("config_json"));
    assert_eq!(res.json::<Value>()["name"], "renamed");

    let stored = store.find_channel(chid).await.unwrap().unwrap();
    assert_eq!(stored.name, "renamed");
    assert!(
        stored.config_json.contains(secret),
        "an omitted credential must keep its stored value, got {}",
        stored.config_json
    );
}

/// The other half of the merge rule: a submitted field overwrites, so rotating
/// one credential does not require re-sending the rest.
#[tokio::test]
async fn patch_channel_rotates_only_the_submitted_credential() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    let chid = store
        .create_channel(
            pid,
            ChannelKind::Telegram,
            "tg",
            &json!({ "token": "OLD-TOKEN", "chat_id": "12345" }).to_string(),
            Utc::now(),
        )
        .await
        .unwrap();

    server
        .patch(&format!("/api/v1/channels/{chid}"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "telegram_token": "NEW-TOKEN" }))
        .await
        .assert_status_ok();

    let stored = store.find_channel(chid).await.unwrap().unwrap();
    let cfg: Value = serde_json::from_str(&stored.config_json).unwrap();
    assert_eq!(cfg["token"], "NEW-TOKEN", "the submitted secret must win");
    assert_eq!(cfg["chat_id"], "12345", "the omitted field must be kept");
    assert_eq!(stored.name, "tg", "an omitted name must be kept");
}

/// `kind` is immutable: `config_json` only has meaning for the kind that wrote
/// it, so a submitted `kind` is ignored rather than reinterpreting the config.
#[tokio::test]
async fn patch_channel_ignores_a_submitted_kind() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    let chid = store
        .create_channel(
            pid,
            ChannelKind::Webhook,
            "hook",
            &json!({ "url": "https://hooks.example.com/SECRET" }).to_string(),
            Utc::now(),
        )
        .await
        .unwrap();

    let res = server
        .patch(&format!("/api/v1/channels/{chid}"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "kind": "slack", "slack_url": "https://hooks.slack.com/services/X" }))
        .await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["kind"], "webhook");

    let stored = store.find_channel(chid).await.unwrap().unwrap();
    assert_eq!(stored.kind, ChannelKind::Webhook);
    // The webhook block reads `webhook_url`, which was blank, so the stored URL
    // stands; the slack field is ignored.
    let cfg: Value = serde_json::from_str(&stored.config_json).unwrap();
    assert_eq!(cfg["url"], "https://hooks.example.com/SECRET");
}

/// The one optional secret needs an explicit clear: "blank keeps the stored
/// value" would otherwise make a set ntfy token impossible to remove.
#[tokio::test]
async fn patch_channel_clears_the_ntfy_token_on_request() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    let chid = store
        .create_channel(
            pid,
            ChannelKind::Ntfy,
            "n",
            &json!({ "base_url": "https://ntfy.sh", "topic": "t", "token": "TOK" }).to_string(),
            Utc::now(),
        )
        .await
        .unwrap();

    // A blank token alone keeps it, so the clear below is proven to be what did
    // the work.
    server
        .patch(&format!("/api/v1/channels/{chid}"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "ntfy_token": "" }))
        .await
        .assert_status_ok();
    let kept = store.find_channel(chid).await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&kept.config_json).unwrap()["token"],
        "TOK"
    );

    server
        .patch(&format!("/api/v1/channels/{chid}"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "ntfy_token_clear": true }))
        .await
        .assert_status_ok();
    let cleared = store.find_channel(chid).await.unwrap().unwrap();
    let cfg: Value = serde_json::from_str(&cleared.config_json).unwrap();
    assert_eq!(cfg["token"], "");
    assert_eq!(
        cfg["topic"], "t",
        "clearing the token must not touch the rest"
    );
}

/// The merge shares one rule set with create: a patch that would leave a
/// required credential empty is still rejected.
#[tokio::test]
async fn patch_channel_rejects_blanking_a_required_credential() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
    // A config with no `url` at all: a blank submission has nothing to fall back to.
    let chid = store
        .create_channel(
            pid,
            ChannelKind::Webhook,
            "hook",
            &json!({}).to_string(),
            Utc::now(),
        )
        .await
        .unwrap();

    server
        .patch(&format!("/api/v1/channels/{chid}"))
        .add_header("authorization", bearer(&token))
        .json(&json!({ "name": "renamed" }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(
        store.find_channel(chid).await.unwrap().unwrap().name,
        "hook",
        "a rejected patch must not have applied the name either"
    );
}

#[tokio::test]
async fn create_channel_rejects_missing_required_field() {
    let (server, store) = test_app().await;
    let (uid, token) = user_with_key(&store, "alice", false).await;
    let pid = store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap();
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
