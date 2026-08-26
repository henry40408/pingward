//! The programmatic REST API: a bearer-authenticated `/api/v1` surface (reads
//! and writes) plus its `OpenAPI` document and Scalar reference UI.
//!
//! Mounted in [`crate::app`] as a sibling router outside `csrf_guard`: every
//! `/api/v1` handler authenticates via the [`extract::ApiUser`] bearer
//! extractor and never reads the session cookie. `/api/docs` and
//! `/api/openapi.json` do read it ([`CurrentUser`]), but are read-only `GET`s,
//! so there is no ambient authority for a cross-site request to abuse. They
//! are session-authenticated responses, so [`crate::web::no_store`] is layered
//! around just those two — `/api/v1` is bearer-only and changing its headers
//! would only affect API consumers.

pub mod dto;
pub mod error;
pub mod extract;
pub mod input;
pub mod v1;

use crate::auth::CurrentUser;
use crate::state::AppState;
use axum::response::Html;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use utoipa::OpenApi;
use utoipa_scalar::Scalar;

/// The `OpenAPI` document for the `/api/v1` surface. Every operation is
/// bearer-authenticated (`api_key` security scheme).
#[derive(OpenApi)]
#[openapi(
    info(
        title = "pingward API",
        description = "Programmatic access to pingward projects, checks, channels, and history. Authenticate with an API key: `Authorization: Bearer pw_…`.",
        version = "1.0.0"
    ),
    paths(
        v1::list_projects,
        v1::get_project,
        v1::list_project_checks,
        v1::list_project_channels,
        v1::get_check,
        v1::list_check_pings,
        v1::list_check_notifications,
        v1::get_channel,
        v1::list_keys,
        v1::create_project,
        v1::update_project,
        v1::delete_project,
        v1::create_check,
        v1::update_check,
        v1::delete_check,
        v1::pause_check,
        v1::resume_check,
        v1::ack_check,
        v1::regenerate_check,
        v1::set_check_channels,
        v1::create_channel,
        v1::update_channel,
        v1::delete_channel,
    ),
    components(schemas(
        dto::ProjectDto,
        dto::CheckDto,
        dto::ChannelDto,
        dto::PingDto,
        dto::NotificationDto,
        dto::ApiKeyDto,
        dto::PingPage,
        dto::NotificationPage,
        dto::BoundChannels,
        input::ProjectInput,
        input::CheckInput,
        input::ChannelInput,
        input::ChannelBindInput,
        error::ApiErrorInner,
    )),
    modifiers(&BearerAuth),
    tags(
        (name = "projects", description = "Projects and their checks/channels"),
        (name = "checks", description = "Checks and their ping/notification history"),
        (name = "channels", description = "Notification channels"),
        (name = "keys", description = "The caller's own API keys"),
    )
)]
struct ApiDoc;

/// Registers the `api_key` bearer security scheme referenced by every path.
struct BearerAuth;

impl utoipa::Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "api_key",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some("pingward API key: `Authorization: Bearer pw_…`"))
                    .build(),
            ),
        );
    }
}

/// Gated behind a logged-in web session ([`CurrentUser`]): the reference
/// describes the surface but is not itself public. `/api/v1` keeps its
/// independent bearer authentication.
async fn openapi_json(_user: CurrentUser) -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

/// Serve the interactive Scalar API reference. Gated behind a logged-in web
/// session for the same reason as [`openapi_json`].
async fn scalar_docs(_user: CurrentUser) -> Html<String> {
    Html(Scalar::new(ApiDoc::openapi()).to_html())
}

/// A sub-router purely so `no_store` covers just these two
/// session-authenticated routes, without touching `/api/v1`'s headers.
fn docs_routes() -> Router<AppState> {
    Router::new()
        .route("/api/openapi.json", get(openapi_json))
        .route("/api/docs", get(scalar_docs))
        .layer(axum::middleware::from_fn(crate::web::no_store))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/projects",
            get(v1::list_projects).post(v1::create_project),
        )
        .route(
            "/api/v1/projects/{id}",
            get(v1::get_project)
                .patch(v1::update_project)
                .delete(v1::delete_project),
        )
        .route(
            "/api/v1/projects/{id}/checks",
            get(v1::list_project_checks).post(v1::create_check),
        )
        .route(
            "/api/v1/projects/{id}/channels",
            get(v1::list_project_channels).post(v1::create_channel),
        )
        .route(
            "/api/v1/checks/{id}",
            get(v1::get_check)
                .patch(v1::update_check)
                .delete(v1::delete_check),
        )
        .route("/api/v1/checks/{id}/pings", get(v1::list_check_pings))
        .route(
            "/api/v1/checks/{id}/notifications",
            get(v1::list_check_notifications),
        )
        .route("/api/v1/checks/{id}/pause", post(v1::pause_check))
        .route("/api/v1/checks/{id}/resume", post(v1::resume_check))
        .route("/api/v1/checks/{id}/ack", post(v1::ack_check))
        .route("/api/v1/checks/{id}/regenerate", post(v1::regenerate_check))
        .route("/api/v1/checks/{id}/channels", put(v1::set_check_channels))
        .route(
            "/api/v1/channels/{id}",
            get(v1::get_channel)
                .patch(v1::update_channel)
                .delete(v1::delete_channel),
        )
        .route("/api/v1/keys", get(v1::list_keys))
        .merge(docs_routes())
}
