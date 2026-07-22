//! What address a ping is recorded from (`pings.source_ip`, shown as "Source"
//! on the check page).
//!
//! Deliberately does NOT use `axum_test`: it never populates
//! `ConnectInfo<SocketAddr>`, so every request would have no peer and the whole
//! trusted-proxy decision would be skipped. The router is driven directly with
//! `tower::ServiceExt::oneshot` and the peer is injected as a request
//! extension, exactly as `into_make_service_with_connect_info` does in
//! `main.rs`.

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use pingward::{
    app,
    config::Config,
    db,
    models::ScheduleKind,
    state::AppState,
    store::{NewCheck, Store},
};
use std::net::SocketAddr;
use tower::ServiceExt;

/// A migrated in-memory store with one user, one project, and a check whose
/// ping UUID is `abc`.
async fn seeded_store() -> Store {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    sqlx::query(
        "INSERT INTO users (username, is_admin, created_at) VALUES ('u',0,datetime('now'))",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO projects (user_id, name, created_at) VALUES (1,'p',datetime('now'))")
        .execute(&pool)
        .await
        .unwrap();
    let store = Store::new(pool);
    store
        .create_check(&NewCheck {
            project_id: 1,
            name: "job",
            ping_uuid: "abc",
            kind: ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 30,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    store
}

/// Pings `/ping/abc` from `peer`, optionally carrying `X-Forwarded-For`, and
/// returns the `source_ip` that was recorded.
async fn ping_from(
    trusted_proxies: Option<&str>,
    peer: &str,
    forwarded_for: Option<&str>,
) -> Option<String> {
    let store = seeded_store().await;
    let proxies = trusted_proxies.map(str::to_owned);
    let config = Config::from_map(move |k| match k {
        "PINGWARD_TRUSTED_PROXIES" => proxies.clone(),
        _ => None,
    });
    let state = AppState::new(store.clone(), config);

    let peer: SocketAddr = peer.parse().unwrap();
    let mut req = Request::builder()
        .uri("/ping/abc")
        .extension(ConnectInfo(peer));
    if let Some(xff) = forwarded_for {
        req = req.header("x-forwarded-for", xff);
    }
    let resp = app(state)
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let pings = store.list_recent_pings(1, 1).await.unwrap();
    assert_eq!(pings.len(), 1, "the ping should have been recorded");
    pings[0].source_ip.clone()
}

#[tokio::test]
async fn ping_behind_a_trusted_proxy_records_the_forwarded_client() {
    // The deployment this fixes: pingward in a container with Caddy in front,
    // so the peer is always the proxy's bridge-network address.
    let ip = ping_from(
        Some("172.16.0.0/12"),
        "172.18.0.5:44321",
        Some("203.0.113.7, 172.18.0.5"),
    )
    .await;
    assert_eq!(ip.as_deref(), Some("203.0.113.7"));
}

#[tokio::test]
async fn ping_from_an_untrusted_peer_records_the_peer() {
    // Ping endpoints are public, so anyone can set the header. Without the
    // peer being trusted it must be ignored.
    let ip = ping_from(Some("172.16.0.0/12"), "8.8.8.8:44321", Some("203.0.113.7")).await;
    assert_eq!(ip.as_deref(), Some("8.8.8.8"));
}

#[tokio::test]
async fn ping_without_trusted_proxies_records_the_peer() {
    let ip = ping_from(None, "172.18.0.5:44321", Some("203.0.113.7")).await;
    assert_eq!(ip.as_deref(), Some("172.18.0.5"));
}
