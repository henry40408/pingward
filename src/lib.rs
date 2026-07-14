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
pub mod web;

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(web::routes())
        .merge(ping::routes())
        .merge(assets::routes())
        .with_state(state)
}
