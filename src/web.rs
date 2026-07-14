use crate::auth::{
    hash_password, new_session_token, verify_password, AdminUser, CurrentUser, OptionalUser,
    SESSION_COOKIE, SESSION_TTL_DAYS,
};
use crate::error::AppError;
use crate::models::{
    Channel, ChannelKind, Check, CheckStatus, Notification, Project, ScheduleKind, User,
};
use crate::notify::{notifier_for, EventKind, NotificationEvent};
use crate::state::AppState;
use crate::store::Store;
use askama::Template;
use axum::extract::{Path, Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
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
        .route("/checks/{id}/ack", post(check_ack))
        .route("/checks/{id}/regenerate", post(check_regenerate))
        .route("/checks/{id}/delete", post(check_delete))
        .route("/projects/{pid}/channels/new", get(channel_new))
        .route("/projects/{pid}/channels", post(channel_create))
        .route("/channels/{id}/delete", post(channel_delete))
        .route("/channels/{id}/test", post(channel_test))
        .route("/checks/{id}/channels", post(check_set_channels))
        .route("/settings", get(settings_page).post(settings_save))
        .route("/users", get(users_page).post(users_create))
        .route("/users/{id}/delete", post(users_delete))
        .route("/users/{id}/password", post(users_set_password))
        .route("/users/{id}/admin", post(users_toggle_admin))
        .route("/users/{id}/disabled", post(users_set_disabled))
        // --- admin cross-user route group (each handler guarded by AdminUser) ---
        .route("/admin", get(admin_dashboard))
        .route("/admin/projects", get(admin_projects_page))
        .route(
            "/admin/projects/{id}",
            get(admin_project_show).post(admin_project_update),
        )
        .route("/admin/projects/{id}/edit", get(admin_project_edit))
        .route("/admin/projects/{id}/delete", post(admin_project_delete))
        .route("/admin/projects/{pid}/checks/new", get(admin_check_new))
        .route("/admin/projects/{pid}/checks", post(admin_check_create))
        .route(
            "/admin/checks/{id}",
            get(admin_check_show).post(admin_check_update),
        )
        .route("/admin/checks/{id}/edit", get(admin_check_edit))
        .route("/admin/checks/{id}/pause", post(admin_check_pause))
        .route("/admin/checks/{id}/resume", post(admin_check_resume))
        .route("/admin/checks/{id}/ack", post(admin_check_ack))
        .route(
            "/admin/checks/{id}/regenerate",
            post(admin_check_regenerate),
        )
        .route("/admin/checks/{id}/delete", post(admin_check_delete))
        .route("/admin/projects/{pid}/channels/new", get(admin_channel_new))
        .route("/admin/projects/{pid}/channels", post(admin_channel_create))
        .route("/admin/channels/{id}/delete", post(admin_channel_delete))
        .route("/admin/channels/{id}/test", post(admin_channel_test))
        .route(
            "/admin/checks/{id}/channels",
            post(admin_check_set_channels),
        )
}

// --- templates ---
#[derive(Template)]
#[template(path = "setup.html")]
struct SetupTemplate {
    show_nav: bool,
    is_admin: bool,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    show_nav: bool,
    is_admin: bool,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    show_nav: bool,
    is_admin: bool,
    total: usize,
    up: usize,
    late: usize,
    down: usize,
    groups: Vec<ProjectGroup>,
}

struct CheckRow {
    id: i64,
    name: String,
    status: &'static str, // view::DisplayStatus::as_str()
    schedule: String,     // e.g. "every 1h · 10m grace" or the cron expr
    last: String,         // fmt_relative or "—"
    bars: Vec<crate::view::Bar>,
}

struct ProjectGroup {
    id: i64,
    name: String,
    count: usize,
    checks: Vec<CheckRow>,
}

/// Human-readable schedule summary shown under a check's name (dashboard rows
/// and the check detail page).
fn schedule_label(c: &Check) -> String {
    match c.schedule_kind {
        ScheduleKind::Period => match c.period_secs {
            Some(s) => format!(
                "every {} · {} grace",
                crate::view::fmt_secs(s),
                crate::view::fmt_secs(c.grace_secs)
            ),
            None => format!("{} grace", crate::view::fmt_secs(c.grace_secs)),
        },
        ScheduleKind::Cron => c.cron_expr.clone().unwrap_or_default(),
    }
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
        is_admin: false,
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
            is_admin: false,
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
        is_admin: false,
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
            is_admin: false,
            error: Some("invalid username or password".into()),
        })?
        .into_response());
    }
    let user = user.unwrap();
    if user.disabled {
        return Ok(render(&LoginTemplate {
            show_nav: false,
            is_admin: false,
            error: Some("account is disabled".into()),
        })?
        .into_response());
    }
    let jar = start_session(&state.store, jar, user.id).await?;
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
    let now = Utc::now();
    let (mut total, mut up, mut late, mut down) = (0usize, 0, 0, 0);
    let mut groups = Vec::new();
    for project in state.store.list_projects_for_user(user.id).await? {
        let checks = state.store.list_checks_for_project(project.id).await?;
        let mut rows = Vec::with_capacity(checks.len());
        for c in &checks {
            let ds = crate::view::display_status(c, now);
            total += 1;
            match ds {
                crate::view::DisplayStatus::Up => up += 1,
                crate::view::DisplayStatus::Late => late += 1,
                crate::view::DisplayStatus::Down => down += 1,
                _ => {}
            }
            let pings = state.store.list_recent_pings(c.id, 40).await?;
            let bars = crate::view::heartbeat(
                &pings,
                c.max_runtime_secs,
                c.status == CheckStatus::Paused,
                6,
            );
            rows.push(CheckRow {
                id: c.id,
                name: c.name.clone(),
                status: ds.as_str(),
                schedule: schedule_label(c),
                last: c
                    .last_ping_at
                    .map(|t| crate::view::fmt_relative(t, now))
                    .unwrap_or_else(|| "—".into()),
                bars,
            });
        }
        groups.push(ProjectGroup {
            id: project.id,
            name: project.name,
            count: checks.len(),
            checks: rows,
        });
    }
    Ok(render(&DashboardTemplate {
        show_nav: true,
        is_admin: user.is_admin,
        total,
        up,
        late,
        down,
        groups,
    })?
    .into_response())
}

