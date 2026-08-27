//! End-to-end delivery test for the check-detail live tail
//! (`GET /checks/{id}/events`, `src/web.rs::check_events`).
//!
//! Not `axum_test`: its request helpers await the *entire* response body, and
//! an SSE body never ends, so such a request hangs forever. The router is driven
//! with `tower::ServiceExt::oneshot` and the body read as a stream under a
//! `tokio::time::timeout`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use pingward::{app, db, state::AppState, store::Store};
use std::time::Duration;
use tower::ServiceExt;

mod common;

/// A fresh, migrated in-memory-SQLite store.
async fn test_store() -> Store {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    Store::new(pool)
}

/// Creates a user and a live session row directly through the store, returning
/// the `Cookie` header value: `<id>.<hmac>` signed with `common::TEST_SECRET`,
/// since the bare id no longer authenticates anything.
async fn login_cookie(store: &Store, username: &str) -> String {
    let phc = pingward::auth::hash_password("pw").unwrap();
    let user_id = store
        .create_user(username, Some(&phc), false, Utc::now())
        .await
        .unwrap();
    let session_id = pingward::auth::new_session_token();
    store
        .create_session(
            &session_id,
            user_id,
            Utc::now() + chrono::Duration::hours(pingward::auth::SESSION_IDLE_TTL_HOURS),
            None,
            None,
            false,
            Utc::now(),
        )
        .await
        .unwrap();
    let value = pingward::secret::sign_session(common::TEST_SECRET.as_bytes(), &session_id);
    format!("{}={value}", pingward::auth::session_cookie_name(false))
}

/// Reads `body` until `needle` appears, panicking on timeout — a hang there
/// means the live-tail signal was never delivered to the subscriber.
async fn read_until_contains(body: axum::body::Body, needle: &str, timeout: Duration) {
    let fut = async {
        let mut buf = Vec::new();
        let mut stream = body.into_data_stream();
        loop {
            use tokio_stream::StreamExt as _;
            let chunk = stream
                .next()
                .await
                .expect("SSE body ended before the expected event arrived")
                .expect("error reading SSE body chunk");
            buf.extend_from_slice(&chunk);
            if String::from_utf8_lossy(&buf).contains(needle) {
                return;
            }
        }
    };
    tokio::time::timeout(timeout, fut)
        .await
        .unwrap_or_else(|_| {
            panic!("timed out after {timeout:?} waiting for {needle:?} in SSE body")
        });
}

/// The broadcast published by `ping::apply` reaches a subscriber that opened the
/// stream first. Drives the real `/ping/{uuid}` endpoint rather than calling
/// `state.events.send(...)`, which would only prove the channel itself works.
/// Both requests go through one cloned `Router`, so they share one sender.
#[tokio::test]
async fn owner_receives_changed_event_when_check_is_pinged() {
    let store = test_store().await;
    let state = AppState::new(store.clone(), common::test_config());
    let router = app(state);

    let cookie = login_cookie(&store, "alice").await;
    let owner_id = store
        .find_user_by_username("alice")
        .await
        .unwrap()
        .unwrap()
        .id;
    let pid = store
        .create_project(owner_id, "proj", "", None, None, Utc::now())
        .await
        .unwrap();
    let check_id = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "check",
            ping_uuid: "check-uuid",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 30,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();

    // 1. Open the SSE stream first: `ping::apply`'s publish is gated on
    // `events.receiver_count() > 0`, so without the subscription the signal goes
    // nowhere and this test hangs instead of failing loudly.
    let sse_req = Request::builder()
        .uri(format!("/checks/{check_id}/events"))
        .header("cookie", &cookie)
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(sse_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("text/event-stream"),
        "expected an SSE content-type, got {content_type:?}"
    );

    // 2. The real ping endpoint, the same path a monitored job would call.
    let ping_req = Request::builder()
        .method("POST")
        .uri("/ping/check-uuid")
        .body(Body::from("done"))
        .unwrap();
    let ping_resp = router.oneshot(ping_req).await.unwrap();
    assert_eq!(ping_resp.status(), StatusCode::OK);

    read_until_contains(resp.into_body(), "changed", Duration::from_secs(5)).await;
}

/// 404, as on every other owner-scoped route (`owned_check` in `src/web.rs`).
#[tokio::test]
async fn non_owner_gets_404_from_check_events() {
    let store = test_store().await;
    let state = AppState::new(store.clone(), common::test_config());

    let owner_id = {
        let phc = pingward::auth::hash_password("pw").unwrap();
        store
            .create_user("alice", Some(&phc), false, Utc::now())
            .await
            .unwrap()
    };
    let pid = store
        .create_project(owner_id, "proj", "", None, None, Utc::now())
        .await
        .unwrap();
    let check_id = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "check",
            ping_uuid: "check-uuid-2",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 30,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();

    let cookie = login_cookie(&store, "mallory").await;

    let req = Request::builder()
        .uri(format!("/checks/{check_id}/events"))
        .header("cookie", &cookie)
        .body(Body::empty())
        .unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), app(state).oneshot(req))
        .await
        .expect("non-owner request should resolve to 404 promptly, not stream")
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
