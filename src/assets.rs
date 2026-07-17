use crate::state::AppState;
use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::LazyLock;

const APP_CSS: &str = include_str!("../assets/app.css");

/// Assets are content-addressed (`app.css` via `?v=<hash>`) or never change
/// (fonts), so both can be cached for a year without revalidation.
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";

/// Content hash of the stylesheet, used to cache-bust `/assets/app.css`.
/// The URL changes exactly when the CSS content changes, which lets the
/// response be cached immutably. Not cryptographic — collision resistance is
/// irrelevant here, and an unstable hash across toolchains only ever costs one
/// extra fetch.
static CSS_VERSION: LazyLock<String> = LazyLock::new(|| {
    let mut hasher = DefaultHasher::new();
    APP_CSS.hash(&mut hasher);
    format!("{:x}", hasher.finish())
});

pub fn css_version() -> &'static str {
    CSS_VERSION.as_str()
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/assets/app.css", get(app_css))
        .route("/assets/fonts/{file}", get(font))
}

async fn app_css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, IMMUTABLE_CACHE),
        ],
        APP_CSS,
    )
}

async fn font(Path(file): Path<String>) -> impl IntoResponse {
    let bytes: Option<&'static [u8]> = match file.as_str() {
        "inter-400.woff2" => Some(include_bytes!("../assets/fonts/inter-400.woff2")),
        "inter-500.woff2" => Some(include_bytes!("../assets/fonts/inter-500.woff2")),
        "inter-600.woff2" => Some(include_bytes!("../assets/fonts/inter-600.woff2")),
        "inter-700.woff2" => Some(include_bytes!("../assets/fonts/inter-700.woff2")),
        "ibm-plex-mono-400.woff2" => {
            Some(include_bytes!("../assets/fonts/ibm-plex-mono-400.woff2"))
        }
        "ibm-plex-mono-500.woff2" => {
            Some(include_bytes!("../assets/fonts/ibm-plex-mono-500.woff2"))
        }
        "ibm-plex-mono-600.woff2" => {
            Some(include_bytes!("../assets/fonts/ibm-plex-mono-600.woff2"))
        }
        _ => None,
    };
    match bytes {
        Some(b) => (
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, IMMUTABLE_CACHE),
            ],
            b,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
