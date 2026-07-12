use axum::{routing::get, Router};
use store::Store;

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod models;
pub mod notify;
pub mod ping;
pub mod scheduler;
pub mod store;

pub fn app(store: Store) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(ping::routes())
        .with_state(store)
}
