//! `/api/v1` handlers (reads and writes). Every resource is resolved through an
//! ownership choke point ([`resolve_project`]/[`resolve_check`]/
//! [`resolve_channel`]) that returns 404 for a resource the caller neither owns
//! nor (as an admin) may reach — hiding existence — and records an audit entry
//! whenever an admin key crosses into another user's data, mirroring the web
//! UI's `admin_*` choke points. Writes reuse the web UI's own validators
//! ([`crate::web::validate_project`]/[`crate::web::validate_check`]/
//! [`crate::web::validate_channel`]) so the API and the browser agree on what a
//! valid resource is.

use crate::api::dto::{
    ApiKeyDto, BoundChannels, ChannelDto, CheckDto, NotificationPage, PingPage, ProjectDto,
};
use crate::api::error::ApiError;
use crate::api::extract::{ApiJson, ApiUser};
use crate::api::input::{ChannelBindInput, ChannelInput, CheckInput, ProjectInput};
use crate::models::{Channel, Check, CheckStatus, Project, User};
use crate::state::AppState;
use crate::store::{NewAudit, NewCheck, NotifFilter, PageCursor, PingFilter, UpdateCheck};
use crate::web::{
    ChannelForm, CheckForm, ProjectForm, validate_channel, validate_check, validate_project,
};
use axum::Json;
use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashSet;
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
/// admin branch of a resolver, so an admin touching their own data never logs.
/// `method` is the request verb (`GET` for reads, `POST`/`PATCH`/`DELETE`/`PUT`
/// for writes) so the audit trail distinguishes a cross-user read from a write.
async fn audit_cross_user(
    state: &AppState,
    admin: &User,
    target_type: &str,
    target_id: i64,
    owner: Option<i64>,
    method: &str,
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
                method: Some(method),
                path: Some(path),
                detail: None,
            },
            Utc::now(),
        )
        .await?;
    Ok(())
}

/// Resolve a project the caller may act on: owner-scope first, else an audited
/// admin cross-user access, else 404 (existence hidden). `method` labels the
/// audit entry (the caller's HTTP verb).
async fn resolve_project(
    state: &AppState,
    id: i64,
    user: &User,
    method: &str,
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
        audit_cross_user(state, user, "project", p.id, Some(p.user_id), method, path).await?;
        return Ok(p);
    }
    Err(ApiError::not_found())
}

/// Resolve a check the caller may act on (ownership derived from its project).
async fn resolve_check(
    state: &AppState,
    id: i64,
    user: &User,
    method: &str,
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
        audit_cross_user(state, user, "check", c.id, owner, method, path).await?;
        return Ok(c);
    }
    Err(ApiError::not_found())
}

/// Resolve a channel the caller may act on (ownership derived from its project).
async fn resolve_channel(
    state: &AppState,
    id: i64,
    user: &User,
    method: &str,
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
        audit_cross_user(state, user, "channel", ch.id, owner, method, path).await?;
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
    let p = resolve_project(&state, id, &user, "GET", uri.path()).await?;
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
    let p = resolve_project(&state, id, &user, "GET", uri.path()).await?;
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
    let p = resolve_project(&state, id, &user, "GET", uri.path()).await?;
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
    let c = resolve_check(&state, id, &user, "GET", uri.path()).await?;
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
    let c = resolve_check(&state, id, &user, "GET", uri.path()).await?;
    let page = state
        .store
        .list_pings_page(
            c.id,
            params.cursor(),
            params.limit(),
            &PingFilter::default(),
        )
        .await?;
    Ok(Json(PingPage::from_page(page)))
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
    let c = resolve_check(&state, id, &user, "GET", uri.path()).await?;
    let page = state
        .store
        .list_notifications_page(
            c.id,
            params.cursor(),
            params.limit(),
            &NotifFilter::default(),
        )
        .await?;
    Ok(Json(NotificationPage::from_page(page)))
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
    let ch = resolve_channel(&state, id, &user, "GET", uri.path()).await?;
    Ok(Json(ch.into()))
}

// ----------------------------------------------------------------------------
// Writes
// ----------------------------------------------------------------------------

/// Re-fetch a check and map it to its DTO. Used by the action endpoints
/// (pause/resume/ack/regenerate) to return the check's new state.
async fn reload_check(state: &AppState, id: i64) -> Result<Json<CheckDto>, ApiError> {
    let c = state
        .store
        .find_check(id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(c.into()))
}

