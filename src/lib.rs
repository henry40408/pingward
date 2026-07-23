use axum::{Router, routing::get};
use state::AppState;

pub mod api;
pub mod apikey;
pub mod assets;
pub mod auth;
pub mod config;
pub mod db;
pub mod duration;
pub mod error;
pub mod markdown;
pub mod models;
pub mod notify;
pub mod ping;
pub mod prune;
pub mod scheduler;
pub mod secret;
pub mod shutdown;
pub mod state;
pub mod store;
pub mod view;
pub mod web;

pub fn app(state: AppState) -> Router {
    // CSRF protection applies to the browser-facing `web` router only. The
    // machine `/ping/*` endpoints, static assets, and `/healthz` are merged in
    // as sibling routers and are therefore structurally exempt.
    // Layers run outside-in, so the last one added sees the request first:
    // forward_auth_session -> anonymous_session -> csrf_guard -> handler.
    //
    // Both orderings here are load-bearing. The two session layers run before
    // `csrf_guard` because the guard must see the cookie on the same request
    // that minted it. And `forward_auth_session` runs before
    // `anonymous_session` because when both would mint, the real session has
    // to win — reversed, the anonymous layer's `Set-Cookie` would be appended
    // last and shadow it.
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
        ));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(web)
        .merge(ping::routes())
        // API router: the `/api/v1` data endpoints are bearer-only (`ApiUser`
        // never reads the session cookie). Its `/api/docs` + `/api/openapi.json`
        // routes do read the session cookie, but are read-only `GET`s that
        // change no state, so the whole router stays structurally CSRF-exempt.
        .merge(api::routes())
        .merge(assets::routes())
        .with_state(state)
}
