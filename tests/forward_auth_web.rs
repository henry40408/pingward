//! Forward-auth (trusted-header) users driving the browser surface.
//!
//! Driven with `oneshot` and an injected `ConnectInfo`: `axum_test` has no peer,
//! so `forward_auth_username` would reject every request.

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, Response, StatusCode, header};
use pingward::{app, config::Config, db, state::AppState, store::Store};
use std::net::SocketAddr;
use tower::ServiceExt;

const PROXY_PEER: &str = "172.18.0.5:44321";
const UNTRUSTED_PEER: &str = "8.8.8.8:44321";

/// No user seeded: forward-auth auto-provisions on first sight.
async fn empty_store() -> Store {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    Store::new(pool)
}

/// Secret pinned to `common::TEST_SECRET` so tests can sign/verify the flash
/// cookie and derive CSRF tokens themselves.
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

fn csrf_of(html: &str) -> String {
    let marker = r#"name="_csrf" value=""#;
    let start = html.find(marker).expect("the form has a _csrf input") + marker.len();
    html[start..start + html[start..].find('"').unwrap()].to_string()
}

/// Default `http://` base URL, so the cookie name is the unprefixed one.
fn session_cookie_of(resp: &Response<Body>) -> Option<String> {
    set_cookie_of(
        resp,
        &format!("{}=", pingward::auth::session_cookie_name(false)),
    )
}

/// First `Set-Cookie` `name=value` (attributes stripped) starting with `prefix`.
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

/// Returns `alice`'s session cookie and the page's CSRF token.
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
    // No `/login` visit: the session comes only from the proxy header.
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
    // The anonymous cookie it gets must address no account or session row.
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
    assert_eq!(
        resp.headers()["clear-site-data"],
        r#""cache""#,
        "handing off to the gateway also asks the browser to drop this origin's cache \
         (not \"cookies\": that directive reaches the whole registrable domain, which \
         would clear the gateway's own cookies on a sibling subdomain)"
    );
}

#[tokio::test]
async fn without_a_logout_url_a_forward_auth_logout_warns_on_the_dashboard() {
    // The gateway re-authenticates on the next request, so a bounce to `/login`
    // would silently sign the user back in; warn instead.
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
    assert!(
        !out.headers().contains_key("clear-site-data"),
        "the flash exit must omit Clear-Site-Data, or the warning below can never render"
    );
    let flash = set_cookie_of(&out, "pingward_flash=").expect("the warning flash cookie is set");
    let payload = flash
        .strip_prefix("pingward_flash=")
        .and_then(common::flash_payload);
    assert_eq!(
        payload.as_deref(),
        Some("forward_auth_logout"),
        "the flash must carry a forward_auth_logout payload signed by this server: {flash}"
    );

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
    // Every page sees the cookie; one that cleared it would swallow the warning
    // before the dashboard renders it.
    let store = empty_store().await;
    let state = AppState::new(store.clone(), forward_auth_config());
    let (cookie, _csrf) = signed_in_via_gateway(&state).await;

    let page = request(
        &state,
        PROXY_PEER,
        "GET",
        "/projects/new",
        Some("alice"),
        Some(&format!(
            "{cookie}; {}",
            common::signed_flash_cookie("forward_auth_logout")
        )),
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
async fn forward_auth_session_is_flagged_sso_on_the_account_page() {
    let store = empty_store().await;
    let state = AppState::new(store.clone(), forward_auth_config());
    let (cookie, _csrf) = signed_in_via_gateway(&state).await;

    let page = request(
        &state,
        PROXY_PEER,
        "GET",
        "/account",
        Some("alice"),
        Some(&cookie),
        Body::empty(),
    )
    .await;
    assert_eq!(page.status(), StatusCode::OK);
    let body = body_text(page).await;
    assert!(
        body.contains("data-testid=\"session-sso\""),
        "a forward-auth-minted session must render the SSO pill"
    );
}

#[tokio::test]
async fn a_stale_session_cookie_is_replaced_rather_than_trusted() {
    // A cookie outliving its row (pruned, or wiped by a migration) would
    // otherwise be a phantom session, absent from /account.
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
    // This request's own form must already use the fresh cookie's token.
    let id = fresh
        .trim_start_matches(&format!("{}=", pingward::auth::session_cookie_name(false)))
        .split('.')
        .next()
        .unwrap()
        .to_owned();
    assert_eq!(
        csrf_of(&body_text(resp).await),
        pingward::secret::derive_csrf(common::TEST_SECRET.as_bytes(), &id)
    );
}
