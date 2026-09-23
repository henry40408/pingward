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
    // CSRF covers only `web`; the sibling routers are structurally exempt.
    // The last layer added runs first: forward_auth_session -> anonymous_session
    // -> csrf_guard -> handler. `csrf_guard` must see a cookie minted on the same
    // request, and an anonymous `Set-Cookie` must not shadow a forward-auth
    // session. `no_store` sits outside them to cover their early returns.
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
        // Web-scoped: `/api/docs` loads a CDN bundle and stays outside the CSP.
        .layer(axum::middleware::from_fn(web::content_security_policy));
    // HSTS describes the whole origin, so it wraps every merged router.
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(web)
        .merge(ping::routes())
        // CSRF-exempt: `/api/v1` is bearer-only (`ApiUser` ignores the cookie)
        // and the cookie-reading docs routes are read-only GETs.
        .merge(api::routes())
        .merge(assets::routes())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            web::hsts,
        ))
        // App-wide, like `hsts`.
        .layer(axum::middleware::from_fn(web::security_headers))
        .with_state(state)
}
