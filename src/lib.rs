use axum::{Router, routing::get};
use state::AppState;

pub mod api;
pub mod apikey;
pub mod assets;
pub mod auth;
pub mod config;
pub mod db;
pub mod duration;
pub mod elevate;
pub mod error;
pub mod markdown;
pub mod models;
pub mod notify;
pub mod ping;
pub mod prune;
pub mod ratelimit;
pub mod scheduler;
pub mod secret;
pub mod shutdown;
pub mod state;
pub mod store;
pub mod view;
pub mod web;

pub fn app(state: AppState) -> Router {
    // CSRF applies to the `web` router only; `/ping/*`, assets and `/healthz`
    // are merged as siblings and are structurally exempt.
    // Layers run outside-in (last added sees the request first): no_store ->
    // forward_auth_session -> anonymous_session -> csrf_guard -> handler.
    // Both session layers must precede `csrf_guard` so it sees the cookie
    // minted on the same request, and `forward_auth_session` must precede
    // `anonymous_session` so the anonymous `Set-Cookie` cannot shadow a real
    // session. `no_store` is response-only, so it sits outermost purely to
    // cover early returns (csrf_guard's 403s, the session layers' responses).
    let web = web::routes()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            web::csrf_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            web::anonymous_session,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            web::forward_auth_session,
        ))
        .layer(axum::middleware::from_fn(web::no_store))
        // Web-scoped like `no_store`: the CSP describes pages this app
        // renders. `/api/docs` is a sibling router and stays outside it —
        // see `web::content_security_policy`.
        .layer(axum::middleware::from_fn(web::content_security_policy));
    // `web::hsts` sits outside every `.merge(...)`: HSTS describes the whole
    // origin, so it must also cover `/healthz`, `/ping/*`, `/api/*` and
    // assets. Response-only, so its position in the chain above is irrelevant.
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(web)
        .merge(ping::routes())
        // `/api/v1` is bearer-only (`ApiUser` never reads the session cookie);
        // `/api/docs` + `/api/openapi.json` do read it but are read-only GETs,
        // so the whole router stays structurally CSRF-exempt.
        .merge(api::routes())
        .merge(assets::routes())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            web::hsts,
        ))
        // App-wide like `hsts`: statements about the whole origin, covering
        // the sibling routers the CSP above skips.
        .layer(axum::middleware::from_fn(web::security_headers))
        .with_state(state)
}
