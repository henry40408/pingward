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
use pingward::{app, config::Config, db, state::AppState, store::Store};
use std::time::Duration;
use tower::ServiceExt;

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
/// request.
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
            "csrf-token-unused-for-get",
            Utc::now() + chrono::Duration::days(pingward::auth::SESSION_TTL_DAYS),
            None,
            None,
            Utc::now(),
        )
        .await
        .unwrap();
    format!("{}={session_id}", pingward::auth::SESSION_COOKIE)
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
#[tokio::test]
async fn owner_receives_changed_event_when_check_is_pinged() {
    let store = test_store().await;
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let events = state.events.clone();

    let cookie = login_cookie(&store, "alice").await;
    let owner_id = store
        .find_user_by_username("alice")
        .await
        .unwrap()
        .unwrap()
        .id;
    let pid = store
        .create_project(owner_id, "proj", None, None, Utc::now())
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

    let req = Request::builder()
        .uri(format!("/checks/{check_id}/events"))
        .header("cookie", &cookie)
        .body(Body::empty())
        .unwrap();

    // `app(state)` moves `state`; `events` was cloned above so the sender
    // survives for the `send` below.
    let resp = app(state).oneshot(req).await.unwrap();
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

    // The handler has already subscribed by the time `oneshot` returns a
    // response (subscription happens while the response/stream is built), so
    // a `send` issued here is guaranteed to reach it.
    events.send(check_id).unwrap();

    read_until_contains(resp.into_body(), "changed", Duration::from_secs(5)).await;
}

/// A non-owner requesting another user's check's event stream gets 404, same
/// as every other owner-scoped route (`owned_check` in `src/web.rs`).
#[tokio::test]
async fn non_owner_gets_404_from_check_events() {
    let store = test_store().await;
    let state = AppState::new(store.clone(), Config::from_map(|_| None));

    let owner_id = {
        let phc = pingward::auth::hash_password("pw").unwrap();
        store
            .create_user("alice", Some(&phc), false, Utc::now())
            .await
            .unwrap()
    };
    let pid = store
        .create_project(owner_id, "proj", None, None, Utc::now())
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
