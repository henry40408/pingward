//! Response headers other than `Strict-Transport-Security` (see `tests/hsts.rs`).
//! Guards against the CSP's `script-src 'self'` being widened.

use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};

mod common;

async fn server() -> TestServer {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let state = AppState::new(Store::new(pool), common::test_config());
    TestServer::new(app(state))
}

/// One route per router, so an "app-wide" header is checked on all of them.
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

    assert!(csp.contains("script-src 'self';"), "{csp}");
    assert!(!csp.contains("script-src 'self' 'unsafe-inline'"), "{csp}");
    assert!(!csp.contains("nonce-"), "{csp}");
    for directive in [
        "default-src 'self'",
        "object-src 'none'",
        "base-uri 'none'",
        "form-action 'self'",
        "frame-ancestors 'none'",
        // The live tail's EventSource.
        "connect-src 'self'",
    ] {
        assert!(csp.contains(directive), "missing {directive}: {csp}");
    }
    // For the heartbeat bars' computed `style="height:Npx"`.
    assert!(csp.contains("style-src 'self' 'unsafe-inline'"), "{csp}");
}

/// Scalar loads from `cdn.jsdelivr.net`, so `/api/docs` skips the CSP rather
/// than widening it for everyone.
#[tokio::test]
async fn the_api_docs_page_is_outside_the_csp_but_not_the_rest() {
    let server = server().await;
    let res = server.get("/api/docs").await;
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
    // The literal `/assets/app.css` route must beat the `/assets/{file}` wildcard.
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
