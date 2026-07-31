//! The response headers that are not `Strict-Transport-Security` (see
//! `tests/hsts.rs`): a Content-Security-Policy over the browser UI, plus the
//! nosniff/framing/referrer/permissions set over the whole origin.
//!
//! The split is the point of most of these assertions. The CSP names
//! `script-src 'self'` with no `'unsafe-inline'`, which only holds because
//! every script is a file under `/assets` and no template carries an inline
//! event attribute — so the regression these tests exist to catch is someone
//! reintroducing one and quietly widening the policy to match. `/api/docs`
//! loads its bundle from a CDN and is deliberately left outside the CSP, but
//! must still carry the origin-wide headers.

use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};

mod common;

async fn server() -> TestServer {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let state = AppState::new(Store::new(pool), common::test_config());
    TestServer::new(app(state))
}

/// Every router, so a header claimed to be app-wide is checked on one route
/// from each: the browser UI, the machine ping endpoints, the health probe,
/// static assets, and the API.
const EVERY_ROUTER: [&str; 5] = [
    "/login",
    "/ping/does-not-exist",
    "/healthz",
    "/assets/app.css",
    "/api/openapi.json",
];

#[tokio::test]
async fn the_origin_wide_headers_are_on_every_router() {
    let server = server().await;
    for route in EVERY_ROUTER {
        let res = server.get(route).await;
        let header = |name: &str| res.header(name).to_str().unwrap().to_string();
        assert_eq!(header("x-content-type-options"), "nosniff", "{route}");
        assert_eq!(header("x-frame-options"), "DENY", "{route}");
        assert_eq!(header("referrer-policy"), "same-origin", "{route}");
        assert!(
            header("permissions-policy").contains("geolocation=()"),
            "{route}"
        );
    }
}

#[tokio::test]
async fn the_browser_ui_carries_a_script_src_self_policy() {
    let server = server().await;
    let res = server.get("/login").await;
    let csp = res.header("content-security-policy");
    let csp = csp.to_str().unwrap();

    // The whole arrangement rests on this: no `'unsafe-inline'` and no nonce
    // in `script-src`. Both would make an injected `<script>` runnable again.
    assert!(csp.contains("script-src 'self';"), "{csp}");
    assert!(!csp.contains("script-src 'self' 'unsafe-inline'"), "{csp}");
    assert!(!csp.contains("nonce-"), "{csp}");
    for directive in [
        "default-src 'self'",
        "object-src 'none'",
        "base-uri 'none'",
        "form-action 'self'",
        "frame-ancestors 'none'",
        // The live tail's EventSource is same-origin; blocking it would make
        // the LIVE toggle silently do nothing.
        "connect-src 'self'",
    ] {
        assert!(csp.contains(directive), "missing {directive}: {csp}");
    }
    // The heartbeat bars carry a computed `style="height:Npx"`, so inline
    // styles stay allowed — deliberately, and only for styles.
    assert!(csp.contains("style-src 'self' 'unsafe-inline'"), "{csp}");
}

/// `/api/docs` renders Scalar, which pulls its bundle from `cdn.jsdelivr.net`;
/// `script-src 'self'` would leave the page blank. It is outside the CSP layer
/// rather than the policy being widened for everyone — but it still gets the
/// origin-wide headers, framing included.
#[tokio::test]
async fn the_api_docs_page_is_outside_the_csp_but_not_the_rest() {
    let server = server().await;
    let res = server.get("/api/docs").await;
    // Signed out it redirects; either way the layers below have run.
    assert!(
        !res.headers().contains_key("content-security-policy"),
        "/api/docs must not inherit the browser UI's CSP"
    );
    assert_eq!(
        res.header("x-frame-options").to_str().unwrap(),
        "DENY",
        "/api/docs still must not be framable"
    );
}

#[tokio::test]
async fn the_scripts_are_served_as_javascript_and_do_not_shadow_the_stylesheet() {
    let server = server().await;
    for script in ["/assets/app.js", "/assets/theme-init.js"] {
        let res = server.get(script).await;
        res.assert_status_ok();
        assert!(
            res.header("content-type")
                .to_str()
                .unwrap()
                .starts_with("text/javascript"),
            "{script}"
        );
    }
    // `/assets/{file}` is a wildcard route: the literal `/assets/app.css` must
    // still win, or the stylesheet would 404 as an unknown script name.
    let css = server.get("/assets/app.css").await;
    css.assert_status_ok();
    assert!(
        css.header("content-type")
            .to_str()
            .unwrap()
            .starts_with("text/css")
    );
    server
        .get("/assets/nope.js")
        .await
        .assert_status(axum::http::StatusCode::NOT_FOUND);
}