/// Create a session row and return a jar carrying the session cookie.
async fn start_session(store: &Store, jar: CookieJar, user_id: i64) -> Result<CookieJar, AppError> {
    let token = new_session_token();
    // Per-session CSRF synchronizer token, validated by `csrf_guard` on every
    // state-changing browser request and embedded in POST forms by the render
    // path (looked up via `Store::session_csrf_token`).
    let csrf = new_session_token();
    let expires = Utc::now() + Duration::days(SESSION_TTL_DAYS);
    store
        .create_session(&token, user_id, &csrf, expires)
        .await?;
    let cookie = Cookie::build((SESSION_COOKIE, token))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .build();
    Ok(jar.add(cookie))
}

/// CSRF synchronizer-token guard, applied to `web::routes()` only (the machine
/// `/ping/*` endpoints, assets, and `/healthz` live in sibling routers and are
/// therefore structurally exempt).
///
/// Safe methods (GET/HEAD/OPTIONS) and the pre-session `POST /login` and
/// `POST /setup` paths pass through untouched. Every other state-changing
/// request must present the session's stored token, taken from the
/// `X-CSRF-Token` header or, failing that, the `_csrf` urlencoded form field
/// (in which case the body is buffered and the request rebuilt so the
/// downstream `Form<T>` extractor still works). The token is a random UUID, so
/// a plain `==` comparison (no constant-time compare) is adequate here.
pub async fn csrf_guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // Safe methods never change state.
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    // Pre-session paths: no session (and hence no token) exists yet.
    let path = req.uri().path();
    if path == "/login" || path == "/setup" {
        return next.run(req).await;
    }
    // Resolve the caller's session token from the cookie, then its stored CSRF.
    let jar = CookieJar::from_headers(req.headers());
    let stored = match jar.get(SESSION_COOKIE) {
        Some(cookie) => match state.store.session_csrf_token(cookie.value()).await {
            Ok(Some(tok)) if !tok.is_empty() => tok,
            _ => return StatusCode::FORBIDDEN.into_response(),
        },
        None => return StatusCode::FORBIDDEN.into_response(),
    };
    // Prefer the header token — this path avoids buffering the body.
    if let Some(submitted) = req
        .headers()
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
    {
        if stored == submitted {
            return next.run(req).await;
        }
        return StatusCode::FORBIDDEN.into_response();
    }
    // Otherwise read the `_csrf` form field: buffer the body, extract the token,
    // then rebuild the request with the same bytes for the downstream handler.
    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, 1 << 20).await {
        Ok(b) => b,
        Err(_) => return StatusCode::FORBIDDEN.into_response(),
    };
    let submitted = form_urlencoded::parse(&bytes)
        .find(|(k, _)| k == "_csrf")
        .map(|(_, v)| v.into_owned());
    if submitted.as_deref() != Some(stored.as_str()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let req = Request::from_parts(parts, axum::body::Body::from(bytes));
    next.run(req).await
}

// --- project templates ---
#[derive(Template)]
#[template(path = "project_form.html")]
struct ProjectFormTemplate {
    show_nav: bool,
    is_admin: bool,
    heading: String,
    action: String,
    name: String,
    scan_interval_secs: String,
    nag_interval_secs: String,
}

#[derive(Template)]
#[template(path = "project.html")]
struct ProjectTemplate {
    show_nav: bool,
    is_admin: bool,
    admin: bool,
    project: Project,
    checks: Vec<Check>,
    channels: Vec<Channel>,
    test_result: Option<TestResult>,
}

struct TestResult {
    ok: bool,
    message: String,
}

#[derive(Deserialize)]
struct ProjectForm {
    name: String,
    scan_interval_secs: String,
    nag_interval_secs: String,
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

/// Resolve any project by id (no owner filter) and record an admin-access
/// audit entry. The single choke point for #1 cross-user reads and writes.
async fn admin_project(
    state: &AppState,
    id: i64,
    admin: &User,
    method: &str,
    path: &str,
) -> Result<Project, AppError> {
    let p = state
        .store
        .find_project(id)
        .await?
        .ok_or(AppError::NotFound)?;
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "admin.access",
                target_type: Some("project"),
                target_id: Some(p.id),
                target_owner_id: Some(p.user_id),
                method: Some(method),
                path: Some(path),
                detail: None,
            },
            Utc::now(),
        )
        .await?;
    Ok(p)
}

