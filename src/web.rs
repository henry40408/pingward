use crate::auth::{
    hash_password, new_session_token, verify_password, CurrentUser, SESSION_COOKIE,
    SESSION_TTL_DAYS,
};
use crate::error::AppError;
use crate::models::{Channel, Check, Project};
use crate::state::AppState;
use crate::store::Store;
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{Duration, Utc};
use serde::Deserialize;

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

// NOTE on `Option<CurrentUser>`: the brief's primary approach was to make
// `dashboard` take `user: Option<CurrentUser>` and rely on axum's blanket
// `OptionalFromRequestParts` impl (available when `Rejection: IntoResponse`,
// which `CurrentUser`'s `Response` rejection satisfies). That did not
// compile — `fn(State<AppState>, Option<CurrentUser>) -> ...` did not
// satisfy `Handler<_, _>` under the pinned axum 0.8.9 — so this uses the
// brief's stated fallback: read the session cookie directly via a
// `CookieJar` extractor and `Store::find_session_user`.
async fn dashboard(State(state): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    if state.store.count_users().await? == 0 {
        return Ok(Redirect::to("/setup").into_response());
    }
    let user = match jar.get(SESSION_COOKIE) {
        Some(cookie) => {
            state
                .store
                .find_session_user(cookie.value(), Utc::now())
                .await?
        }
        None => None,
    };
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
