use crate::auth::{
    hash_password, new_session_token, verify_password, AdminUser, CurrentUser, OptionalUser,
    SESSION_COOKIE, SESSION_TTL_DAYS,
};
use crate::error::AppError;
use crate::models::{
    Channel, ChannelKind, Check, CheckStatus, Notification, Project, ScheduleKind, User,
};
use crate::state::AppState;
use crate::store::Store;
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use axum_extra::extract::Form as HtmlForm;
use chrono::{Duration, Utc};
use cron::Schedule;
use serde::Deserialize;
use std::str::FromStr;

pub fn render<T: Template>(t: &T) -> Result<Html<String>, AppError> {
    let body = t.render().map_err(|e| AppError::Other(Box::new(e)))?;
    Ok(Html(body))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", axum::routing::get(dashboard))
        .route("/setup", axum::routing::get(setup_page).post(setup_submit))
        .route("/login", axum::routing::get(login_page).post(login_submit))
        .route("/logout", post(logout))
        .route("/projects/new", get(project_new))
        .route("/projects", post(project_create))
        .route("/projects/{id}", get(project_show).post(project_update))
        .route("/projects/{id}/edit", get(project_edit))
        .route("/projects/{id}/delete", post(project_delete))
        .route("/projects/{pid}/checks/new", get(check_new))
        .route("/projects/{pid}/checks", post(check_create))
        .route("/checks/{id}", get(check_show).post(check_update))
        .route("/checks/{id}/edit", get(check_edit))
        .route("/checks/{id}/pause", post(check_pause))
        .route("/checks/{id}/resume", post(check_resume))
        .route("/checks/{id}/regenerate", post(check_regenerate))
        .route("/checks/{id}/delete", post(check_delete))
        .route("/projects/{pid}/channels/new", get(channel_new))
        .route("/projects/{pid}/channels", post(channel_create))
        .route("/channels/{id}/delete", post(channel_delete))
        .route("/checks/{id}/channels", post(check_set_channels))
        .route("/settings", get(settings_page).post(settings_save))
        .route("/users", get(users_page).post(users_create))
        .route("/users/{id}/delete", post(users_delete))
}

// --- templates ---
#[derive(Template)]
#[template(path = "setup.html")]
struct SetupTemplate {
    show_nav: bool,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    show_nav: bool,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    show_nav: bool,
    projects: Vec<ProjectRow>,
}

pub struct ProjectRow {
    pub project: Project,
    pub checks: Vec<Check>,
}

// --- forms ---
#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
}

// --- handlers ---
async fn setup_page(State(state): State<AppState>) -> Result<Response, AppError> {
    if state.store.count_users().await? > 0 {
        return Ok(Redirect::to("/login").into_response());
    }
    Ok(render(&SetupTemplate {
        show_nav: false,
        error: None,
    })?
    .into_response())
}

async fn setup_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(creds): Form<Credentials>,
) -> Result<Response, AppError> {
    if state.store.count_users().await? > 0 {
        return Ok(Redirect::to("/login").into_response());
    }
    if creds.username.is_empty() || creds.password.is_empty() {
        return Ok(render(&SetupTemplate {
            show_nav: false,
            error: Some("username and password are required".into()),
        })?
        .into_response());
    }
    // `argon2::password_hash::Error` does not implement `std::error::Error`,
    // so it cannot be boxed directly into `AppError::Other`'s
    // `Box<dyn Error + Send + Sync>` payload; go through its `Display` text.
    let phc = hash_password(&creds.password).map_err(|e| AppError::Other(e.to_string().into()))?;
    let uid = state
        .store
        .create_user(&creds.username, Some(&phc), true, Utc::now())
        .await?;
    let jar = start_session(&state.store, jar, uid).await?;
    Ok((jar, Redirect::to("/")).into_response())
}

async fn login_page(State(state): State<AppState>) -> Result<Response, AppError> {
    if state.store.count_users().await? == 0 {
        return Ok(Redirect::to("/setup").into_response());
    }
    Ok(render(&LoginTemplate {
        show_nav: false,
        error: None,
    })?
    .into_response())
}

