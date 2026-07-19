//! Read-only `/api/v1` handlers. Every resource is resolved through an
//! ownership choke point ([`resolve_project`]/[`resolve_check`]/
//! [`resolve_channel`]) that returns 404 for a resource the caller neither owns
//! nor (as an admin) may reach — hiding existence — and records an audit entry
//! whenever an admin key crosses into another user's data, mirroring the web
//! UI's `admin_*` choke points.

use crate::api::dto::{
    ApiKeyDto, ChannelDto, CheckDto, NotificationDto, NotificationPage, PingDto, PingPage,
    ProjectDto,
};
use crate::api::error::ApiError;
use crate::api::extract::ApiUser;
use crate::models::{Channel, Check, Project, User};
use crate::state::AppState;
use crate::store::{NewAudit, NotifFilter, PageCursor, PingFilter};
use axum::extract::{OriginalUri, Path, Query, State};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use utoipa::IntoParams;

/// Default and maximum page size for the paginated ping/notification lists.
const DEFAULT_PAGE_LIMIT: i64 = 20;
const MAX_PAGE_LIMIT: i64 = 100;

/// Keyset pagination query for the ping/notification lists. `before`/`after`
/// carry a boundary item id (mutually exclusive; `before` wins if both are
/// given). `limit` is clamped to `1..=100`, defaulting to 20.
#[derive(Debug, Deserialize, IntoParams)]
pub struct PageParams {
    /// Return items older than this id (page toward older).
    pub before: Option<i64>,
    /// Return items newer than this id (page back toward newer).
    pub after: Option<i64>,
    /// Page size, clamped to `1..=100` (default 20).
    pub limit: Option<i64>,
}

impl PageParams {
    fn cursor(&self) -> PageCursor {
        match (self.before, self.after) {
            (Some(id), _) => PageCursor::Before(id),
            (None, Some(id)) => PageCursor::After(id),
            (None, None) => PageCursor::Latest,
        }
    }

    fn limit(&self) -> i64 {
        self.limit
            .unwrap_or(DEFAULT_PAGE_LIMIT)
            .clamp(1, MAX_PAGE_LIMIT)
    }
}

/// Record an admin key reaching a resource it does not own. Only called from the
/// admin branch of a resolver, so an admin reading their own data never logs.
async fn audit_cross_user(
    state: &AppState,
    admin: &User,
    target_type: &str,
    target_id: i64,
    owner: Option<i64>,
    path: &str,
) -> Result<(), ApiError> {
    state
        .store
        .record_audit(
            &NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "admin.api.access",
                target_type: Some(target_type),
                target_id: Some(target_id),
                target_owner_id: owner,
                method: Some("GET"),
                path: Some(path),
                detail: None,
            },
            Utc::now(),
        )
        .await?;
    Ok(())
}

