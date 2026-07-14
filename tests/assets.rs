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