async fn login_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(creds): Form<Credentials>,
) -> Result<Response, AppError> {
    let user = state.store.find_user_by_username(&creds.username).await?;
    let ok = user
        .as_ref()
        .and_then(|u| u.password_hash.as_deref())
        .map(|phc| verify_password(&creds.password, phc))
        .unwrap_or(false);
    if !ok {
        return Ok(render(&LoginTemplate {
            show_nav: false,
            error: Some("invalid username or password".into()),
        })?
        .into_response());
    }
    let jar = start_session(&state.store, jar, user.unwrap().id).await?;
    Ok((jar, Redirect::to("/")).into_response())
}

async fn logout(State(state): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        state.store.delete_session(cookie.value()).await?;
    }
    let jar = jar.remove(Cookie::from(SESSION_COOKIE));
    Ok((jar, Redirect::to("/login")).into_response())
}

async fn dashboard(
    State(state): State<AppState>,
    OptionalUser(user): OptionalUser,
) -> Result<Response, AppError> {
    if state.store.count_users().await? == 0 {
        return Ok(Redirect::to("/setup").into_response());
    }
    let user = match user {
        Some(u) => u,
        None => return Ok(Redirect::to("/login").into_response()),
    };
    let projects = state.store.list_projects_for_user(user.id).await?;
    let mut rows = Vec::with_capacity(projects.len());
    for project in projects {
        let checks = state.store.list_checks_for_project(project.id).await?;
        rows.push(ProjectRow { project, checks });
    }
    Ok(render(&DashboardTemplate {
        show_nav: true,
        projects: rows,
    })?
    .into_response())
}

/// Create a session row and return a jar carrying the session cookie.
async fn start_session(store: &Store, jar: CookieJar, user_id: i64) -> Result<CookieJar, AppError> {
    let token = new_session_token();
    let expires = Utc::now() + Duration::days(SESSION_TTL_DAYS);
    store.create_session(&token, user_id, expires).await?;
    let cookie = Cookie::build((SESSION_COOKIE, token))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .build();
    Ok(jar.add(cookie))
}

// --- project templates ---
#[derive(Template)]
#[template(path = "project_form.html")]
struct ProjectFormTemplate {
    show_nav: bool,
    heading: String,
    action: String,
    name: String,
    scan_interval_secs: String,
}

#[derive(Template)]
#[template(path = "project.html")]
struct ProjectTemplate {
    show_nav: bool,
    project: Project,
    checks: Vec<Check>,
    channels: Vec<Channel>,
}

#[derive(Deserialize)]
struct ProjectForm {
    name: String,
    scan_interval_secs: String,
}

fn parse_opt_i64(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        t.parse::<i64>().ok()
    }
}

/// Load a project and enforce ownership, returning `AppError::NotFound` if it
/// does not exist or belongs to another user.
async fn owned_project(store: &Store, id: i64, user_id: i64) -> Result<Project, AppError> {
    let p = store.find_project(id).await?.ok_or(AppError::NotFound)?;
    if p.user_id != user_id {
        return Err(AppError::NotFound);
    }
    Ok(p)
}

async fn project_new(CurrentUser(_u): CurrentUser) -> Result<Response, AppError> {
    Ok(render(&ProjectFormTemplate {
        show_nav: true,
        heading: "New project".into(),
        action: "/projects".into(),
        name: String::new(),
        scan_interval_secs: String::new(),
    })?
    .into_response())
}

async fn project_create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<ProjectForm>,
) -> Result<Response, AppError> {
    let id = state
        .store
        .create_project(
            user.id,
            &form.name,
            parse_opt_i64(&form.scan_interval_secs),
            Utc::now(),
        )
        .await?;
    Ok(Redirect::to(&format!("/projects/{id}")).into_response())
}

async fn project_show(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = owned_project(&state.store, id, user.id).await?;
    let checks = state.store.list_checks_for_project(id).await?;
    let channels = state.store.list_channels_for_project(id).await?;
    Ok(render(&ProjectTemplate {
        show_nav: true,
        project,
        checks,
        channels,
    })?
    .into_response())
}

async fn project_edit(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = owned_project(&state.store, id, user.id).await?;
    Ok(render(&ProjectFormTemplate {
        show_nav: true,
        heading: "Edit project".into(),
        action: format!("/projects/{id}"),
        name: project.name,
        scan_interval_secs: project
            .scan_interval_secs
            .map(|v| v.to_string())
            .unwrap_or_default(),
    })?
    .into_response())
}

