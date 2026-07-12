use axum::{routing::get, Router};

pub mod config;
pub mod error;

pub fn app() -> Router {
    Router::new().route("/healthz", get(|| async { "ok" }))
}
