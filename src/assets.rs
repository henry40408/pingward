use crate::state::AppState;
use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::LazyLock;

const APP_CSS_TEMPLATE: &str = include_str!("../assets/app.css");

/// Substituted with `FONT_VERSION` at startup. Lives inside a quoted CSS
/// `url("…")` string, so `assets/app.css` stays valid CSS on its own.
const FONT_PLACEHOLDER: &str = "{{FONT_V}}";

/// Every embedded font. One table so the version hash and the handler can
/// never disagree about what is served.
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

/// Both assets are content-addressed — `app.css` via `?v=<css hash>`, and the
/// font URLs via `?v=<font hash>` baked into that stylesheet — so neither ever
/// needs revalidation.
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";

/// Content hash of every embedded font, baked into the stylesheet's font URLs
/// so a font swap invalidates both the font and the stylesheet that points at
/// it. Not cryptographic — see `CSS_VERSION`.
static FONT_VERSION: LazyLock<String> = LazyLock::new(|| {
    let mut hasher = DefaultHasher::new();
    for (name, bytes) in FONTS {
        name.hash(&mut hasher);
        bytes.hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
});

/// The stylesheet as served: the font-URL placeholder resolved to the current
/// font version.
static APP_CSS: LazyLock<String> =
    LazyLock::new(|| APP_CSS_TEMPLATE.replace(FONT_PLACEHOLDER, FONT_VERSION.as_str()));

/// Content hash of the rendered stylesheet, used to cache-bust
/// `/assets/app.css`. The URL changes exactly when the rendered CSS content
/// changes — which includes a font version bump, since the font URLs are
/// baked into this text — which lets the response be cached immutably. Not
/// cryptographic — collision resistance is irrelevant here, and an unstable
/// hash across toolchains only ever costs one extra fetch.
static CSS_VERSION: LazyLock<String> = LazyLock::new(|| {
    let mut hasher = DefaultHasher::new();
    APP_CSS.as_str().hash(&mut hasher);
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
        APP_CSS.as_str(),
    )
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
