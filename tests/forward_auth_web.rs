//! Forward-auth (trusted-header) users driving the browser surface.
//!
//! Deliberately does NOT use `axum_test`: it never populates
//! `ConnectInfo<SocketAddr>`, so the peer would be absent and
//! `forward_auth_username` would reject every request before the header is even
//! read. The router is driven with `tower::ServiceExt::oneshot` and the peer is
//! injected as a request extension, exactly as
//! `into_make_service_with_connect_info` does in `main.rs` — same approach as
//! `tests/ping_source_ip.rs`.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, Response, StatusCode, header};
use pingward::{app, config::Config, db, state::AppState, store::Store};
use std::net::SocketAddr;
use tower::ServiceExt;

const PROXY_PEER: &str = "172.18.0.5:44321";
const UNTRUSTED_PEER: &str = "8.8.8.8:44321";

/// A migrated, empty store. Forward-auth auto-provisions its user on first
/// sight, so no user row is seeded.
async fn empty_store() -> Store {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    Store::new(pool)
}

/// `PINGWARD_SECRET` is pinned so the two requests of a flow share one signing
/// key — `Config` otherwise generates a random one per instance.
fn forward_auth_config() -> Config {
    Config::from_map(|k| match k {
        "PINGWARD_FORWARD_AUTH_HEADER" => Some("Remote-User".into()),
        "PINGWARD_TRUSTED_PROXIES" => Some("172.16.0.0/12".into()),
        "PINGWARD_SECRET" => Some(common::TEST_SECRET.into()),
        _ => None,
    })
}

/// Drives one request against a router built from `state`, as `peer` would see
/// it, optionally carrying `Remote-User` and a `Cookie` header.
async fn request(
    state: &AppState,
    peer: &str,
    method: &str,
    uri: &str,
    remote_user: Option<&str>,
    cookie: Option<&str>,
    body: Body,
) -> Response<Body> {
    let peer: SocketAddr = peer.parse().unwrap();
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .extension(ConnectInfo(peer));
    if let Some(user) = remote_user {
        req = req.header("remote-user", user);
    }
    if let Some(cookie) = cookie {
        req = req.header(header::COOKIE, cookie);
    }
    if method == "POST" {
        req = req.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    }
    app(state.clone())
        .oneshot(req.body(body).unwrap())
        .await
        .unwrap()
}

async fn body_text(resp: Response<Body>) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// The value of the hidden `_csrf` input in `html`.
fn csrf_of(html: &str) -> String {
    let marker = r#"name="_csrf" value=""#;
    let start = html.find(marker).expect("the form has a _csrf input") + marker.len();
    html[start..start + html[start..].find('"').unwrap()].to_string()
}

/// The `pingward_session=...` pair from a response's `Set-Cookie` headers.
fn session_cookie_of(resp: &Response<Body>) -> Option<String> {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("pingward_session="))
        .map(|v| v.split(';').next().unwrap().to_owned())
}

fn new_project_body(csrf: &str) -> Body {
    Body::from(format!(
        "_csrf={}&name=demo&description=&scan_interval_secs=60&nag_interval_secs=3600",
        form_urlencoded::byte_serialize(csrf.as_bytes()).collect::<String>()
    ))
}

#[tokio::test]
async fn forward_auth_user_can_create_a_project() {
    // The deployment this fixes: Caddy + Authelia in front, pingward reached
    // only through the proxy, so no user ever visits `/login` and no session
    // cookie is ever minted by it.
    let store = empty_store().await;
    let state = AppState::new(store.clone(), forward_auth_config());

    let form = request(
        &state,
        PROXY_PEER,
        "GET",
        "/projects/new",
        Some("alice"),
        None,
        Body::empty(),
    )
    .await;
    assert_eq!(form.status(), StatusCode::OK);
    let cookie = session_cookie_of(&form).expect("forward-auth should mint a session cookie");
    let csrf = csrf_of(&body_text(form).await);
    assert!(!csrf.is_empty(), "the rendered form must carry a token");

    let created = request(
        &state,
        PROXY_PEER,
        "POST",
        "/projects",
        Some("alice"),
        Some(&cookie),
        new_project_body(&csrf),
    )
    .await;
    assert_eq!(
        created.status(),
        StatusCode::SEE_OTHER,
        "the CSRF guard rejected a forward-auth user's own form"
    );
    assert_eq!(store.list_projects_for_user(1).await.unwrap().len(), 1);
}

#[tokio::test]
async fn forward_auth_session_is_reused_across_requests() {
    // One session row per browser, not one per request: the second GET already
    // carries the cookie, so nothing new is minted.
    let store = empty_store().await;
    let state = AppState::new(store.clone(), forward_auth_config());

    let first = request(
        &state,
        PROXY_PEER,
        "GET",
        "/projects/new",
        Some("alice"),
        None,
        Body::empty(),
    )
    .await;
    let cookie = session_cookie_of(&first).expect("first request mints a session");

    let second = request(
        &state,
        PROXY_PEER,
        "GET",
        "/projects/new",
        Some("alice"),
        Some(&cookie),
        Body::empty(),
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    assert!(
        session_cookie_of(&second).is_none(),
        "a live session must not be replaced"
    );
    let sessions = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(sessions, 1);
}

#[tokio::test]
async fn forward_auth_header_from_an_untrusted_peer_mints_nothing() {
    // Anyone can set `Remote-User`; only the configured proxy is believed.
    let store = empty_store().await;
    let state = AppState::new(store.clone(), forward_auth_config());

    let resp = request(
        &state,
        UNTRUSTED_PEER,
        "GET",
        "/projects/new",
        Some("mallory"),
        None,
        Body::empty(),
    )
    .await;
    // A cookie *is* set — the anonymous-session layer gives one to every
    // visitor — but it must address nothing: no account, no session row, and
    // the request still bounces to the login page.
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers()["location"], "/login");
    for (table, sql) in [
        ("users", "SELECT COUNT(*) FROM users"),
        ("sessions", "SELECT COUNT(*) FROM sessions"),
    ] {
        let count = sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "an untrusted peer must not populate `{table}`");
    }
}

#[tokio::test]
async fn a_stale_session_cookie_is_replaced_rather_than_trusted() {
    // The cookie outlives its row (pruned, or wiped by the 0012 migration).
    // Without replacement the user keeps a phantom session forever: authorised
    // by the header, but absent from the account page's session list.
    let store = empty_store().await;
    let state = AppState::new(store.clone(), forward_auth_config());

    let first = request(
        &state,
        PROXY_PEER,
        "GET",
        "/projects/new",
        Some("alice"),
        None,
        Body::empty(),
    )
    .await;
    let stale = session_cookie_of(&first).expect("first request mints a session");
    sqlx::query("DELETE FROM sessions")
        .execute(&store.pool)
        .await
        .unwrap();

    let resp = request(
        &state,
        PROXY_PEER,
        "GET",
        "/projects/new",
        Some("alice"),
        Some(&stale),
        Body::empty(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let fresh = session_cookie_of(&resp).expect("a stale cookie must be replaced");
    assert_ne!(fresh, stale);
    // The form rendered in *this* request must match the fresh cookie, not the
    // stale one — that is what the request-side cookie rewrite buys.
    let id = fresh
        .trim_start_matches("pingward_session=")
        .split('.')
        .next()
        .unwrap()
        .to_owned();
    assert_eq!(
        csrf_of(&body_text(resp).await),
        pingward::secret::derive_csrf(common::TEST_SECRET.as_bytes(), &id)
    );
}