async fn admin_check(
    state: &AppState,
    id: i64,
    admin: &User,
    method: &str,
    path: &str,
) -> Result<Check, AppError> {
    let c = state
        .store
        .find_check(id)
        .await?
        .ok_or(AppError::NotFound)?;
    let owner = state
        .store
        .find_project(c.project_id)
        .await?
        .map(|p| p.user_id);
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "admin.access",
                target_type: Some("check"),
                target_id: Some(c.id),
                target_owner_id: owner,
                method: Some(method),
                path: Some(path),
                detail: None,
            },
            Utc::now(),
        )
        .await?;
    Ok(c)
}

async fn admin_channel(
    state: &AppState,
    id: i64,
    admin: &User,
    method: &str,
    path: &str,
) -> Result<Channel, AppError> {
    let ch = state
        .store
        .find_channel(id)
        .await?
        .ok_or(AppError::NotFound)?;
    let owner = state
        .store
        .find_project(ch.project_id)
        .await?
        .map(|p| p.user_id);
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "admin.access",
                target_type: Some("channel"),
                target_id: Some(ch.id),
                target_owner_id: owner,
                method: Some(method),
                path: Some(path),
                detail: None,
            },
            Utc::now(),
        )
        .await?;
    Ok(ch)
}

async fn project_new(CurrentUser(user): CurrentUser) -> Result<Response, AppError> {
    Ok(render(&ProjectFormTemplate {
        show_nav: true,
        is_admin: user.is_admin,
        heading: "New project".into(),
        action: "/projects".into(),
        name: String::new(),
        scan_interval_secs: String::new(),
        nag_interval_secs: String::new(),
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
            parse_opt_i64(&form.nag_interval_secs),
            Utc::now(),
        )
        .await?;
    Ok(Redirect::to(&format!("/projects/{id}")).into_response())
}

/// `/admin` when acting as an admin, otherwise the empty (owner) prefix. Used
/// to point rendered links, form actions, and redirects at the right route.
fn admin_prefix(admin: bool) -> &'static str {
    if admin {
        "/admin"
    } else {
        ""
    }
}

/// Render the project page, optionally with a channel-test result banner.
/// `admin` renders `/admin`-prefixed action URLs; `is_admin` reflects the
/// current viewer's admin status and controls the nav Admin link.
async fn render_project_page(
    store: &Store,
    project: Project,
    test_result: Option<TestResult>,
    admin: bool,
    is_admin: bool,
) -> Result<Response, AppError> {
    let checks = store.list_checks_for_project(project.id).await?;
    let channels = store.list_channels_for_project(project.id).await?;
    Ok(render(&ProjectTemplate {
        show_nav: true,
        is_admin,
        admin,
        project,
        checks,
        channels,
        test_result,
    })?
    .into_response())
}

/// Build the project edit form, pointing its action at the owner or `/admin`
/// route depending on `admin`. `is_admin` reflects the current viewer's admin
/// status and controls the nav Admin link.
fn project_edit_form(project: Project, admin: bool, is_admin: bool) -> ProjectFormTemplate {
    let base = admin_prefix(admin);
    ProjectFormTemplate {
        show_nav: true,
        is_admin,
        heading: "Edit project".into(),
        action: format!("{base}/projects/{}", project.id),
        name: project.name,
        scan_interval_secs: project
            .scan_interval_secs
            .map(|v| v.to_string())
            .unwrap_or_default(),
        nag_interval_secs: project
            .nag_interval_secs
            .map(|v| v.to_string())
            .unwrap_or_default(),
    }
}

async fn project_show(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = owned_project(&state.store, id, user.id).await?;
    render_project_page(&state.store, project, None, false, user.is_admin).await
}

async fn project_edit(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = owned_project(&state.store, id, user.id).await?;
    Ok(render(&project_edit_form(project, false, user.is_admin))?.into_response())
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
        .update_project(
            id,
            &form.name,
            parse_opt_i64(&form.scan_interval_secs),
            parse_opt_i64(&form.nag_interval_secs),
        )
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
    nag_interval_secs: String,
}

struct PingRow {
    time: String,             // UTC fallback shown when JS is off
    iso: String,              // RFC3339 UTC; localized to the viewer's zone client-side
    pill_class: &'static str, // pill/output css class: "ok"|"fail"|"start"|"log"
    kind_label: &'static str, // visible kind label (spec §8): "success"|"fail"|"start"|"log"
    exit: String,
    duration: String,
    source: String,
    body: String,
}

/// Maps a stored `PingKind` to the pill/output CSS class used on the
/// check-detail page (the visible label instead uses `PingKind::as_str()`).
/// `Exitcode` never reaches storage — `apply()` in `ping.rs` rewrites it to
/// `Success`/`Fail` before insert — but is handled defensively.
fn ping_pill_class(k: crate::models::PingKind) -> &'static str {
    use crate::models::PingKind;
    match k {
        PingKind::Success | PingKind::Exitcode => "ok",
        PingKind::Fail => "fail",
        PingKind::Start => "start",
        PingKind::Log => "log",
    }
}

