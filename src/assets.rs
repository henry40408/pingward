use crate::state::AppState;
use axum::Router;
use axum::extract::Path;
use axum::http::{StatusCode, Uri, header};
use axum::response::IntoResponse;
use axum::routing::get;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::LazyLock;

const APP_CSS_TEMPLATE: &str = include_str!("../assets/app.css");

/// Replaced by `FONT_VERSION` at startup; sits inside a quoted `url("…")` so
/// `assets/app.css` stays valid CSS.
const FONT_PLACEHOLDER: &str = "{{FONT_V}}";

/// One table, so the version hash and the handler agree on what is served.
const FONTS: &[(&str, &[u8])] = &[
    (
        "inter-400.woff2",
        include_bytes!("../assets/fonts/inter-400.woff2"),
    ),
    (
        "inter-500.woff2",
        include_bytes!("../assets/fonts/inter-500.woff2"),
    ),
    (
        "inter-600.woff2",
        include_bytes!("../assets/fonts/inter-600.woff2"),
    ),
    (
        "inter-700.woff2",
        include_bytes!("../assets/fonts/inter-700.woff2"),
    ),
    (
        "ibm-plex-mono-400.woff2",
        include_bytes!("../assets/fonts/ibm-plex-mono-400.woff2"),
    ),
    (
        "ibm-plex-mono-500.woff2",
        include_bytes!("../assets/fonts/ibm-plex-mono-500.woff2"),
    ),
    (
        "ibm-plex-mono-600.woff2",
        include_bytes!("../assets/fonts/ibm-plex-mono-600.woff2"),
    ),
];

/// Files, not inline `<script>`, so the CSP can stay `script-src 'self'`.
const SCRIPTS: &[(&str, &str)] = &[
    ("app.js", include_str!("../assets/app.js")),
    ("theme-init.js", include_str!("../assets/theme-init.js")),
];

/// `apple-touch-icon.png` is rendered from `favicon.svg`: rerun
/// `cargo run --bin icons` in `e2e/` after editing the SVG.
const ICONS: &[(&str, &str, &[u8])] = &[
    (
        "favicon.svg",
        "image/svg+xml",
        include_bytes!("../assets/favicon.svg"),
    ),
    (
        "apple-touch-icon.png",
        "image/png",
        include_bytes!("../assets/apple-touch-icon.png"),
    ),
];

/// Safe because every asset URL carries a content hash (`?v=<hash>`).
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";

/// Hash of every embedded font, baked into the stylesheet's font URLs.
static FONT_VERSION: LazyLock<String> = LazyLock::new(|| {
    let mut hasher = DefaultHasher::new();
    for (name, bytes) in FONTS {
        name.hash(&mut hasher);
        bytes.hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
});

/// The stylesheet as served, with the font-URL placeholder resolved.
static APP_CSS: LazyLock<String> =
    LazyLock::new(|| APP_CSS_TEMPLATE.replace(FONT_PLACEHOLDER, FONT_VERSION.as_str()));

/// Hash of the rendered stylesheet (so it also changes with the fonts). These
/// hashes are not cryptographic; instability across toolchains costs one refetch.
static CSS_VERSION: LazyLock<String> = LazyLock::new(|| {
    let mut hasher = DefaultHasher::new();
    APP_CSS.as_str().hash(&mut hasher);
    format!("{:x}", hasher.finish())
});

/// One version for all icons, since the PNG is rendered from the SVG.
static ICON_VERSION: LazyLock<String> = LazyLock::new(|| {
    let mut hasher = DefaultHasher::new();
    for (name, _, bytes) in ICONS {
        name.hash(&mut hasher);
        bytes.hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
});

/// One version for both scripts, which ship together.
static JS_VERSION: LazyLock<String> = LazyLock::new(|| {
    let mut hasher = DefaultHasher::new();
    for (name, body) in SCRIPTS {
        name.hash(&mut hasher);
        body.hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
});

pub fn css_version() -> &'static str {
    CSS_VERSION.as_str()
}

pub fn js_version() -> &'static str {
    JS_VERSION.as_str()
}

pub fn icon_version() -> &'static str {
    ICON_VERSION.as_str()
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/assets/app.css", get(app_css))
        .route("/assets/{file}", get(script))
        .route("/assets/fonts/{file}", get(font))
        // At the root: browsers and iOS probe these paths without a `<link>`.
        .route("/favicon.svg", get(icon))
        .route("/apple-touch-icon.png", get(icon))
}

async fn app_css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, IMMUTABLE_CACHE),
        ],
        APP_CSS.as_str(),
    )
}

/// Serve the icon the path names, keeping `ICONS` the single source of truth.
async fn icon(uri: Uri) -> impl IntoResponse {
    let name = uri.path().trim_start_matches('/');
    match ICONS.iter().find(|(n, _, _)| *n == name) {
        Some((_, mime, bytes)) => (
            [
                (header::CONTENT_TYPE, *mime),
                (header::CACHE_CONTROL, IMMUTABLE_CACHE),
            ],
            *bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Serve the script the path names. axum prefers the literal `/assets/app.css`
/// route, so the stylesheet never lands here.
async fn script(Path(file): Path<String>) -> impl IntoResponse {
    match SCRIPTS.iter().find(|(name, _)| *name == file) {
        Some((_, body)) => (
            [
                (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
                (header::CACHE_CONTROL, IMMUTABLE_CACHE),
            ],
            *body,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn font(Path(file): Path<String>) -> impl IntoResponse {
    match FONTS.iter().find(|(name, _)| *name == file) {
        Some((_, bytes)) => (
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, IMMUTABLE_CACHE),
            ],
            *bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
