use axum::{routing::get, Router};
use state::AppState;

pub mod assets;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod models;
pub mod notify;
pub mod ping;
pub mod prune;
pub mod scheduler;
pub mod state;
pub mod store;
pub mod view;
pub mod web;

pub fn app(state: AppState) -> Router {
    // CSRF protection applies to the browser-facing `web` router only. The
    // machine `/ping/*` endpoints, static assets, and `/healthz` are merged in
    // as sibling routers and are therefore structurally exempt.
    let web = web::routes().layer(axum::middleware::from_fn_with_state(
        state.clone(),
        web::csrf_guard,
    ));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(web)
        .merge(ping::routes())
        .merge(assets::routes())
        .with_state(state)
}