struct ChannelBox {
    id: i64,
    name: String,
    kind: &'static str,
    bound: bool,
}

struct NotificationRow {
    created_at: String, // UTC fallback shown when JS is off
    iso: String,        // RFC3339 UTC; localized to the viewer's zone client-side
    event: &'static str,
    status: &'static str,
    channel: String,
    error: String,
}

#[derive(Template)]
#[template(path = "check_form.html")]
struct CheckFormTemplate {
    show_nav: bool,
    is_admin: bool,
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
    nag_interval_secs: String,
}

#[derive(Template)]
#[template(path = "check.html")]
struct CheckTemplate {
    show_nav: bool,
    is_admin: bool,
    admin: bool,
    check: Check,
    project_name: String,
    status: &'static str,
    since: String,
    schedule: String,
    ping_url: String,
    bars: Vec<crate::view::Bar>,
    channel_boxes: Vec<ChannelBox>,
    pings: Vec<PingRow>,
    notifications: Vec<NotificationRow>,
}

/// Short status line shown next to the check name on the detail page, e.g.
/// "down · 2h 14m ago · not acknowledged" or "updated 3m ago".
fn status_since_label(check: &Check, now: chrono::DateTime<Utc>) -> String {
    if crate::view::display_status(check, now) == crate::view::DisplayStatus::Down {
        let ack = if check.acknowledged {
            "acknowledged"
        } else {
            "not acknowledged"
        };
        // A check can go New -> Down (e.g. it never checked in before its
        // first deadline) without ever having received a ping.
        let relative = check
            .last_ping_at
            .map(|t| crate::view::fmt_relative(t, now))
            .unwrap_or_else(|| "no pings yet".into());
        format!("down · {relative} · {ack}")
    } else {
        let relative = check
            .last_ping_at
            .map(|t| crate::view::fmt_relative(t, now))
            .unwrap_or_else(|| "never".into());
        format!("updated {relative}")
    }
}

/// Load a check and enforce ownership through its project.
async fn owned_check(store: &Store, id: i64, user_id: i64) -> Result<Check, AppError> {
    let check = store.find_check(id).await?.ok_or(AppError::NotFound)?;
    owned_project(store, check.project_id, user_id).await?;
    Ok(check)
}