/// Create a project owned by the caller.
#[utoipa::path(
    post, path = "/api/v1/projects", tag = "projects",
    security(("api_key" = [])),
    request_body = ProjectInput,
    responses(
        (status = 201, description = "Created", body = ProjectDto),
        (status = 400, description = "Invalid input", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn create_project(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    ApiJson(input): ApiJson<ProjectInput>,
) -> Result<(StatusCode, Json<ProjectDto>), ApiError> {
    let form: ProjectForm = input.into();
    let (name, scan, nag) = validate_project(&form).map_err(ApiError::bad_request)?;
    let id = state
        .store
        .create_project(user.id, &name, scan, nag, Utc::now())
        .await?;
    let p = state
        .store
        .find_project(id)
        .await?
        .ok_or_else(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(p.into())))
}

/// Replace a project's editable fields (name + overrides). Send the full
/// representation — this is not a partial patch.
#[utoipa::path(
    patch, path = "/api/v1/projects/{id}", tag = "projects",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Project id")),
    request_body = ProjectInput,
    responses(
        (status = 200, description = "Updated", body = ProjectDto),
        (status = 400, description = "Invalid input", body = crate::api::error::ApiErrorInner),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn update_project(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
    ApiJson(input): ApiJson<ProjectInput>,
) -> Result<Json<ProjectDto>, ApiError> {
    resolve_project(&state, id, &user, "PATCH", uri.path()).await?;
    let form: ProjectForm = input.into();
    let (name, scan, nag) = validate_project(&form).map_err(ApiError::bad_request)?;
    state.store.update_project(id, &name, scan, nag).await?;
    let p = state
        .store
        .find_project(id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(p.into()))
}

/// Delete a project and everything under it (checks, channels, history).
#[utoipa::path(
    delete, path = "/api/v1/projects/{id}", tag = "projects",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Project id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn delete_project(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    resolve_project(&state, id, &user, "DELETE", uri.path()).await?;
    state.store.delete_project(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Create a check under a project. A fresh ping UUID is generated server-side.
#[utoipa::path(
    post, path = "/api/v1/projects/{id}/checks", tag = "checks",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Project id")),
    request_body = CheckInput,
    responses(
        (status = 201, description = "Created", body = CheckDto),
        (status = 400, description = "Invalid input", body = crate::api::error::ApiErrorInner),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn create_check(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(pid): Path<i64>,
    ApiJson(input): ApiJson<CheckInput>,
) -> Result<(StatusCode, Json<CheckDto>), ApiError> {
    resolve_project(&state, pid, &user, "POST", uri.path()).await?;
    let form: CheckForm = input.into();
    let v = validate_check(&form).map_err(ApiError::bad_request)?;
    let uuid = uuid::Uuid::new_v4().to_string();
    let id = state
        .store
        .create_check(&NewCheck {
            project_id: pid,
            name: &v.name,
            ping_uuid: &uuid,
            kind: v.kind,
            period_secs: v.period_secs,
            grace_secs: v.grace,
            cron_expr: v.cron_expr.as_deref(),
            timezone: &form.timezone,
            scan_interval_secs: v.scan_interval_secs,
            max_runtime_secs: v.max_runtime_secs,
            nag_interval_secs: v.nag_interval_secs,
        })
        .await?;
    let c = state
        .store
        .find_check(id)
        .await?
        .ok_or_else(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(c.into())))
}

/// Replace a check's editable fields (schedule + grace + overrides). Send the
/// full representation — this is not a partial patch.
#[utoipa::path(
    patch, path = "/api/v1/checks/{id}", tag = "checks",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Check id")),
    request_body = CheckInput,
    responses(
        (status = 200, description = "Updated", body = CheckDto),
        (status = 400, description = "Invalid input", body = crate::api::error::ApiErrorInner),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn update_check(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
    ApiJson(input): ApiJson<CheckInput>,
) -> Result<Json<CheckDto>, ApiError> {
    resolve_check(&state, id, &user, "PATCH", uri.path()).await?;
    let form: CheckForm = input.into();
    let v = validate_check(&form).map_err(ApiError::bad_request)?;
    state
        .store
        .update_check_schedule(
            id,
            &UpdateCheck {
                name: &v.name,
                kind: v.kind,
                period_secs: v.period_secs,
                grace_secs: v.grace,
                cron_expr: v.cron_expr.as_deref(),
                timezone: &form.timezone,
                scan_interval_secs: v.scan_interval_secs,
                max_runtime_secs: v.max_runtime_secs,
                nag_interval_secs: v.nag_interval_secs,
            },
        )
        .await?;
    reload_check(&state, id).await
}

/// Delete a check and its history.
#[utoipa::path(
    delete, path = "/api/v1/checks/{id}", tag = "checks",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Check id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn delete_check(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    resolve_check(&state, id, &user, "DELETE", uri.path()).await?;
    state.store.delete_check(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Pause a check (suspends scheduling and notifications).
#[utoipa::path(
    post, path = "/api/v1/checks/{id}/pause", tag = "checks",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Check id")),
    responses(
        (status = 200, description = "Paused", body = CheckDto),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn pause_check(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<Json<CheckDto>, ApiError> {
    resolve_check(&state, id, &user, "POST", uri.path()).await?;
    state.store.set_status(id, CheckStatus::Paused).await?;
    reload_check(&state, id).await
}

/// Resume a paused check (returns it to the `new` state; the next ping or scan
/// re-establishes its status).
#[utoipa::path(
    post, path = "/api/v1/checks/{id}/resume", tag = "checks",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Check id")),
    responses(
        (status = 200, description = "Resumed", body = CheckDto),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn resume_check(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<Json<CheckDto>, ApiError> {
    resolve_check(&state, id, &user, "POST", uri.path()).await?;
    state.store.set_status(id, CheckStatus::New).await?;
    reload_check(&state, id).await
}

/// Acknowledge a check's current down incident (silences its reminders until
/// the next status change).
#[utoipa::path(
    post, path = "/api/v1/checks/{id}/ack", tag = "checks",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Check id")),
    responses(
        (status = 200, description = "Acknowledged", body = CheckDto),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn ack_check(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<Json<CheckDto>, ApiError> {
    resolve_check(&state, id, &user, "POST", uri.path()).await?;
    state.store.acknowledge(id).await?;
    reload_check(&state, id).await
}

/// Regenerate a check's ping UUID (invalidates the old ping URL).
#[utoipa::path(
    post, path = "/api/v1/checks/{id}/regenerate", tag = "checks",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Check id")),
    responses(
        (status = 200, description = "Regenerated", body = CheckDto),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn regenerate_check(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<Json<CheckDto>, ApiError> {
    resolve_check(&state, id, &user, "POST", uri.path()).await?;
    state
        .store
        .regenerate_uuid(id, &uuid::Uuid::new_v4().to_string())
        .await?;
    reload_check(&state, id).await
}

/// Replace the set of channels bound to a check. Ids that do not belong to the
/// check's own project are ignored. Returns the resulting bound-channel ids.
#[utoipa::path(
    put, path = "/api/v1/checks/{id}/channels", tag = "checks",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Check id")),
    request_body = ChannelBindInput,
    responses(
        (status = 200, description = "The check's bound channels", body = BoundChannels),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn set_check_channels(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
    ApiJson(input): ApiJson<ChannelBindInput>,
) -> Result<Json<BoundChannels>, ApiError> {
    let check = resolve_check(&state, id, &user, "PUT", uri.path()).await?;
    let valid: HashSet<i64> = state
        .store
        .list_channels_for_project(check.project_id)
        .await?
        .into_iter()
        .map(|c| c.id)
        .collect();
    let current: HashSet<i64> = state
        .store
        .bound_channel_ids(id)
        .await?
        .into_iter()
        .collect();
    let desired: HashSet<i64> = input
        .channel_ids
        .into_iter()
        .filter(|c| valid.contains(c))
        .collect();
    for add in desired.difference(&current) {
        state.store.bind_channel(id, *add).await?;
    }
    for remove in current.difference(&desired) {
        state.store.unbind_channel(id, *remove).await?;
    }
    let channel_ids = state.store.bound_channel_ids(id).await?;
    Ok(Json(BoundChannels { channel_ids }))
}

/// Create a notification channel in a project.
#[utoipa::path(
    post, path = "/api/v1/projects/{id}/channels", tag = "channels",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Project id")),
    request_body = ChannelInput,
    responses(
        (status = 201, description = "Created", body = ChannelDto),
        (status = 400, description = "Invalid input", body = crate::api::error::ApiErrorInner),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn create_channel(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(pid): Path<i64>,
    ApiJson(input): ApiJson<ChannelInput>,
) -> Result<(StatusCode, Json<ChannelDto>), ApiError> {
    resolve_project(&state, pid, &user, "POST", uri.path()).await?;
    let form: ChannelForm = input.into();
    let (kind, name, config) = validate_channel(&form).map_err(ApiError::bad_request)?;
    let id = state
        .store
        .create_channel(pid, kind, &name, &config, Utc::now())
        .await?;
    let ch = state
        .store
        .find_channel(id)
        .await?
        .ok_or_else(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(ch.into())))
}

/// Delete a notification channel (also unbinds it from every check).
#[utoipa::path(
    delete, path = "/api/v1/channels/{id}", tag = "channels",
    security(("api_key" = [])),
    params(("id" = i64, Path, description = "Channel id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "Not found", body = crate::api::error::ApiErrorInner)
    )
)]
pub async fn delete_channel(
    State(state): State<AppState>,
    ApiUser(user): ApiUser,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    resolve_channel(&state, id, &user, "DELETE", uri.path()).await?;
    state.store.delete_channel(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