async fn project_update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<ProjectForm>,
) -> Result<Response, AppError> {
    owned_project(&state.store, id, user.id).await?;
    state
        .store
        .update_project(id, &form.name, parse_opt_i64(&form.scan_interval_secs))
        .await?;
    Ok(Redirect::to(&format!("/projects/{id}")).into_response())
}

async fn project_delete(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    owned_project(&state.store, id, user.id).await?;
    state.store.delete_project(id).await?;
    Ok(Redirect::to("/").into_response())
}

// --- check templates ---
#[derive(Deserialize)]
struct CheckForm {
    name: String,
    schedule_kind: String,
    period_secs: String,
    cron_expr: String,
    grace_secs: String,
    timezone: String,
    scan_interval_secs: String,
    max_runtime_secs: String,
}

struct PingRow {
    created_at: String,
    kind: PingKindWrap,
    exit_code_display: String,
}
struct PingKindWrap(crate::models::PingKind);
impl PingKindWrap {
    fn as_str(&self) -> &'static str {
        self.0.as_str()
    }
}

struct ChannelBox {
    id: i64,
    name: String,
    kind: &'static str,
    bound: bool,
}

#[derive(Template)]
#[template(path = "check_form.html")]
struct CheckFormTemplate {
    show_nav: bool,
    heading: String,
    action: String,
    error: Option<String>,
    name: String,
    schedule_kind: String,
    period_secs: String,
    cron_expr: String,
    grace_secs: String,
    timezone: String,
    scan_interval_secs: String,
    max_runtime_secs: String,
}

#[derive(Template)]
#[template(path = "check.html")]
struct CheckTemplate {
    show_nav: bool,
    check: Check,
    ping_url: String,
    channel_boxes: Vec<ChannelBox>,
    pings: Vec<PingRow>,
    notifications: Vec<Notification>,
}

/// Load a check and enforce ownership through its project.
async fn owned_check(store: &Store, id: i64, user_id: i64) -> Result<Check, AppError> {
    let check = store.find_check(id).await?.ok_or(AppError::NotFound)?;
    owned_project(store, check.project_id, user_id).await?;
    Ok(check)
}

fn empty_check_form(heading: &str, action: String) -> CheckFormTemplate {
    CheckFormTemplate {
        show_nav: true,
        heading: heading.into(),
        action,
        error: None,
        name: String::new(),
        schedule_kind: "period".into(),
        period_secs: String::new(),
        cron_expr: String::new(),
        grace_secs: "300".into(),
        timezone: "UTC".into(),
        scan_interval_secs: String::new(),
        max_runtime_secs: String::new(),
    }
}

/// Validate a check form into (kind, period_secs, grace_secs, cron_expr). Returns
/// `Err(message)` on invalid input.
fn validate_check(
    form: &CheckForm,
) -> Result<(ScheduleKind, Option<i64>, i64, Option<String>), String> {
    let grace = parse_opt_i64(&form.grace_secs).ok_or("grace_secs must be an integer")?;
    if grace < 0 {
        return Err("grace_secs must be >= 0".into());
    }
    let kind = ScheduleKind::from_str(&form.schedule_kind)
        .map_err(|_| "invalid schedule kind".to_string())?;
    match kind {
        ScheduleKind::Period => {
            let secs =
                parse_opt_i64(&form.period_secs).ok_or("period_secs required for period mode")?;
            if secs <= 0 {
                return Err("period_secs must be > 0".into());
            }
            Ok((kind, Some(secs), grace, None))
        }
        ScheduleKind::Cron => {
            let expr = form.cron_expr.trim();
            if expr.is_empty() {
                return Err("cron_expr required for cron mode".into());
            }
            Schedule::from_str(expr).map_err(|e| format!("invalid cron expression: {e}"))?;
            Ok((kind, None, grace, Some(expr.to_string())))
        }
    }
}

async fn check_new(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    Ok(render(&empty_check_form(
        "New check",
        format!("/projects/{pid}/checks"),
    ))?
    .into_response())
}