fn empty_check_form(heading: &str, action: String, is_admin: bool) -> CheckFormTemplate {
    CheckFormTemplate {
        show_nav: true,
        is_admin,
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
        nag_interval_secs: String::new(),
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
    let form = empty_check_form(
        "New check",
        format!("/projects/{pid}/checks"),
        user.is_admin,
    );
    Ok(render(&form)?.into_response())
}

/// Shared create-check core: validate, re-render the form on error, else create
/// the check and redirect. `admin` selects the owner or `/admin` route surface;
/// `is_admin` reflects the current viewer's admin status and controls the nav
/// Admin link.
async fn check_create_core(
    state: &AppState,
    pid: i64,
    form: CheckForm,
    admin: bool,
    is_admin: bool,
) -> Result<Response, AppError> {
    let base = admin_prefix(admin);
    let (kind, period_secs, grace, cron_expr) = match validate_check(&form) {
        Ok(v) => v,
        Err(msg) => {
            let mut t = empty_check_form(
                "New check",
                format!("{base}/projects/{pid}/checks"),
                is_admin,
            );
            t.error = Some(msg);
            t.name = form.name;
            t.schedule_kind = form.schedule_kind;
            t.period_secs = form.period_secs;
            t.cron_expr = form.cron_expr;
            t.grace_secs = form.grace_secs;
            t.timezone = form.timezone;
            t.scan_interval_secs = form.scan_interval_secs;
            t.max_runtime_secs = form.max_runtime_secs;
            t.nag_interval_secs = form.nag_interval_secs;
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
            parse_opt_i64(&form.nag_interval_secs),
        )
        .await?;
    Ok(Redirect::to(&format!("{base}/checks/{id}")).into_response())
}

async fn check_create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
    Form(form): Form<CheckForm>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    check_create_core(&state, pid, form, false, user.is_admin).await
}

async fn check_show(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    render_check_page(&state, check, false, user.is_admin).await
}

/// Render the check detail page. `admin` renders `/admin`-prefixed action URLs;
/// `is_admin` reflects the current viewer's admin status and controls the nav
/// Admin link.
async fn render_check_page(
    state: &AppState,
    check: Check,
    admin: bool,
    is_admin: bool,
) -> Result<Response, AppError> {
    let id = check.id;
    let project = state
        .store
        .find_project(check.project_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let now = Utc::now();
    let ping_url = format!(
        "{}/ping/{}",
        state.config.base_url.trim_end_matches('/'),
        check.ping_uuid
    );
    let bound = state.store.bound_channel_ids(id).await?;
    let project_channels = state
        .store
        .list_channels_for_project(check.project_id)
        .await?;
    let channel_names: std::collections::HashMap<i64, String> = project_channels
        .iter()
        .map(|c| (c.id, c.name.clone()))
        .collect();
    let channel_boxes = project_channels
        .into_iter()
        .map(|c| ChannelBox {
            id: c.id,
            name: c.name,
            kind: c.kind.as_str(),
            bound: bound.contains(&c.id),
        })
        .collect();
    let recent = state.store.list_recent_pings(id, 40).await?;
    let durations = crate::view::run_durations(&recent);
    let bars = crate::view::heartbeat(
        &recent,
        check.max_runtime_secs,
        check.status == CheckStatus::Paused,
        30,
    );
    let pings = recent
        .iter()
        .take(20)
        .map(|p| PingRow {
            time: p.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            iso: p.created_at.to_rfc3339(),
            pill_class: ping_pill_class(p.kind),
            kind_label: p.kind.as_str(),
            exit: p
                .exit_code
                .map(|c| format!("exit {c}"))
                .unwrap_or_else(|| "—".into()),
            duration: durations
                .get(&p.id)
                .map(|d| crate::view::fmt_secs(*d))
                .unwrap_or_else(|| "—".into()),
            source: p.source_ip.clone().unwrap_or_else(|| "—".into()),
            body: p.body.clone(),
        })
        .collect();
    let status = crate::view::display_status(&check, now).as_str();
    let since = status_since_label(&check, now);
    let schedule = schedule_label(&check);
    let notifications = state
        .store
        .list_recent_notifications(id, 20)
        .await?
        .into_iter()
        .map(|n| NotificationRow {
            created_at: n.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            iso: n.created_at.to_rfc3339(),
            event: n.event.as_str(),
            status: n.status.as_str(),
            channel: channel_names
                .get(&n.channel_id)
                .cloned()
                .unwrap_or_else(|| "(deleted)".into()),
            error: n.error.unwrap_or_default(),
        })
        .collect();
    Ok(render(&CheckTemplate {
        show_nav: true,
        is_admin,
        admin,
        check,
        project_name: project.name,
        status,
        since,
        schedule,
        ping_url,
        bars,
        channel_boxes,
        pings,
        notifications,
    })?
    .into_response())
}

/// Build the check edit form pre-filled from `check`, pointing its action at
/// the owner or `/admin` route depending on `admin`. `is_admin` reflects the
/// current viewer's admin status and controls the nav Admin link.
fn check_edit_form(check: Check, admin: bool, is_admin: bool) -> CheckFormTemplate {
    let base = admin_prefix(admin);
    CheckFormTemplate {
        show_nav: true,
        is_admin,
        heading: "Edit check".into(),
        action: format!("{base}/checks/{}", check.id),
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
        nag_interval_secs: check
            .nag_interval_secs
            .map(|v| v.to_string())
            .unwrap_or_default(),
    }
}

async fn check_edit(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    Ok(render(&check_edit_form(check, false, user.is_admin))?.into_response())
}

/// Shared update-check core: validate, re-render the form on error, else apply
/// the schedule update and redirect. `admin` selects the route surface;
/// `is_admin` reflects the current viewer's admin status and controls the nav
/// Admin link.
async fn check_update_core(
    state: &AppState,
    id: i64,
    form: CheckForm,
    admin: bool,
    is_admin: bool,
) -> Result<Response, AppError> {
    let base = admin_prefix(admin);
    let (kind, period_secs, grace, cron_expr) = match validate_check(&form) {
        Ok(v) => v,
        Err(msg) => {
            let t = CheckFormTemplate {
                show_nav: true,
                is_admin,
                heading: "Edit check".into(),
                action: format!("{base}/checks/{id}"),
                error: Some(msg),
                name: form.name,
                schedule_kind: form.schedule_kind,
                period_secs: form.period_secs,
                cron_expr: form.cron_expr,
                grace_secs: form.grace_secs,
                timezone: form.timezone,
                scan_interval_secs: form.scan_interval_secs,
                max_runtime_secs: form.max_runtime_secs,
                nag_interval_secs: form.nag_interval_secs,
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
            parse_opt_i64(&form.nag_interval_secs),
        )
        .await?;
    Ok(Redirect::to(&format!("{base}/checks/{id}")).into_response())
}

async fn check_update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<CheckForm>,
) -> Result<Response, AppError> {
    owned_check(&state.store, id, user.id).await?;
    check_update_core(&state, id, form, false, user.is_admin).await
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

async fn check_ack(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    owned_check(&state.store, id, user.id).await?;
    state.store.acknowledge(id).await?;
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
    is_admin: bool,
    admin: bool,
    project_id: i64,
    error: Option<String>,
    smtp_available: bool,
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
    #[serde(default)]
    email_to: String,
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
        is_admin: user.is_admin,
        admin: false,
        project_id: pid,
        error: None,
        smtp_available: state.config.smtp.is_some(),
    })?
    .into_response())
}

/// Shared create-channel core: validate config by kind, re-render the form on
/// error, else create the channel and redirect. `admin` selects the route
/// surface (form action + redirect target); `is_admin` reflects the current
/// viewer's admin status and controls the nav Admin link.
async fn channel_create_core(
    state: &AppState,
    pid: i64,
    form: ChannelForm,
    admin: bool,
    is_admin: bool,
) -> Result<Response, AppError> {
    let base = admin_prefix(admin);

    let err = |msg: &str| -> Result<Response, AppError> {
        Ok(render(&ChannelFormTemplate {
            show_nav: true,
            is_admin,
            admin,
            project_id: pid,
            error: Some(msg.to_string()),
            smtp_available: state.config.smtp.is_some(),
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
        ChannelKind::Email => {
            let to = form.email_to.trim();
            if to.is_empty() {
                return err("an email recipient address is required");
            }
            serde_json::json!({ "to": to }).to_string()
        }
    };

    state
        .store
        .create_channel(pid, kind, name, &config, Utc::now())
        .await?;
    Ok(Redirect::to(&format!("{base}/projects/{pid}")).into_response())
}

async fn channel_create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
    Form(form): Form<ChannelForm>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    channel_create_core(&state, pid, form, false, user.is_admin).await
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

/// Send a one-off test notification to a single channel. Sends once (no retry)
/// and does not record the attempt in the notification history.
async fn run_channel_test(state: &AppState, channel: &Channel) -> TestResult {
    let ev = NotificationEvent {
        check_id: 0,
        check_name: channel.name.clone(),
        event: EventKind::Test,
        at: Utc::now(),
        project_id: channel.project_id,
    };
    match notifier_for(channel, state.config.smtp.as_ref()) {
        None => TestResult {
            ok: false,
            message: "channel configuration is incomplete".into(),
        },
        Some(n) => match n.send(&ev).await {
            Ok(()) => TestResult {
                ok: true,
                message: format!("Test notification sent to \"{}\"", channel.name),
            },
            Err(e) => TestResult {
                ok: false,
                message: format!("Test notification failed: {e}"),
            },
        },
    }
}

/// Send a one-off test notification to a single channel and re-render the
/// project page with a result banner.
async fn channel_test(
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
    let result = run_channel_test(&state, &channel).await;
    render_project_page(&state.store, project, Some(result), false, user.is_admin).await
}

/// Replace a check's bound channel set with exactly the submitted ids (only
/// those that belong to the same project are honored). `admin` selects the
/// redirect route surface.
async fn set_channels_core(
    state: &AppState,
    check: &Check,
    form: BindForm,
    admin: bool,
) -> Result<Response, AppError> {
    let base = admin_prefix(admin);
    let id = check.id;
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
    Ok(Redirect::to(&format!("{base}/checks/{id}")).into_response())
}

async fn check_set_channels(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    HtmlForm(form): HtmlForm<BindForm>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    set_channels_core(&state, &check, form, false).await
}

// --- settings / user administration (admin only) ---
#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsTemplate {
    show_nav: bool,
    is_admin: bool,
    scan_interval: String,
    nag_interval: String,
    pings_retention_days: String,
    notifications_retention_days: String,
}

#[derive(Template)]
#[template(path = "users.html")]
struct UsersTemplate {
    show_nav: bool,
    is_admin: bool,
    users: Vec<User>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct SettingsForm {
    scan_interval: String,
    nag_interval: String,
    pings_retention_days: String,
    notifications_retention_days: String,
}

#[derive(Deserialize)]
struct NewUserForm {
    username: String,
    password: String,
    #[serde(default)]
    is_admin: Option<String>,
}

#[derive(Deserialize)]
struct PasswordForm {
    password: String,
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
    let nag_interval = state
        .store
        .get_setting("nag_interval")
        .await?
        .unwrap_or_default();
    let pings_retention_days = state
        .store
        .get_setting("pings_retention_days")
        .await?
        .unwrap_or_default();
    let notifications_retention_days = state
        .store
        .get_setting("notifications_retention_days")
        .await?
        .unwrap_or_default();
    Ok(render(&SettingsTemplate {
        show_nav: true,
        is_admin: true,
        scan_interval,
        nag_interval,
        pings_retention_days,
        notifications_retention_days,
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
    let nag = form.nag_interval.trim();
    if nag.is_empty() {
        state.store.set_setting("nag_interval", "").await?;
    } else if nag.parse::<u64>().map(|v| v > 0).unwrap_or(false) {
        state.store.set_setting("nag_interval", nag).await?;
    }
    let pr = form.pings_retention_days.trim();
    if pr.is_empty() {
        state.store.set_setting("pings_retention_days", "").await?;
    } else if pr.parse::<u64>().map(|v| v > 0).unwrap_or(false) {
        state.store.set_setting("pings_retention_days", pr).await?;
    }
    let nr = form.notifications_retention_days.trim();
    if nr.is_empty() {
        state
            .store
            .set_setting("notifications_retention_days", "")
            .await?;
    } else if nr.parse::<u64>().map(|v| v > 0).unwrap_or(false) {
        state
            .store
            .set_setting("notifications_retention_days", nr)
            .await?;
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
        is_admin: true,
        users,
        error: None,
    })?
    .into_response())
}

async fn users_create(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Form(form): Form<NewUserForm>,
) -> Result<Response, AppError> {
    if form.username.trim().is_empty() || form.password.is_empty() {
        let users = state.store.list_users().await?;
        return Ok(render(&UsersTemplate {
            show_nav: true,
            is_admin: true,
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
    let new_id = state
        .store
        .create_user(form.username.trim(), Some(&phc), is_admin, Utc::now())
        .await?;
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "user.create",
                target_type: Some("user"),
                target_id: Some(new_id),
                detail: Some(if is_admin { "admin" } else { "member" }),
                ..Default::default()
            },
            Utc::now(),
        )
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
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/users").into_response());
    };
    // Refuse to delete the last enabled admin.
    if target.is_admin && !target.disabled && state.store.count_enabled_admins().await? <= 1 {
        return Ok(Redirect::to("/users").into_response());
    }
    state.store.delete_user(id).await?;
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "user.delete",
                target_type: Some("user"),
                target_id: Some(id),
                ..Default::default()
            },
            Utc::now(),
        )
        .await?;
    Ok(Redirect::to("/users").into_response())
}

async fn users_set_password(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
    Form(form): Form<PasswordForm>,
) -> Result<Response, AppError> {
    if form.password.is_empty() {
        return Ok(Redirect::to("/users").into_response());
    }
    if state.store.find_user_by_id(id).await?.is_none() {
        return Ok(Redirect::to("/users").into_response());
    }
    let phc = hash_password(&form.password).map_err(|e| AppError::Other(e.to_string().into()))?;
    state.store.set_user_password(id, &phc).await?;
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "user.password_reset",
                target_type: Some("user"),
                target_id: Some(id),
                ..Default::default()
            },
            Utc::now(),
        )
        .await?;
    Ok(Redirect::to("/users").into_response())
}

async fn users_toggle_admin(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/users").into_response());
    };
    let new_admin = !target.is_admin;
    // Refuse to remove the last enabled admin.
    if !new_admin
        && target.is_admin
        && !target.disabled
        && state.store.count_enabled_admins().await? <= 1
    {
        return Ok(Redirect::to("/users").into_response());
    }
    state.store.set_user_admin(id, new_admin).await?;
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "user.set_admin",
                target_type: Some("user"),
                target_id: Some(id),
                detail: Some(if new_admin { "promote" } else { "demote" }),
                ..Default::default()
            },
            Utc::now(),
        )
        .await?;
    Ok(Redirect::to("/users").into_response())
}

