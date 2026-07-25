//! `Strict-Transport-Security`, gated by `PINGWARD_HSTS_MAX_AGE`
//! (`web::hsts`). Default is off — pingward does not terminate TLS, so
//! sending HSTS unconditionally would be wrong on a plain-HTTP internal
//! deployment. When configured, the header must be app-wide: it covers
//! `/healthz` and static assets, not just the browser-facing `web` router
//! (unlike `Cache-Control: no-store`, which is `web`-only).

use axum_test::TestServer;
use pingward::{app, config::Config, db, state::AppState, store::Store};

async fn server_with(hsts_max_age: Option<&str>) -> TestServer {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let hsts_max_age = hsts_max_age.map(str::to_string);
    let config = Config::from_map(move |k| match k {
        "PINGWARD_HSTS_MAX_AGE" => hsts_max_age.clone(),
        _ => None,
    });
    let state = AppState::new(store, config);
    TestServer::new(app(state))
}

const ROUTES: [&str; 3] = ["/", "/healthz", "/assets/app.css"];

#[tokio::test]
async fn no_header_by_default() {
    let server = server_with(None).await;
    for route in ROUTES {
        let res = server.get(route).await;
        assert!(
            !res.headers().contains_key("strict-transport-security"),
            "{route} must not carry Strict-Transport-Security by default"
        );
    }
}

#[tokio::test]
async fn header_is_app_wide_when_configured() {
    let server = server_with(Some("31536000")).await;
    for route in ROUTES {
        let res = server.get(route).await;
        let value = res
            .header("strict-transport-security")
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(
            value, "max-age=31536000",
            "{route} must carry the configured max-age"
        );
        assert!(
            !value.contains("includeSubDomains"),
            "{route}: includeSubDomains must never be sent"
        );
        assert!(
            !value.contains("preload"),
            "{route}: preload must never be sent"
        );
    }
}