/// Resolve a project the caller may read: owner-scope first, else an audited
/// admin cross-user read, else 404 (existence hidden).
async fn resolve_project(
    state: &AppState,
    id: i64,
    user: &User,
    path: &str,
) -> Result<Project, ApiError> {
    let p = state
        .store
        .find_project(id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if p.user_id == user.id {
        return Ok(p);
    }
    if user.is_admin {
        audit_cross_user(state, user, "project", p.id, Some(p.user_id), path).await?;
        return Ok(p);
    }
    Err(ApiError::not_found())
}

/// Resolve a check the caller may read (ownership derived from its project).
async fn resolve_check(
    state: &AppState,
    id: i64,
    user: &User,
    path: &str,
) -> Result<Check, ApiError> {
    let c = state
        .store
        .find_check(id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let owner = state
        .store
        .find_project(c.project_id)
        .await?
        .map(|p| p.user_id);
    if owner == Some(user.id) {
        return Ok(c);
    }
    if user.is_admin {
        audit_cross_user(state, user, "check", c.id, owner, path).await?;
        return Ok(c);
    }
    Err(ApiError::not_found())
}

/// Resolve a channel the caller may read (ownership derived from its project).
async fn resolve_channel(
    state: &AppState,
    id: i64,
    user: &User,
    path: &str,
) -> Result<Channel, ApiError> {
    let ch = state
        .store
        .find_channel(id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let owner = state
        .store
        .find_project(ch.project_id)
        .await?
        .map(|p| p.user_id);
    if owner == Some(user.id) {
        return Ok(ch);
    }
    if user.is_admin {
        audit_cross_user(state, user, "channel", ch.id, owner, path).await?;
        return Ok(ch);
    }
    Err(ApiError::not_found())
}

/// List the caller's own projects.
#[utoipa::path(
    get, path = "/api/v1/projects", tag = "projects",
    security(("api_key" = [])),
    responses((status = 200, description = "The caller's projects", body = [ProjectDto]))
)]
pub async fn list_projects(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
) -> Result<Json<Vec<ProjectDto>>, ApiError> {
    let projects = state.store.list_projects_for_user(user.id).await?;
    Ok(Json(projects.into_iter().map(ProjectDto::from).collect()))
}

/// Get one project by id.
#[utoipa::path(
    get, path = "/api/v1/projects/{id}", tag = "projects",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Project id")),
    responses(
        (status = 200, description = "The project", body = ProjectDto),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn get_project(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<Json<ProjectDto>, ApiError> {
    let p = resolve_project(&state, id, &user, uri.path()).await?;
    Ok(Json(p.into()))
}

/// List the checks in a project.
#[utoipa::path(
    get, path = "/api/v1/projects/{id}/checks", tag = "projects",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Project id")),
    responses(
        (status = 200, description = "The project's checks", body = [CheckDto]),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn list_project_checks(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<Json<Vec<CheckDto>>, ApiError> {
    let p = resolve_project(&state, id, &user, uri.path()).await?;
    let checks = state.store.list_checks_for_project(p.id).await?;
    Ok(Json(checks.into_iter().map(CheckDto::from).collect()))
}

/// List the notification channels in a project.
#[utoipa::path(
    get, path = "/api/v1/projects/{id}/channels", tag = "projects",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Project id")),
    responses(
        (status = 200, description = "The project's channels", body = [ChannelDto]),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn list_project_channels(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<Json<Vec<ChannelDto>>, ApiError> {
    let p = resolve_project(&state, id, &user, uri.path()).await?;
    let channels = state.store.list_channels_for_project(p.id).await?;
    Ok(Json(channels.into_iter().map(ChannelDto::from).collect()))
}

/// Get one check by id.
#[utoipa::path(
    get, path = "/api/v1/checks/{id}", tag = "checks",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Check id")),
    responses(
        (status = 200, description = "The check", body = CheckDto),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn get_check(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<Json<CheckDto>, ApiError> {
    let c = resolve_check(&state, id, &user, uri.path()).await?;
    Ok(Json(c.into()))
}

/// List a check's pings, newest-first, keyset-paginated.
#[utoipa::path(
    get, path = "/api/v1/checks/{id}/pings", tag = "checks",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Check id"), PageParams),
    responses(
        (status = 200, description = "A page of pings", body = PingPage),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn list_check_pings(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
    Query(params): Query<PageParams>,
) -> Result<Json<PingPage>, ApiError> {
    let c = resolve_check(&state, id, &user, uri.path()).await?;
    let page = state
        .store
        .list_pings_page(
            c.id,
            params.cursor(),
            params.limit(),
            &PingFilter::default(),
        )
        .await?;
    Ok(Json(PingPage {
        items: page.items.into_iter().map(PingDto::from).collect(),
        has_newer: page.has_newer,
        has_older: page.has_older,
    }))
}

/// List a check's notifications, newest-first, keyset-paginated.
#[utoipa::path(
    get, path = "/api/v1/checks/{id}/notifications", tag = "checks",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Check id"), PageParams),
    responses(
        (status = 200, description = "A page of notifications", body = NotificationPage),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn list_check_notifications(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
    Query(params): Query<PageParams>,
) -> Result<Json<NotificationPage>, ApiError> {
    let c = resolve_check(&state, id, &user, uri.path()).await?;
    let page = state
        .store
        .list_notifications_page(
            c.id,
            params.cursor(),
            params.limit(),
            &NotifFilter::default(),
        )
        .await?;
    Ok(Json(NotificationPage {
        items: page.items.into_iter().map(NotificationDto::from).collect(),
        has_newer: page.has_newer,
        has_older: page.has_older,
    }))
}

/// List the caller's own API keys (metadata only — the secret token is never
/// returned). Always self-scoped: an admin key sees only its owner's keys.
#[utoipa::path(
    get, path = "/api/v1/keys", tag = "keys",
    security(("api_key" = [])),
    responses((status = 200, description = "The caller's API keys", body = [ApiKeyDto]))
)]
pub async fn list_keys(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
) -> Result<Json<Vec<ApiKeyDto>>, ApiError> {
    let keys = state.store.list_api_keys_for_user(user.id).await?;
    Ok(Json(keys.into_iter().map(ApiKeyDto::from).collect()))
}

/// Get one channel by id (secrets in its config are never returned).
#[utoipa::path(
    get, path = "/api/v1/channels/{id}", tag = "channels",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Channel id")),
    responses(
        (status = 200, description = "The channel", body = ChannelDto),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn get_channel(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<Json<ChannelDto>, ApiError> {
    let ch = resolve_channel(&state, id, &user, uri.path()).await?;
    Ok(Json(ch.into()))
}
