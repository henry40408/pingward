//! Integration tests for the read-only `/api/v1` bearer API (Stage B).

use axum::http::StatusCode;
use axum_test::TestServer;
use chrono::Utc;
use pingward::models::ScheduleKind;
use pingward::notify::EventKind;
use pingward::store::NewCheck;
use pingward::{apikey, app, config::Config, db, state::AppState, store::Store};
use serde_json::Value;

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
        .create_project(uid, "mine", None, None, now)
        .await
        .unwrap();
    let (other, _) = user_with_key(&store, "bob", false).await;
    store
        .create_project(other, "theirs", None, None, now)
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
        .create_project(other, "theirs", None, None, Utc::now())
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
        .create_project(other, "theirs", None, None, Utc::now())
        .await
        .unwrap();

    let res = server
        .get(&format!("/api/v1/projects/{pid}"))
        .add_header("authorization", format!("Bearer {token}"))
        .await;
    res.assert_status_ok();
    assert_eq!(res.json::<Value>()["name"], "theirs");

    // The cross-user read is audited.
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
        .create_project(admin, "mine", None, None, Utc::now())
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
        .create_project(uid, "p", None, None, Utc::now())
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
    // Cursor envelope: next_before points at the last (oldest) item on the
    // page; there is no newer page, so next_after is null.
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
        .create_project(uid, "p", None, None, Utc::now())
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
    // limit=0 clamps up to 1; a huge limit clamps down but still returns all 3.
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
        .create_project(uid, "p", None, None, Utc::now())
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
        .create_project(uid, "p", None, None, Utc::now())
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
    // The spec and the Scalar page are gated behind a web session: an
    // unauthenticated request redirects to /login rather than exposing them.
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
    server
        .post("/login")
        .form(&[("username", "member"), ("password", "pw")])
        .await;

    let spec = server.get("/api/openapi.json").await;
    spec.assert_status_ok();
    assert_eq!(spec.json::<Value>()["info"]["title"], "pingward API");

    let docs = server.get("/api/docs").await;
    docs.assert_status_ok();
    assert!(docs.text().to_lowercase().contains("scalar"));
}
