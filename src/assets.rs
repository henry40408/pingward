use crate::state::AppState;
use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

const APP_CSS: &str = include_str!("../assets/app.css");

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/assets/app.css", get(app_css))
        .route("/assets/fonts/{file}", get(font))
}

async fn app_css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        APP_CSS,
    )
}

async fn font(Path(file): Path<String>) -> impl IntoResponse {
    // Vendored in Task 2. Until then, unknown files 404 cleanly.
    #[allow(clippy::match_single_binding)] // arms populated when fonts land
    let bytes: Option<&'static [u8]> = match file.as_str() {
        _ => None,
    };
    match bytes {
        Some(b) => (
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            b,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
