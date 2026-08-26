use crate::state::AppState;
use axum::Router;
use axum::extract::Path;
use axum::http::{StatusCode, Uri, header};
use axum::response::IntoResponse;
use axum::routing::get;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::LazyLock;

const APP_CSS_TEMPLATE: &str = include_str!("../assets/app.css");

/// Substituted with `FONT_VERSION` at startup. Lives inside a quoted CSS
/// `url("…")` string, so `assets/app.css` stays valid CSS on its own.
const FONT_PLACEHOLDER: &str = "{{FONT_V}}";

/// One table so the version hash and the handler cannot disagree about what
/// is served.
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

/// Files rather than inline `<script>` blocks, so the CSP can stay
/// `script-src 'self'` — see `assets/app.js`'s own header.
const SCRIPTS: &[(&str, &str)] = &[
    ("app.js", include_str!("../assets/app.js")),
    ("theme-init.js", include_str!("../assets/theme-init.js")),
];

/// `apple-touch-icon.png` is the 180×180 raster iOS uses for a home-screen
/// bookmark, rendered from `favicon.svg` — regenerate it with
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

/// Safe because every asset URL is content-addressed with a `?v=<hash>`, so
/// none of them ever needs revalidation.
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";

/// Content hash of every embedded font, baked into the stylesheet's font URLs
/// so a font swap invalidates both. Not cryptographic — see `CSS_VERSION`.
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

/// Content hash of the rendered stylesheet, used to cache-bust
/// `/assets/app.css`. Changes exactly when the rendered CSS does, including on
/// a font version bump, since the font URLs are baked into this text. Not
/// cryptographic — collision resistance is irrelevant, and an unstable hash
/// across toolchains only costs one extra fetch.
static CSS_VERSION: LazyLock<String> = LazyLock::new(|| {
    let mut hasher = DefaultHasher::new();
    APP_CSS.as_str().hash(&mut hasher);
    format!("{:x}", hasher.finish())
});

/// One version for the whole set, so editing the SVG and re-rendering the PNG
/// busts both `<link>`s at once.
static ICON_VERSION: LazyLock<String> = LazyLock::new(|| {
    let mut hasher = DefaultHasher::new();
    for (name, _, bytes) in ICONS {
        name.hash(&mut hasher);
        bytes.hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
});

/// One version for the whole set, like [`ICON_VERSION`]: the two files ship
/// together, so a finer grain would not buy anything.
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
        // Served from the root: browsers and iOS probe these exact paths when
        // a page omits the `<link>`.
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

/// Serve whichever icon the request path names. Both icon routes point here so
/// `ICONS` stays the only place a name, MIME type and bytes are tied together.
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

/// Serve whichever script the request path names. `/assets/{file}` also
/// matches `/assets/app.css`, but axum prefers the literal route, so the
/// stylesheet still reaches `app_css` and only unknown names 404 here.
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