async fn check_create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
    Form(form): Form<CheckForm>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    let (kind, period_secs, grace, cron_expr) = match validate_check(&form) {
        Ok(v) => v,
        Err(msg) => {
            let mut t = empty_check_form("New check", format!("/projects/{pid}/checks"));
            t.error = Some(msg);
            t.name = form.name;
            t.schedule_kind = form.schedule_kind;
            t.period_secs = form.period_secs;
            t.cron_expr = form.cron_expr;
            t.grace_secs = form.grace_secs;
            t.timezone = form.timezone;
            t.scan_interval_secs = form.scan_interval_secs;
            t.max_runtime_secs = form.max_runtime_secs;
            return Ok(render(&t)?.into_response());
        }
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let id = state
        .store
        .create_check(
            pid,
            &form.name,
            &uuid,
            kind,
            period_secs,
            grace,
            cron_expr.as_deref(),
            &form.timezone,
        )
        .await?;
    state
        .store
        .update_check_schedule(
            id,
            &form.name,
            kind,
            period_secs,
            grace,
            cron_expr.as_deref(),
            &form.timezone,
            parse_opt_i64(&form.scan_interval_secs),
            parse_opt_i64(&form.max_runtime_secs),
            // Task 5 replaces this with parse_opt_i64(&form.nag_interval_secs).
            None,
        )
        .await?;
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}

async fn check_show(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    let ping_url = format!(
        "{}/ping/{}",
        state.config.base_url.trim_end_matches('/'),
        check.ping_uuid
    );
    let bound = state.store.bound_channel_ids(id).await?;
    let channel_boxes = state
        .store
        .list_channels_for_project(check.project_id)
        .await?
        .into_iter()
        .map(|c| ChannelBox {
            id: c.id,
            name: c.name,
            kind: c.kind.as_str(),
            bound: bound.contains(&c.id),
        })
        .collect();
    let pings = state
        .store
        .list_recent_pings(id, 20)
        .await?
        .into_iter()
        .map(|p| PingRow {
            created_at: p.created_at.to_rfc3339(),
            kind: PingKindWrap(p.kind),
            exit_code_display: p.exit_code.map(|c| c.to_string()).unwrap_or_default(),
        })
        .collect();
    let notifications = state.store.list_recent_notifications(id, 20).await?;
    Ok(render(&CheckTemplate {
        show_nav: true,
        check,
        ping_url,
        channel_boxes,
        pings,
        notifications,
    })?
    .into_response())
}

async fn check_edit(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    Ok(render(&CheckFormTemplate {
        show_nav: true,
        heading: "Edit check".into(),
        action: format!("/checks/{id}"),
        error: None,
        name: check.name,
        schedule_kind: check.schedule_kind.as_str().into(),
        period_secs: check.period_secs.map(|v| v.to_string()).unwrap_or_default(),
        cron_expr: check.cron_expr.unwrap_or_default(),
        grace_secs: check.grace_secs.to_string(),
        timezone: check.timezone,
        scan_interval_secs: check
            .scan_interval_secs
            .map(|v| v.to_string())
            .unwrap_or_default(),
        max_runtime_secs: check
            .max_runtime_secs
            .map(|v| v.to_string())
            .unwrap_or_default(),
    })?
    .into_response())
}

async fn check_update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<CheckForm>,
) -> Result<Response, AppError> {
    owned_check(&state.store, id, user.id).await?;
    let (kind, period_secs, grace, cron_expr) = match validate_check(&form) {
        Ok(v) => v,
        Err(msg) => {
            let t = CheckFormTemplate {
                show_nav: true,
                heading: "Edit check".into(),
                action: format!("/checks/{id}"),
                error: Some(msg),
                name: form.name,
                schedule_kind: form.schedule_kind,
                period_secs: form.period_secs,
                cron_expr: form.cron_expr,
                grace_secs: form.grace_secs,
                timezone: form.timezone,
                scan_interval_secs: form.scan_interval_secs,
                max_runtime_secs: form.max_runtime_secs,
            };
            return Ok(render(&t)?.into_response());
        }
    };
    state
        .store
        .update_check_schedule(
            id,
            &form.name,
            kind,
            period_secs,
            grace,
            cron_expr.as_deref(),
            &form.timezone,
            parse_opt_i64(&form.scan_interval_secs),
            parse_opt_i64(&form.max_runtime_secs),
            // Task 5 replaces this with parse_opt_i64(&form.nag_interval_secs).
            None,
        )
        .await?;
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}

