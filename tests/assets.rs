use axum_test::TestServer;
use pingward::{app, config::Config, db, state::AppState, store::Store};

async fn server() -> TestServer {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store, Config::from_map(|_| None));
    TestServer::new(app(state))
}

#[tokio::test]
async fn serves_stylesheet() {
    let server = server().await;
    let res = server.get("/assets/app.css").await;
    res.assert_status_ok();
    assert_eq!(res.header("content-type"), "text/css; charset=utf-8");
    assert!(res.text().contains("--brand"), "tokens missing from css");
}

#[tokio::test]
async fn unknown_font_file_404s() {
    let server = server().await;
    let res = server.get("/assets/fonts/does-not-exist.woff2").await;
    res.assert_status_not_found();
}

#[tokio::test]
async fn serves_a_font() {
    let server = server().await;
    let res = server.get("/assets/fonts/inter-400.woff2").await;
    res.assert_status_ok();
    assert_eq!(res.header("content-type"), "font/woff2");
}

#[tokio::test]
async fn stylesheet_is_cached_immutably() {
    let server = server().await;
    let res = server.get("/assets/app.css").await;
    res.assert_status_ok();
    assert_eq!(
        res.header("cache-control"),
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn stylesheet_font_urls_are_cache_busted() {
    let server = server().await;
    let res = server.get("/assets/app.css").await;
    res.assert_status_ok();
    let css = res.text();
    // Name-agnostic on purpose: catches a placeholder rename in either
    // `assets/app.css` or `FONT_PLACEHOLDER` that would silently skip
    // substitution and ship a literal placeholder in every font URL.
    assert!(
        !css.contains("{{"),
        "unsubstituted placeholder in the served stylesheet"
    );
    assert!(
        css.contains("/assets/fonts/inter-400.woff2?v="),
        "font URL is not cache-busted"
    );
}

#[tokio::test]
async fn pages_link_the_content_hashed_stylesheet() {
    let server = server().await;
    let res = server.get("/setup").await;
    res.assert_status_ok();
    let version = pingward::assets::css_version();
    assert!(!version.is_empty(), "css version must not be empty");
    let expected = format!("/assets/app.css?v={version}");
    assert!(
        res.text().contains(&expected),
        "versioned stylesheet link missing from rendered page"
    );
}
