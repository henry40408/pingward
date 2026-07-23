//! End-to-end delivery test for the check-detail live tail
//! (`GET /checks/{id}/events`, `src/web.rs::check_events`).
//!
//! Deliberately does NOT use `axum_test`: its request helpers await the
//! *entire* response body, and an SSE body never ends (the connection stays
//! open, periodically emitting keep-alive comments), so a request built that
//! way would hang forever. Instead the router is driven directly with
//! `tower::ServiceExt::oneshot` and the body is read as a stream, bounded by
//! `tokio::time::timeout`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use pingward::{app, db, state::AppState, store::Store};
use std::time::Duration;
use tower::ServiceExt;

mod common;

/// A fresh, empty, migrated in-memory-SQLite store.
async fn test_store() -> Store {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    Store::new(pool)
}

/// Creates a user and a live (non-expired) session row directly through the
/// store — bypassing the `/login` handshake, which is unnecessary here since
/// `GET` requests are structurally exempt from the CSRF guard (see
/// `web::csrf_guard`). Returns the `Cookie` header value to attach to a raw
/// request — signed with `common::TEST_SECRET`, since the cookie carries
/// `<id>.<hmac>` and the bare id no longer authenticates anything.
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
            Utc::now() + chrono::Duration::days(pingward::auth::SESSION_TTL_DAYS),
            None,
            None,
            Utc::now(),
        )
        .await
        .unwrap();
    let value = pingward::secret::sign_session(common::TEST_SECRET.as_bytes(), &session_id);
    format!("{}={value}", pingward::auth::SESSION_COOKIE)
}

/// Reads chunks off `body` until `needle` has appeared in the accumulated
/// bytes, or `timeout` elapses (in which case this panics — a hang here
/// means the live-tail signal was never delivered to the subscriber).
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

/// A ping arriving for check N shows up on that check's `/checks/{id}/events`
/// stream as a `changed` SSE event. This is PR1's end-to-end contract: the
/// broadcast published by `ping::apply` reaches a subscriber that opened the
/// stream first, exactly as the browser (PR2) will.
///
/// Deliberately drives the REAL `/ping/{uuid}` endpoint rather than calling
/// `state.events.send(...)` directly — the latter only proves the broadcast
/// channel itself works, not that `ping::apply` actually publishes to it.
/// One `Router` (hence one `AppState`, hence one broadcast sender) serves
/// both requests via `.clone()` — `axum::Router` is cheap to clone (an `Arc`
/// underneath), and the two requests must share the same sender for the
/// signal to cross between them.
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

    // 1. Open the SSE stream FIRST. `ping::apply`'s publish is gated on
    // `events.receiver_count() > 0`, so the subscription must exist before
    // the ping below or the signal is sent to nobody and this test would
    // hang instead of failing loudly.
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

    // 2. Now hit the real ping endpoint for this check's `ping_uuid` — the
    // same path a monitored job would call.
    let ping_req = Request::builder()
        .method("POST")
        .uri("/ping/check-uuid")
        .body(Body::from("done"))
        .unwrap();
    let ping_resp = router.oneshot(ping_req).await.unwrap();
    assert_eq!(ping_resp.status(), StatusCode::OK);

    // 3. The SSE stream opened in step 1 should now carry a "changed" event.
    read_until_contains(resp.into_body(), "changed", Duration::from_secs(5)).await;
}

/// A non-owner requesting another user's check's event stream gets 404, same
/// as every other owner-scoped route (`owned_check` in `src/web.rs`).
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

    // A second, unrelated user.
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