async fn check_pause(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    owned_check(&state.store, id, user.id).await?;
    state.store.set_status(id, CheckStatus::Paused).await?;
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}

async fn check_resume(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    owned_check(&state.store, id, user.id).await?;
    state.store.set_status(id, CheckStatus::New).await?;
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}

async fn check_regenerate(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    owned_check(&state.store, id, user.id).await?;
    state
        .store
        .regenerate_uuid(id, &uuid::Uuid::new_v4().to_string())
        .await?;
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}

async fn check_delete(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    state.store.delete_check(id).await?;
    Ok(Redirect::to(&format!("/projects/{}", check.project_id)).into_response())
}

// --- channel templates ---
#[derive(Template)]
#[template(path = "channel_form.html")]
struct ChannelFormTemplate {
    show_nav: bool,
    project_id: i64,
    error: Option<String>,
}

#[derive(Deserialize)]
struct ChannelForm {
    name: String,
    kind: String,
    #[serde(default)]
    webhook_url: String,
    #[serde(default)]
    slack_url: String,
    #[serde(default)]
    telegram_token: String,
    #[serde(default)]
    telegram_chat_id: String,
    #[serde(default)]
    ntfy_base_url: String, // optional, defaults to https://ntfy.sh
    #[serde(default)]
    ntfy_topic: String,
    #[serde(default)]
    ntfy_token: String, // optional
    #[serde(default)]
    pushover_token: String, // application token
    #[serde(default)]
    pushover_user: String, // user/group key
}

#[derive(Deserialize)]
struct BindForm {
    #[serde(default)]
    channel_ids: Vec<i64>,
}

async fn channel_new(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    Ok(render(&ChannelFormTemplate {
        show_nav: true,
        project_id: pid,
        error: None,
    })?
    .into_response())
}

async fn channel_create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
    Form(form): Form<ChannelForm>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;

    let err = |msg: &str| -> Result<Response, AppError> {
        Ok(render(&ChannelFormTemplate {
            show_nav: true,
            project_id: pid,
            error: Some(msg.to_string()),
        })?
        .into_response())
    };

    let name = form.name.trim();
    if name.is_empty() {
        return err("a channel name is required");
    }

    let Ok(kind) = ChannelKind::from_str(&form.kind) else {
        return err("unknown channel kind");
    };

    let config = match kind {
        ChannelKind::Webhook => {
            let url = form.webhook_url.trim();
            if url.is_empty() {
                return err("a webhook URL is required");
            }
            serde_json::json!({ "url": url }).to_string()
        }
        ChannelKind::Slack => {
            let url = form.slack_url.trim();
            if url.is_empty() {
                return err("a Slack incoming-webhook URL is required");
            }
            serde_json::json!({ "url": url }).to_string()
        }
        ChannelKind::Telegram => {
            let token = form.telegram_token.trim();
            let chat_id = form.telegram_chat_id.trim();
            if token.is_empty() || chat_id.is_empty() {
                return err("Telegram requires both a bot token and a chat id");
            }
            serde_json::json!({ "token": token, "chat_id": chat_id }).to_string()
        }
        ChannelKind::Ntfy => {
            let topic = form.ntfy_topic.trim();
            if topic.is_empty() {
                return err("ntfy requires a topic");
            }
            let base_url = {
                let b = form.ntfy_base_url.trim();
                if b.is_empty() {
                    "https://ntfy.sh"
                } else {
                    b
                }
            };
            let token = form.ntfy_token.trim();
            serde_json::json!({
                "base_url": base_url,
                "topic": topic,
                "token": token,
            })
            .to_string()
        }
        ChannelKind::Pushover => {
            let token = form.pushover_token.trim();
            let user = form.pushover_user.trim();
            if token.is_empty() || user.is_empty() {
                return err("Pushover requires both an application token and a user key");
            }
            serde_json::json!({ "token": token, "user": user }).to_string()
        }
    };

    state
        .store
        .create_channel(pid, kind, name, &config, Utc::now())
        .await?;
    Ok(Redirect::to(&format!("/projects/{pid}")).into_response())
}

