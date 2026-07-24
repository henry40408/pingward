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

/// [`forward_auth_config`] plus a gateway sign-out URL.
fn logout_url_config() -> Config {
    Config::from_map(|k| match k {
        "PINGWARD_FORWARD_AUTH_HEADER" => Some("Remote-User".into()),
        "PINGWARD_TRUSTED_PROXIES" => Some("172.16.0.0/12".into()),
        "PINGWARD_SECRET" => Some(common::TEST_SECRET.into()),
        "PINGWARD_FORWARD_AUTH_LOGOUT_URL" => Some(GATEWAY_LOGOUT.into()),
        _ => None,
    })
}

const GATEWAY_LOGOUT: &str = "https://auth.example.com/logout";

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
    set_cookie_of(resp, "pingward_session=")
}

/// The first `Set-Cookie` pair (`name=value`, attributes stripped) whose name
/// starts with `prefix`.
fn set_cookie_of(resp: &Response<Body>, prefix: &str) -> Option<String> {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with(prefix))
        .map(|v| v.split(';').next().unwrap().to_owned())
}

fn new_project_body(csrf: &str) -> Body {
    Body::from(format!(
        "_csrf={}&name=demo&description=&scan_interval_secs=60&nag_interval_secs=3600",
        form_urlencoded::byte_serialize(csrf.as_bytes()).collect::<String>()
    ))
}

fn csrf_body(csrf: &str) -> Body {
    Body::from(format!(
        "_csrf={}",
        form_urlencoded::byte_serialize(csrf.as_bytes()).collect::<String>()
    ))
}

/// Signs `alice` in through the gateway and returns her session cookie plus the
/// CSRF token of the nav's log-out form, ready to POST with.
async fn signed_in_via_gateway(state: &AppState) -> (String, String) {
    let page = request(
        state,
        PROXY_PEER,
        "GET",
        "/projects/new",
        Some("alice"),
        None,
        Body::empty(),
    )
    .await;
    let cookie = session_cookie_of(&page).expect("forward auth mints a session cookie");
    (cookie, csrf_of(&body_text(page).await))
}

async fn session_count(store: &Store) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions")
        .fetch_one(&store.pool)
        .await
        .unwrap()
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
async fn logout_hands_off_to_the_gateway_when_a_url_is_configured() {
    // The local session is still ended — the redirect is what additionally lets
    // the gateway end the identity that would otherwise sign the visitor
    // straight back in.
    let store = empty_store().await;
    let state = AppState::new(store.clone(), logout_url_config());
    let (cookie, csrf) = signed_in_via_gateway(&state).await;
    assert_eq!(session_count(&store).await, 1);

    let resp = request(
        &state,
        PROXY_PEER,
        "POST",
        "/logout",
        Some("alice"),
        Some(&cookie),
        csrf_body(&csrf),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers()["location"], GATEWAY_LOGOUT);
    assert_eq!(
        session_count(&store).await,
        0,
        "the local session must be deleted whatever the redirect target"
    );
}

#[tokio::test]
async fn without_a_logout_url_a_forward_auth_logout_warns_on_the_dashboard() {
    // Nothing pingward deletes can outlive the redirect while the gateway keeps
    // sending an identity header. Rather than bounce to `/login` and silently
    // re-authenticate, `logout` lands the visitor on `/` with a one-shot flash
    // telling them only their proxy/SSO provider can end the session.
    let store = empty_store().await;
    let state = AppState::new(store.clone(), forward_auth_config());
    let (cookie, csrf) = signed_in_via_gateway(&state).await;

    let out = request(
        &state,
        PROXY_PEER,
        "POST",
        "/logout",
        Some("alice"),
        Some(&cookie),
        csrf_body(&csrf),
    )
    .await;
    assert_eq!(out.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        out.headers()["location"],
        "/",
        "a forward-auth logout lands on the dashboard, not the login form"
    );
    assert_eq!(
        session_count(&store).await,
        0,
        "the local session is deleted"
    );
    let flash = set_cookie_of(&out, "pingward_flash=").expect("the warning flash cookie is set");
    assert_eq!(flash, "pingward_flash=forward_auth_logout");

    // The dashboard the browser lands on: the gateway re-mints the session, and
    // the flash renders exactly once.
    let dash = request(
        &state,
        PROXY_PEER,
        "GET",
        "/",
        Some("alice"),
        Some(&format!("{cookie}; {flash}")),
        Body::empty(),
    )
    .await;
    assert_eq!(dash.status(), StatusCode::OK);
    assert_eq!(session_count(&store).await, 1, "a fresh session was minted");
    // The one-shot cookie is cleared on this render so the warning shows once.
    let cleared = set_cookie_of(&dash, "pingward_flash=").expect("the flash cookie is cleared");
    assert_eq!(cleared, "pingward_flash=");
    let html = body_text(dash).await;
    assert!(
        html.contains(r#"data-testid="forward-auth-logout-flash""#),
        "the dashboard must render the forward-auth logout warning"
    );
    assert!(
        html.contains("log out at your proxy or SSO provider"),
        "the warning must tell the visitor where to actually sign out"
    );
}

#[tokio::test]
async fn a_forward_auth_logout_flash_does_not_leak_onto_other_pages() {
    // The flash cookie is path-scoped to `/`, so every page sees it; only the
    // dashboard is meant to consume it. A page that does not know the surface
    // must neither render nor clear it — otherwise a redirect that skips the
    // dashboard would silently swallow the warning.
    let store = empty_store().await;
    let state = AppState::new(store.clone(), forward_auth_config());
    let (cookie, _csrf) = signed_in_via_gateway(&state).await;

    let page = request(
        &state,
        PROXY_PEER,
        "GET",
        "/projects/new",
        Some("alice"),
        Some(&format!("{cookie}; pingward_flash=forward_auth_logout")),
        Body::empty(),
    )
    .await;
    assert_eq!(page.status(), StatusCode::OK);
    assert!(
        set_cookie_of(&page, "pingward_flash=").is_none(),
        "a page that does not own the surface must leave the cookie intact"
    );
    assert!(
        !body_text(page).await.contains("forward-auth-logout-flash"),
        "the warning must not render off the dashboard"
    );
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
