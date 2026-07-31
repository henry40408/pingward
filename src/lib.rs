use axum::{Router, routing::get};
use state::AppState;

pub mod api;
pub mod apikey;
pub mod assets;
pub mod auth;
pub mod config;
pub mod db;
pub mod duration;
pub mod error;
pub mod markdown;
pub mod models;
pub mod notify;
pub mod ping;
pub mod prune;
pub mod ratelimit;
pub mod scheduler;
pub mod secret;
pub mod shutdown;
pub mod state;
pub mod store;
pub mod view;
pub mod web;

pub fn app(state: AppState) -> Router {
    // CSRF protection applies to the browser-facing `web` router only. The
    // machine `/ping/*` endpoints, static assets, and `/healthz` are merged in
    // as sibling routers and are therefore structurally exempt.
    // Layers run outside-in, so the last one added sees the request first:
    // no_store -> forward_auth_session -> anonymous_session -> csrf_guard ->
    // handler.
    //
    // Both orderings among the session/CSRF layers are load-bearing. The two
    // session layers run before `csrf_guard` because the guard must see the
    // cookie on the same request that minted it. And `forward_auth_session`
    // runs before `anonymous_session` because when both would mint, the real
    // session has to win — reversed, the anonymous layer's `Set-Cookie` would
    // be appended last and shadow it. `no_store` only reads and writes
    // response headers, so it does not participate in that request-ordering
    // chain at all — it sits outermost purely so it covers every early-return
    // path, including `csrf_guard`'s 403s and the session layers' own
    // responses.
    let web = web::routes()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            web::csrf_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            web::anonymous_session,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            web::forward_auth_session,
        ))
        .layer(axum::middleware::from_fn(web::no_store))
        // Web-scoped for the same reason `no_store` is: the CSP describes
        // pages this app renders. `/api/docs` is a sibling router and stays
        // outside it — see `web::content_security_policy`.
        .layer(axum::middleware::from_fn(web::content_security_policy));
    // `web::hsts` is layered here, outside every `.merge(...)`, rather than
    // inside the `web` router the way `no_store` is layered above. `no_store`
    // is a browser-page-caching concern scoped to the `web` router; HSTS tells
    // the browser the whole *origin* is HTTPS-only, so it must also cover
    // `/healthz`, `/ping/*`, `/api/*` and static assets — sibling routers
    // `no_store` never touches. It is a no-op response-only layer like
    // `no_store` (see `web::hsts`'s doc comment for why it defaults off), so
    // its position relative to the request-ordering chain above does not
    // matter.
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(web)
        .merge(ping::routes())
        // API router: the `/api/v1` data endpoints are bearer-only (`ApiUser`
        // never reads the session cookie). Its `/api/docs` + `/api/openapi.json`
        // routes do read the session cookie, but are read-only `GET`s that
        // change no state, so the whole router stays structurally CSRF-exempt.
        .merge(api::routes())
        .merge(assets::routes())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            web::hsts,
        ))
        // App-wide like `hsts`: these are statements about the whole origin,
        // so they also cover the routers the CSP above deliberately skips.
        .layer(axum::middleware::from_fn(web::security_headers))
        .with_state(state)
}