async fn channel_delete(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let channel = state
        .store
        .find_channel(id)
        .await?
        .ok_or(AppError::NotFound)?;
    let project = owned_project(&state.store, channel.project_id, user.id).await?;
    state.store.delete_channel(id).await?;
    Ok(Redirect::to(&format!("/projects/{}", project.id)).into_response())
}

/// Replace a check's bound channel set with exactly the submitted ids (only
/// those that belong to the same project are honored).
async fn check_set_channels(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    HtmlForm(form): HtmlForm<BindForm>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    let valid: std::collections::HashSet<i64> = state
        .store
        .list_channels_for_project(check.project_id)
        .await?
        .into_iter()
        .map(|c| c.id)
        .collect();
    let current: std::collections::HashSet<i64> = state
        .store
        .bound_channel_ids(id)
        .await?
        .into_iter()
        .collect();
    let desired: std::collections::HashSet<i64> = form
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
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}

// --- settings / user administration (admin only) ---
#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsTemplate {
    show_nav: bool,
    scan_interval: String,
}

#[derive(Template)]
#[template(path = "users.html")]
struct UsersTemplate {
    show_nav: bool,
    users: Vec<User>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct SettingsForm {
    scan_interval: String,
}

#[derive(Deserialize)]
struct NewUserForm {
    username: String,
    password: String,
    #[serde(default)]
    is_admin: Option<String>,
}

async fn settings_page(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Response, AppError> {
    let scan_interval = state
        .store
        .get_setting("scan_interval")
        .await?
        .unwrap_or_default();
    Ok(render(&SettingsTemplate {
        show_nav: true,
        scan_interval,
    })?
    .into_response())
}

async fn settings_save(
    State(state): State<AppState>,
    _admin: AdminUser,
    Form(form): Form<SettingsForm>,
) -> Result<Response, AppError> {
    let trimmed = form.scan_interval.trim();
    // Only persist a positive integer; blank clears to default behavior.
    if trimmed.is_empty() {
        state.store.set_setting("scan_interval", "").await?;
    } else if trimmed.parse::<u64>().map(|v| v > 0).unwrap_or(false) {
        state.store.set_setting("scan_interval", trimmed).await?;
    }
    Ok(Redirect::to("/settings").into_response())
}

async fn users_page(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Response, AppError> {
    let users = state.store.list_users().await?;
    Ok(render(&UsersTemplate {
        show_nav: true,
        users,
        error: None,
    })?
    .into_response())
}

async fn users_create(
    State(state): State<AppState>,
    _admin: AdminUser,
    Form(form): Form<NewUserForm>,
) -> Result<Response, AppError> {
    if form.username.trim().is_empty() || form.password.is_empty() {
        let users = state.store.list_users().await?;
        return Ok(render(&UsersTemplate {
            show_nav: true,
            users,
            error: Some("username and password are required".into()),
        })?
        .into_response());
    }
    let phc = hash_password(&form.password).map_err(|e| AppError::Other(e.to_string().into()))?;
    // A checked checkbox submits `is_admin=1`; an unchecked one is either
    // omitted entirely or (as form-encoded test clients sometimes do) sent as
    // an empty string — both must be treated as "not admin".
    let is_admin = form.is_admin.as_deref().is_some_and(|s| !s.is_empty());
    state
        .store
        .create_user(form.username.trim(), Some(&phc), is_admin, Utc::now())
        .await?;
    Ok(Redirect::to("/users").into_response())
}

async fn users_delete(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    // Never allow deleting yourself or the last admin (lockout guard).
    if id == admin.id {
        return Ok(Redirect::to("/users").into_response());
    }
    let admins = state
        .store
        .list_users()
        .await?
        .into_iter()
        .filter(|u| u.is_admin)
        .count();
    let target = state.store.find_user_by_id(id).await?;
    if let Some(t) = target {
        if t.is_admin && admins <= 1 {
            return Ok(Redirect::to("/users").into_response());
        }
    }
    state.store.delete_user(id).await?;
    Ok(Redirect::to("/users").into_response())
}
