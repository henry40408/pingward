use crate::auth::{
    hash_password, new_session_token, verify_password, SESSION_COOKIE, SESSION_TTL_DAYS,
};
use crate::error::AppError;
use crate::models::{Check, Project};
use crate::state::AppState;
use crate::store::Store;
use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::post;
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
