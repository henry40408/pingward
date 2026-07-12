use axum::{routing::get, Router};

pub mod config;
pub mod db;
pub mod error;
pub mod models;
pub mod notify;
pub mod scheduler;
pub mod store;

pub fn app() -> Router {
    Router::new().route("/healthz", get(|| async { "ok" }))
}