async fn users_set_disabled(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    // Never disable yourself.
    if id == admin.id {
        return Ok(Redirect::to("/users").into_response());
    }
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/users").into_response());
    };
    let new_disabled = !target.disabled;
    // Refuse to disable the last enabled admin.
    if new_disabled
        && target.is_admin
        && !target.disabled
        && state.store.count_enabled_admins().await? <= 1
    {
        return Ok(Redirect::to("/users").into_response());
    }
    state.store.set_user_disabled(id, new_disabled).await?;
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "user.set_disabled",
                target_type: Some("user"),
                target_id: Some(id),
                detail: Some(if new_disabled { "disable" } else { "enable" }),
                ..Default::default()
            },
            Utc::now(),
        )
        .await?;
    Ok(Redirect::to("/users").into_response())
}

// --- admin route group (cross-user management, every access audited) ---
//
// Each handler resolves its target through the `admin_*` helpers (which fetch
// unfiltered and write one `admin.access` audit row), then reuses the exact
// same core logic/render helper/mutator as the owner handler, differing only in
// pointing links and redirects at the `/admin`-prefixed route surface.
#[derive(Template)]
#[template(path = "admin_projects.html")]
struct AdminProjectsTemplate {
    show_nav: bool,
    is_admin: bool,
    projects: Vec<(Project, String)>,
}

/// `/admin` landing: a cross-user dashboard with site-wide health and scale
/// figures (users/projects/checks/pings, check status rollup, notification
/// health, and scheduler heartbeat).
#[derive(Template)]
#[template(path = "admin_dashboard.html")]
struct AdminDashboardTemplate {
    show_nav: bool,
    is_admin: bool,
    users: i64,
    projects: i64,
    checks: i64,
    pings_24h: i64,
    status: crate::store::CheckStatusCounts,
    down: Vec<(Check, String, String)>,
    notif_ok: i64,
    notif_err: i64,
    channel_fail: Vec<(String, i64, i64)>,
    recent_fail: Vec<Notification>,
    last_scan_at: Option<String>,
    last_prune_at: Option<String>,
}

async fn admin_dashboard(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Response, AppError> {
    let day_ago = Utc::now() - Duration::days(1);
    let (notif_ok, notif_err) = state.store.notification_counts_since(day_ago).await?;
    Ok(render(&AdminDashboardTemplate {
        show_nav: true,
        is_admin: true,
        users: state.store.count_users().await?,
        projects: state.store.count_projects().await?,
        checks: state.store.count_checks().await?,
        pings_24h: state.store.count_pings_since(day_ago).await?,
        status: state.store.count_checks_by_status().await?,
        down: state.store.list_down_checks_with_owner().await?,
        notif_ok,
        notif_err,
        channel_fail: state.store.channel_failure_counts_since(day_ago).await?,
        recent_fail: state.store.recent_failed_notifications(10).await?,
        last_scan_at: state.store.get_setting("last_scan_at").await?,
        last_prune_at: state.store.get_setting("last_prune_at").await?,
    })?
    .into_response())
}

async fn admin_projects_page(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Response, AppError> {
    let projects = state.store.list_all_projects_with_owner().await?;
    Ok(render(&AdminProjectsTemplate {
        show_nav: true,
        is_admin: true,
        projects,
    })?
    .into_response())
}

// -- projects --
async fn admin_project_show(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = admin_project(&state, id, &admin, method.as_str(), uri.path()).await?;
    render_project_page(&state.store, project, None, true, true).await
}

async fn admin_project_edit(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = admin_project(&state, id, &admin, method.as_str(), uri.path()).await?;
    Ok(render(&project_edit_form(project, true, true))?.into_response())
}

async fn admin_project_update(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Form(form): Form<ProjectForm>,
) -> Result<Response, AppError> {
    admin_project(&state, id, &admin, method.as_str(), uri.path()).await?;
    state
        .store
        .update_project(
            id,
            &form.name,
            parse_opt_i64(&form.scan_interval_secs),
            parse_opt_i64(&form.nag_interval_secs),
        )
        .await?;
    Ok(Redirect::to(&format!("/admin/projects/{id}")).into_response())
}

async fn admin_project_delete(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    admin_project(&state, id, &admin, method.as_str(), uri.path()).await?;
    state.store.delete_project(id).await?;
    Ok(Redirect::to("/admin/projects").into_response())
}

// -- checks --
async fn admin_check_new(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(pid): Path<i64>,
) -> Result<Response, AppError> {
    admin_project(&state, pid, &admin, method.as_str(), uri.path()).await?;
    Ok(render(&empty_check_form(
        "New check",
        format!("/admin/projects/{pid}/checks"),
        true,
    ))?
    .into_response())
}

async fn admin_check_create(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(pid): Path<i64>,
    Form(form): Form<CheckForm>,
) -> Result<Response, AppError> {
    admin_project(&state, pid, &admin, method.as_str(), uri.path()).await?;
    check_create_core(&state, pid, form, true, true).await
}

async fn admin_check_show(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let check = admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    render_check_page(&state, check, true, true).await
}

async fn admin_check_edit(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let check = admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    Ok(render(&check_edit_form(check, true, true))?.into_response())
}

async fn admin_check_update(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Form(form): Form<CheckForm>,
) -> Result<Response, AppError> {
    admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    check_update_core(&state, id, form, true, true).await
}

async fn admin_check_pause(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    state.store.set_status(id, CheckStatus::Paused).await?;
    Ok(Redirect::to(&format!("/admin/checks/{id}")).into_response())
}

async fn admin_check_resume(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    state.store.set_status(id, CheckStatus::New).await?;
    Ok(Redirect::to(&format!("/admin/checks/{id}")).into_response())
}

async fn admin_check_ack(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    state.store.acknowledge(id).await?;
    Ok(Redirect::to(&format!("/admin/checks/{id}")).into_response())
}

async fn admin_check_regenerate(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    state
        .store
        .regenerate_uuid(id, &uuid::Uuid::new_v4().to_string())
        .await?;
    Ok(Redirect::to(&format!("/admin/checks/{id}")).into_response())
}

async fn admin_check_delete(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let check = admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    state.store.delete_check(id).await?;
    Ok(Redirect::to(&format!("/admin/projects/{}", check.project_id)).into_response())
}

async fn admin_check_set_channels(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    HtmlForm(form): HtmlForm<BindForm>,
) -> Result<Response, AppError> {
    let check = admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    set_channels_core(&state, &check, form, true).await
}

// -- channels --
async fn admin_channel_new(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(pid): Path<i64>,
) -> Result<Response, AppError> {
    admin_project(&state, pid, &admin, method.as_str(), uri.path()).await?;
    Ok(render(&ChannelFormTemplate {
        show_nav: true,
        is_admin: true,
        admin: true,
        project_id: pid,
        error: None,
        smtp_available: state.config.smtp.is_some(),
    })?
    .into_response())
}

async fn admin_channel_create(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(pid): Path<i64>,
    Form(form): Form<ChannelForm>,
) -> Result<Response, AppError> {
    admin_project(&state, pid, &admin, method.as_str(), uri.path()).await?;
    channel_create_core(&state, pid, form, true, true).await
}

async fn admin_channel_delete(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let channel = admin_channel(&state, id, &admin, method.as_str(), uri.path()).await?;
    state.store.delete_channel(id).await?;
    Ok(Redirect::to(&format!("/admin/projects/{}", channel.project_id)).into_response())
}

async fn admin_channel_test(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let channel = admin_channel(&state, id, &admin, method.as_str(), uri.path()).await?;
    let project = state
        .store
        .find_project(channel.project_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let result = run_channel_test(&state, &channel).await;
    render_project_page(&state.store, project, Some(result), true, true).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_check() -> Check {
        Check {
            id: 1,
            project_id: 1,
            name: "c".into(),
            ping_uuid: "u".into(),
            schedule_kind: ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            cron_expr: None,
            timezone: "UTC".into(),
            status: CheckStatus::Down,
            last_ping_at: None,
            last_start_at: None,
            next_due_at: None,
            scan_interval_secs: None,
            max_runtime_secs: None,
            nag_interval_secs: None,
            last_alert_at: None,
            acknowledged: false,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn status_since_label_down_never_pinged_reads_no_pings_yet() {
        let c = base_check();
        assert_eq!(
            status_since_label(&c, Utc::now()),
            "down · no pings yet · not acknowledged"
        );
    }

    #[test]
    fn status_since_label_down_with_ping_shows_relative_time() {
        let mut c = base_check();
        c.last_ping_at = Some(Utc::now() - Duration::seconds(120));
        assert_eq!(
            status_since_label(&c, Utc::now()),
            "down · 2m ago · not acknowledged"
        );
    }
}
