use crate::auth::{
    AdminUser, CurrentUser, OptionalUser, SESSION_IDLE_TTL_HOURS, hash_password, new_session_token,
    verify_password,
};
use crate::error::AppError;
use crate::models::{
    Channel, ChannelKind, Check, CheckStatus, Notification, Project, ScheduleKind, User,
};
use crate::notify::{EventDetail, EventKind, NotificationEvent, notifier_for};
use crate::secret;
use crate::state::AppState;
use crate::store::{AuditFilter, NotifFilter, PageCursor, PingFilter, Store};
use askama::Template;
use axum::extract::{FromRequestParts, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use axum_extra::extract::Form as HtmlForm;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{DateTime, Duration, Utc};
use cron::Schedule;
use serde::Deserialize;
use std::convert::Infallible;
use std::net::IpAddr;
use std::str::FromStr;
use tokio::sync::broadcast;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

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
        .route("/checks/{id}/pings", get(check_pings))
        .route("/checks/{id}/events", get(check_events))
        .route("/checks/{id}/notifications", get(check_notifications))
        .route("/checks/{id}/edit", get(check_edit))
        .route("/checks/{id}/pause", post(check_pause))
        .route("/checks/{id}/resume", post(check_resume))
        .route("/checks/{id}/ack", post(check_ack))
        .route("/checks/{id}/regenerate", post(check_regenerate))
        .route("/checks/{id}/delete", post(check_delete))
        .route("/projects/{pid}/channels/new", get(channel_new))
        .route("/projects/{pid}/channels", post(channel_create))
        .route("/channels/{id}/edit", get(channel_edit))
        .route("/channels/{id}", post(channel_update))
        .route("/channels/{id}/delete", post(channel_delete))
        .route("/channels/{id}/test", post(channel_test))
        .route("/checks/{id}/channels", post(check_set_channels))
        .route("/account", get(account_page))
        .route("/account/password", post(account_password))
        .route("/account/api-keys", post(api_keys_create))
        .route("/account/api-keys/{id}/delete", post(api_keys_delete))
        .route("/account/sessions/{handle}/revoke", post(sessions_revoke))
        .route(
            "/account/sessions/revoke-others",
            post(sessions_revoke_others),
        )
        // --- admin cross-user routes (every handler guarded by AdminUser) ---
        .route("/admin", get(admin_page))
        .route("/admin/audit", get(admin_audit_fragment))
        .route("/admin/settings", post(settings_save))
        .route("/admin/unlock", get(admin_unlock_page).post(admin_unlock))
        .route("/admin/users", post(users_create))
        .route("/admin/users/{id}/delete", post(users_delete))
        .route("/admin/users/{id}/password", post(users_set_password))
        .route("/admin/users/{id}/admin", post(users_toggle_admin))
        .route("/admin/users/{id}/disabled", post(users_set_disabled))
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
        .route(
            "/admin/checks/{id}/ping-url",
            post(admin_check_reveal_ping_url),
        )
        .route("/admin/checks/{id}/pings", get(admin_check_pings))
        .route("/admin/checks/{id}/events", get(admin_check_events))
        .route(
            "/admin/checks/{id}/notifications",
            get(admin_check_notifications),
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
        .route("/admin/channels/{id}/edit", get(admin_channel_edit))
        .route("/admin/channels/{id}", post(admin_channel_update))
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
    csrf: String,
    is_admin: bool,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    show_nav: bool,
    csrf: String,
    is_admin: bool,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    show_nav: bool,
    csrf: String,
    is_admin: bool,
    total: usize,
    /// Includes `Running`: an in-flight run is still up, so the two share a tile.
    up: usize,
    late: usize,
    down: usize,
    groups: Vec<ProjectGroup>,
    q: String,
    status: String,
    /// Forward-auth logout with no gateway URL: tell the visitor to sign out at the proxy.
    forward_auth_logout: Option<String>,
}

/// Absent, blank, or whitespace-only means "no filter".
#[derive(Deserialize, Default)]
struct DashboardQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// Narrower than [`DisplayStatus`]: it mirrors the summary tiles, so `Up` folds
/// in `Running` and `Paused`/`New` have no entry at all.
#[derive(Clone, Copy)]
enum StatusFilter {
    Up,
    Late,
    Down,
}

impl StatusFilter {
    /// An unknown value is "no filter", degrading to the full list rather than a 400.
    fn parse(s: &str) -> Option<Self> {
        match s {
            "up" => Some(Self::Up),
            "late" => Some(Self::Late),
            "down" => Some(Self::Down),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Late => "late",
            Self::Down => "down",
        }
    }

    fn matches(self, ds: crate::view::DisplayStatus) -> bool {
        use crate::view::DisplayStatus;
        match self {
            Self::Up => matches!(ds, DisplayStatus::Up | DisplayStatus::Running),
            Self::Late => matches!(ds, DisplayStatus::Late),
            Self::Down => matches!(ds, DisplayStatus::Down),
        }
    }
}

/// `needle` must be lowercased. In Rust, not SQL: `LIKE` case-sensitivity differs
/// between `SQLite` and Postgres, and the `Any` driver does not translate `ILIKE`.
fn matches_term(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(needle)
}

/// `Option` ordering (`Some(_) > None`) makes an in-flight start count.
fn last_activity_at(c: &Check) -> Option<DateTime<Utc>> {
    c.last_ping_at.max(c.last_start_at)
}

/// Most recent activity first; never-pinged last, ties by creation order.
fn sort_checks_by_activity(checks: &mut [Check]) {
    checks.sort_by(|a, b| {
        last_activity_at(b)
            .cmp(&last_activity_at(a))
            .then(a.id.cmp(&b.id))
    });
}

/// Case-insensitive so `Web` and `api` interleave rather than split on byte value.
fn sort_projects_by_name(projects: &mut [Project]) {
    projects.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
}

struct CheckRow {
    id: i64,
    name: String,
    status: &'static str,
    schedule: String, // e.g. "every 1h · 10m grace" or the cron expr
    last: String,
    bars: Vec<crate::view::Bar>,
    description: String, // markdown::truncate_plain, single-line summary
    /// No bound channels: rendered as a chip, so a check nobody is alerted for shows.
    no_channel: bool,
}

struct ProjectGroup {
    id: i64,
    name: String,
    count: usize,
    checks: Vec<CheckRow>,
    description: String, // markdown::truncate_plain, single-line summary
}

/// Formatted with `duration::fmt_duration`, so it matches what the form accepts.
pub(crate) fn schedule_label(c: &Check) -> String {
    let grace = crate::duration::fmt_duration(c.grace_secs);
    match c.schedule_kind {
        ScheduleKind::Period => match c.period_secs {
            Some(s) => format!(
                "every {} · {} grace",
                crate::duration::fmt_duration(s),
                grace
            ),
            None => format!("{grace} grace"),
        },
        ScheduleKind::Cron => match &c.cron_expr {
            Some(expr) => format!("{expr} · {grace} grace"),
            None => format!("{grace} grace"),
        },
    }
}

// --- forms ---
#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
}

// --- handlers ---
async fn setup_page(State(state): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    if state.store.count_users().await? > 0 {
        return Ok(Redirect::to("/login").into_response());
    }
    Ok(render(&SetupTemplate {
        show_nav: false,
        csrf: current_csrf(&state, &jar),
        is_admin: false,
        error: None,
    })?
    .into_response())
}

// Not rate-limited: `/setup` closes once a user exists; nothing to brute-force.
async fn setup_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    conn: crate::ping::ClientIp,
    Form(creds): Form<Credentials>,
) -> Result<Response, AppError> {
    if state.store.count_users().await? > 0 {
        return Ok(Redirect::to("/login").into_response());
    }
    let policy = crate::auth::validate_password(&creds.password);
    if creds.username.is_empty() || policy.is_err() {
        let error = if creds.username.is_empty() {
            "username and password are required".to_string()
        } else {
            policy.unwrap_err()
        };
        return Ok(render(&SetupTemplate {
            show_nav: false,
            csrf: current_csrf(&state, &jar),
            is_admin: false,
            error: Some(error),
        })?
        .into_response());
    }
    // argon2's error is not `std::error::Error`, so box its `Display` text.
    let phc = hash_password(&creds.password).map_err(|e| AppError::Other(e.to_string().into()))?;
    // Two racing first visitors both pass the check above; tell the loser, don't 500.
    let uid = match state
        .store
        .create_user(&creds.username, Some(&phc), true, Utc::now())
        .await
    {
        Ok(id) => id,
        Err(crate::store::CreateUserError::UsernameTaken) => {
            return Ok(render(&SetupTemplate {
                show_nav: false,
                csrf: current_csrf(&state, &jar),
                is_admin: false,
                error: Some(username_taken(&creds.username)),
            })?
            .into_response());
        }
        Err(crate::store::CreateUserError::Db(e)) => return Err(e.into()),
    };
    let ua = request_user_agent(&headers);
    let jar = start_session(&state, jar, uid, ua.as_deref(), conn.0.as_deref(), false).await?;
    Ok((jar, Redirect::to("/")).into_response())
}

/// Bounces a signed-in visitor to the dashboard — under forward auth,
/// `forward_auth_session` mints a session from the gateway header before this runs.
async fn login_page(
    State(state): State<AppState>,
    jar: CookieJar,
    OptionalUser(user): OptionalUser,
) -> Result<Response, AppError> {
    if state.store.count_users().await? == 0 {
        return Ok(Redirect::to("/setup").into_response());
    }
    if user.is_some() {
        return Ok(Redirect::to("/").into_response());
    }
    Ok(render(&LoginTemplate {
        show_nav: false,
        csrf: current_csrf(&state, &jar),
        is_admin: false,
        error: None,
    })?
    .into_response())
}

/// One `login.failed` event discriminated by `reason`: the operator's only spray signal.
/// `username` is attacker-chosen, hence [`crate::auth::log_username`] via `Debug`.
fn log_login_failure(
    username: &str,
    ip: Option<&str>,
    bucket: std::net::IpAddr,
    reason: &'static str,
) {
    tracing::warn!(
        target: "pingward::auth",
        username = ?crate::auth::log_username(username),
        ip,
        bucket = %bucket,
        reason,
        "login.failed"
    );
}

enum Reauth {
    /// Verified, or not required because the account has no local password.
    Passed,
    Failed,
    Throttled,
}

/// Re-demands the user's password before a sensitive action (threat: a borrowed session).
/// A passwordless forward-auth account passes — nothing to verify. Attempts charge the
/// account limiter, or this is an unmetered password oracle; success clears the bucket.
fn reauthenticate(state: &AppState, user: &User, submitted: &str, surface: &'static str) -> Reauth {
    let Some(stored) = user.password_hash.as_deref() else {
        return Reauth::Passed;
    };
    let key = crate::ratelimit::account_key(&user.username);
    if !state.account_limiter.try_acquire(key.clone()) {
        log_reauth_failure(user, surface, "rate_limited");
        return Reauth::Throttled;
    }
    if verify_password(submitted, stored) {
        state.account_limiter.clear(&key);
        Reauth::Passed
    } else {
        log_reauth_failure(user, surface, "bad_current_password");
        Reauth::Failed
    }
}

/// Handle (hash) of the verified session *id*, not the raw cookie value; the id is a
/// bearer secret, so the handle names a session everywhere outside the cookie.
fn current_session_handle(state: &AppState, jar: &CookieJar) -> Option<String> {
    secret::session_id_from_jar(jar, &state.config.secret, session_cookie_name(state))
        .map(|id| crate::apikey::hash_api_key(&id))
}

struct Elevation {
    /// No local password, so there is nothing to re-assert — see [`reauthenticate`].
    not_applicable: bool,
    remaining_secs: Option<u64>,
}

impl Elevation {
    /// Whether an access-granting admin action may proceed.
    fn allows(&self) -> bool {
        self.not_applicable || self.remaining_secs.is_some()
    }
}

fn elevation(state: &AppState, jar: &CookieJar, user: &User) -> Elevation {
    if user.password_hash.is_none() {
        return Elevation {
            not_applicable: true,
            remaining_secs: None,
        };
    }
    Elevation {
        not_applicable: false,
        remaining_secs: current_session_handle(state, jar)
            .and_then(|h| state.elevations.remaining_secs(&h)),
    }
}

/// Bounces to the `/admin/unlock` interstitial rather than a 403; the refused action
/// is not replayed after unlocking.
fn admin_locked(config: &crate::config::Config, jar: CookieJar) -> Response {
    let jar = jar.add(flash_cookie(config, "admin_locked"));
    (jar, Redirect::to("/admin/unlock")).into_response()
}

// --- confirming a destructive action (see ARCHITECTURE.md) ---
// `data-confirm` is inert without script, so a handler runs only with `?confirmed=1` and
// otherwise renders `ConfirmTemplate`. A *query* flag: a body extractor would 415 the
// no-body forms before authorization, turning `owned_check`'s 404 into a type error.

/// Page copy, deliberately longer than the template's terse `data-confirm` line.
struct Confirm {
    title: &'static str,
    message: &'static str,
    button: &'static str,
}

const CONFIRM_DELETE_PROJECT: Confirm = Confirm {
    title: "Delete this project?",
    message: "Deleting a project deletes everything inside it: every check it holds, their ping history, and the notification channels configured on it. Any job still pinging one of those checks will start getting 404s. This cannot be undone.",
    button: "Delete project",
};

const CONFIRM_DELETE_CHECK: Confirm = Confirm {
    title: "Delete this check?",
    message: "The check and its whole ping history go away, and its ping URL stops answering — a job still calling it will start getting 404s. This cannot be undone.",
    button: "Delete check",
};

const CONFIRM_REGENERATE_URL: Confirm = Confirm {
    title: "Regenerate this check's ping URL?",
    message: "The current URL stops working immediately. Every job that pings this check must be updated to the new URL, or the check will go down when its next ping never arrives.",
    button: "Regenerate URL",
};

const CONFIRM_REVOKE_SESSION: Confirm = Confirm {
    title: "Revoke this session?",
    message: "The browser holding it is signed out at once. If it is the session you are using now, that includes this one.",
    button: "Revoke session",
};

const CONFIRM_REVOKE_OTHER_SESSIONS: Confirm = Confirm {
    title: "Revoke every other session?",
    message: "Every other signed-in browser is signed out at once. Only this one stays. API keys are unaffected — they are separate credentials, revoked from the API keys card below.",
    button: "Revoke the others",
};

const CONFIRM_REVOKE_API_KEY: Confirm = Confirm {
    title: "Revoke this API key?",
    message: "Anything using this key stops being able to reach the API immediately. The key cannot be recovered — a replacement is a new key with a new token.",
    button: "Revoke key",
};

const CONFIRM_DELETE_USER: Confirm = Confirm {
    title: "Delete this user?",
    message: "The account goes away along with everything it owns: its projects, their checks, and the whole ping history behind them. This cannot be undone. To cut off access while keeping the data, disable the account instead.",
    button: "Delete user",
};

const CONFIRM_DEMOTE_ADMIN: Confirm = Confirm {
    title: "Revoke this user's admin rights?",
    message: "They keep their account and their own projects, but lose access to /admin and to every other user's data.",
    button: "Revoke admin rights",
};

const CONFIRM_DISABLE_USER: Confirm = Confirm {
    title: "Disable this user?",
    message: "They cannot sign in again until the account is re-enabled. Nothing they own is deleted, and their checks keep running and alerting.",
    button: "Disable user",
};

/// Infallible: no query string means "not confirmed", not a 400.
#[derive(Deserialize, Default)]
struct ConfirmQuery {
    #[serde(default)]
    confirmed: Option<String>,
}

impl ConfirmQuery {
    fn is_confirmed(&self) -> bool {
        self.confirmed.is_some()
    }
}

#[derive(Template)]
#[template(path = "confirm.html")]
struct ConfirmTemplate {
    show_nav: bool,
    is_admin: bool,
    csrf: String,
    title: &'static str,
    message: &'static str,
    button: &'static str,
    action: String,
    cancel: String,
}

/// Re-posts `action` plus the flag. Call after authorization and every refusal guard:
/// a request that can never succeed should say so, not ask for confirmation.
fn confirmation_page(
    state: &AppState,
    jar: &CookieJar,
    is_admin: bool,
    confirm: &Confirm,
    action: &str,
    cancel: String,
) -> Result<Response, AppError> {
    let sep = if action.contains('?') { '&' } else { '?' };
    Ok(render(&ConfirmTemplate {
        show_nav: true,
        is_admin,
        csrf: current_csrf(state, jar),
        title: confirm.title,
        message: confirm.message,
        button: confirm.button,
        action: format!("{action}{sep}confirmed=1"),
        cancel,
    })?
    .into_response())
}

/// One `reauth.failed` event discriminated by `surface`. Separate from `login.failed`,
/// which is unauthenticated and carries an address instead of a `user_id`.
fn log_reauth_failure(user: &User, surface: &'static str, reason: &'static str) {
    tracing::warn!(
        target: "pingward::auth",
        username = ?crate::auth::log_username(&user.username),
        user_id = user.id,
        surface,
        reason,
        "reauth.failed"
    );
}

/// Shared by both login limiters so the wording never hints the username is real;
/// only `Retry-After` differs.
fn throttled_login(
    state: &AppState,
    jar: &CookieJar,
    retry_after_secs: u64,
) -> Result<Response, AppError> {
    Ok((
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, retry_after_secs.to_string())],
        render(&LoginTemplate {
            show_nav: false,
            csrf: current_csrf(state, jar),
            is_admin: false,
            error: Some("too many login attempts — try again later".into()),
        })?,
    )
        .into_response())
}

async fn login_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    conn: crate::ping::ClientIp,
    PeerAddr(peer_ip): PeerAddr,
    Form(creds): Form<Credentials>,
) -> Result<Response, AppError> {
    // Keyed by `rate_limit_key`, not `conn` (see there). Before the lookup, so a
    // throttled request never pays for argon2.
    let client = crate::ratelimit::rate_limit_key(peer_ip, &headers, &state.config.trusted_proxies);
    if !state.login_limiter.try_acquire(client) {
        log_login_failure(&creds.username, conn.0.as_deref(), client, "rate_limited");
        return throttled_login(&state, &jar, crate::ratelimit::WINDOW_SECS);
    }
    // Per account, or N addresses buy `MAX_ATTEMPTS × N` guesses. Keyed on the
    // *submitted* name before lookup, so an invented name throttles identically.
    let account = crate::ratelimit::account_key(&creds.username);
    if !state.account_limiter.try_acquire(account.clone()) {
        log_login_failure(&creds.username, conn.0.as_deref(), client, "account_locked");
        return throttled_login(&state, &jar, crate::ratelimit::ACCOUNT_WINDOW_SECS);
    }
    let user = state.store.find_user_by_username(&creds.username).await?;
    // An unknown username must still cost one argon2 verification (timing oracle).
    let ok = crate::auth::verify_password_or_dummy(
        &creds.password,
        user.as_ref().and_then(|u| u.password_hash.as_deref()),
    );
    if !ok {
        log_login_failure(
            &creds.username,
            conn.0.as_deref(),
            client,
            "bad_credentials",
        );
        return Ok(render(&LoginTemplate {
            show_nav: false,
            csrf: current_csrf(&state, &jar),
            is_admin: false,
            error: Some("invalid username or password".into()),
        })?
        .into_response());
    }
    let user = user.unwrap();
    if user.disabled {
        // Not released, or a known-disabled credential could probe the budget forever.
        log_login_failure(
            &creds.username,
            conn.0.as_deref(),
            client,
            "account_disabled",
        );
        return Ok(render(&LoginTemplate {
            show_nav: false,
            csrf: current_csrf(&state, &jar),
            is_admin: false,
            error: Some("account is disabled".into()),
        })?
        .into_response());
    }
    let ua = request_user_agent(&headers);
    let jar = start_session(
        &state,
        jar,
        user.id,
        ua.as_deref(),
        conn.0.as_deref(),
        false,
    )
    .await?;
    // Success refunds the IP reservation and *clears* the account bucket.
    state.login_limiter.release(&client);
    state.account_limiter.clear(&account);
    Ok((jar, Redirect::to("/")).into_response())
}

/// Behind a gateway the next request re-mints the session from the identity header, so
/// redirect to `PINGWARD_FORWARD_AUTH_LOGOUT_URL`, or (unset, header present) flash that
/// on the dashboard. Targets never come from the request. See ARCHITECTURE.md.
async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    PeerAddr(peer_ip): PeerAddr,
) -> Result<Response, AppError> {
    if let Some(id) =
        secret::session_id_from_jar(&jar, &state.config.secret, session_cookie_name(&state))
    {
        state.store.delete_session(&id).await?;
        // Or the dead session's elevation lingers until its window elapses.
        state.elevations.revoke(&crate::apikey::hash_api_key(&id));
        tracing::info!(
            target: "pingward::session",
            reason = "logout",
            handle = %crate::auth::session_log_handle(&id),
            "session.destroyed"
        );
    }
    let jar = jar.remove(session_removal_cookie(&state.config));

    // A configured gateway logout URL ends the upstream identity too.
    if let Some(url) = state.config.forward_auth_logout_url.as_deref() {
        return Ok((
            jar,
            [(HeaderName::from_static("clear-site-data"), CLEAR_SITE_DATA)],
            Redirect::to(url),
        )
            .into_response());
    }

    // Header present, no logout URL: the gateway re-mints the session, so say so.
    if crate::auth::forward_auth_username(&headers, peer_ip, &state.config).is_some() {
        // No Clear-Site-Data: this exit's job is delivering the flash cookie.
        let jar = jar.add(flash_cookie(&state.config, "forward_auth_logout"));
        return Ok((jar, Redirect::to("/")).into_response());
    }

    Ok((
        jar,
        [(HeaderName::from_static("clear-site-data"), CLEAR_SITE_DATA)],
        Redirect::to("/login"),
    )
        .into_response())
}

/// Cache only: `"cookies"` spans the registered domain and would clear the gateway's
/// cookie before its logout redirect; `"storage"` holds only the theme;
/// `"executionContexts"` forces a reload that fights the redirect. No-op over plain HTTP.
const CLEAR_SITE_DATA: &str = r#""cache""#;

/// `None` without `ConnectInfo` (e.g. tests). Same source `forward_auth_session` reads,
/// so `logout`'s trusted-proxy check agrees with how the visitor was authenticated.
struct PeerAddr(Option<IpAddr>);

impl FromRequestParts<AppState> for PeerAddr {
    type Rejection = Infallible;
    fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &AppState,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Ok(Self(crate::auth::peer_ip(&parts.extensions))))
    }
}

async fn dashboard(
    State(state): State<AppState>,
    jar: CookieJar,
    OptionalUser(user): OptionalUser,
    Query(query): Query<DashboardQuery>,
) -> Result<Response, AppError> {
    if state.store.count_users().await? == 0 {
        return Ok(Redirect::to("/setup").into_response());
    }
    let Some(user) = user else {
        return Ok(Redirect::to("/login").into_response());
    };
    // Consumed here so the removal Set-Cookie rides back on this response.
    let (jar, forward_auth_logout) = take_flash(&state.config, jar, "forward_auth_logout");
    let now = Utc::now();
    let q = query.q.unwrap_or_default().trim().to_string();
    let needle = q.to_lowercase();
    let status_raw = query.status.unwrap_or_default();
    let status_filter = StatusFilter::parse(&status_raw);
    // Echo only a recognised value, so garbage neither pre-selects nor shows "clear".
    let status = status_filter.map_or("", StatusFilter::as_str).to_string();
    let (mut total, mut up, mut late, mut down) = (0usize, 0, 0, 0);
    let mut groups = Vec::new();
    // Filter before the batched ping fetch, so a narrow filter narrows the query.
    let mut project_checks = Vec::new();
    let mut check_ids = Vec::new();
    // Ordered here: the `Store` queries are shared with callers wanting id order.
    let mut projects = state.store.list_projects_for_user(user.id).await?;
    sort_projects_by_name(&mut projects);
    let project_ids: Vec<i64> = projects.iter().map(|p| p.id).collect();
    let mut checks_by_project = state.store.list_checks_for_projects(&project_ids).await?;
    for project in projects {
        let mut checks = checks_by_project.remove(&project.id).unwrap_or_default();
        sort_checks_by_activity(&mut checks);
        let checks = if needle.is_empty()
            || matches_term(&project.name, &needle)
            || matches_term(&project.description, &needle)
        {
            // A project-level hit shows the whole project.
            checks
        } else {
            let kept: Vec<Check> = checks
                .into_iter()
                .filter(|c| matches_term(&c.name, &needle) || matches_term(&c.description, &needle))
                .collect();
            if kept.is_empty() {
                continue;
            }
            kept
        };
        check_ids.extend(checks.iter().map(|c| c.id));
        project_checks.push((project, checks));
    }
    let pings_by_check = state
        .store
        .list_recent_ping_summaries_for_checks(&check_ids, 40)
        .await?;
    let with_channels = state.store.checks_with_channels(&check_ids).await?;
    for (project, checks) in project_checks {
        let mut rows = Vec::with_capacity(checks.len());
        for c in &checks {
            let ds = crate::view::display_status(c, now);
            // Tiles count the `q`-filtered set regardless of the status filter.
            total += 1;
            match ds {
                crate::view::DisplayStatus::Up | crate::view::DisplayStatus::Running => up += 1,
                crate::view::DisplayStatus::Late => late += 1,
                crate::view::DisplayStatus::Down => down += 1,
                _ => {}
            }
            if let Some(sf) = status_filter
                && !sf.matches(ds)
            {
                continue;
            }
            let empty = Vec::new();
            let pings = pings_by_check.get(&c.id).unwrap_or(&empty);
            let bars = crate::view::heartbeat(
                pings,
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
                    .map_or_else(|| "—".into(), |t| crate::view::fmt_relative(t, now)),
                bars,
                description: crate::markdown::truncate_plain(&c.description, 120),
                no_channel: !with_channels.contains(&c.id),
            });
        }
        // Drop a project whose checks were all filtered out (still counted above).
        if status_filter.is_some() && rows.is_empty() {
            continue;
        }
        groups.push(ProjectGroup {
            id: project.id,
            description: crate::markdown::truncate_plain(&project.description, 120),
            name: project.name,
            count: rows.len(),
            checks: rows,
        });
    }
    let resp = render(&DashboardTemplate {
        show_nav: true,
        csrf: current_csrf(&state, &jar),
        is_admin: user.is_admin,
        total,
        up,
        late,
        down,
        groups,
        q,
        status,
        forward_auth_logout,
    })?;
    Ok((jar, resp).into_response())
}

/// More bars than fit: `assets/app.css` clips from the *left*, newest pinned right.
/// 120 because `.beat i` is 10px with its gap and `.wrap` caps at 1080px.
const HEARTBEAT_BARS: usize = 120;

/// Don't narrow to `kind IN ('success','fail')`: `view::run_durations` pairs each finish
/// with its preceding `start`. Two rows per bar, plus headroom.
const HEARTBEAT_WINDOW: i64 = 300;

/// A raw browser header is unbounded and the value is display-only, so truncate.
const MAX_USER_AGENT_CHARS: usize = 300;

fn request_user_agent(headers: &HeaderMap) -> Option<String> {
    headers.get(axum::http::header::USER_AGENT).and_then(|v| {
        v.to_str().ok().map(|s| {
            let end = s
                .char_indices()
                .nth(MAX_USER_AGENT_CHARS)
                .map_or(s.len(), |(i, _)| i);
            s[..end].to_string()
        })
    })
}

fn session_cookie_name(state: &AppState) -> &'static str {
    crate::auth::session_cookie_name(state.config.cookie_secure)
}

/// Attributes must match [`session_removal_cookie`]: a removal cookie whose attributes
/// differ can fail to overwrite the original.
fn session_cookie(config: &crate::config::Config, value: String) -> Cookie<'static> {
    Cookie::build((
        crate::auth::session_cookie_name(config.cookie_secure),
        value,
    ))
    .http_only(true)
    .same_site(SameSite::Lax)
    .path("/")
    .secure(config.cookie_secure)
    // No Max-Age/Expires: non-persistent by design; expiry is `sessions.expires_at`.
    .build()
}

/// Attributes must stay aligned with [`session_cookie`].
fn session_removal_cookie(config: &crate::config::Config) -> Cookie<'static> {
    session_cookie(config, String::new())
}

async fn open_session(
    state: &AppState,
    user_id: i64,
    user_agent: Option<&str>,
    ip: Option<&str>,
    sso: bool,
) -> Result<Cookie<'static>, AppError> {
    let session_id = new_session_token();
    let expires = Utc::now() + Duration::hours(SESSION_IDLE_TTL_HOURS);
    state
        .store
        .create_session(
            &session_id,
            user_id,
            expires,
            user_agent,
            ip,
            sso,
            Utc::now(),
        )
        .await?;
    tracing::info!(
        target: "pingward::session",
        handle = %crate::auth::session_log_handle(&session_id),
        user_id,
        sso,
        ip,
        user_agent,
        expires_at = %expires.to_rfc3339(),
        "session.created"
    );
    // The cookie carries `<id>.<hmac>`, never the bare id — see `crate::secret`.
    Ok(session_cookie(
        &state.config,
        secret::sign_session(&state.config.secret, &session_id),
    ))
}

async fn start_session(
    state: &AppState,
    jar: CookieJar,
    user_id: i64,
    user_agent: Option<&str>,
    ip: Option<&str>,
    sso: bool,
) -> Result<CookieJar, AppError> {
    Ok(jar.add(open_session(state, user_id, user_agent, ip, sso).await?))
}

/// Gives every visitor a signed session cookie with no row (the CSRF token derives from
/// the id), so [`csrf_guard`] needs no path exemptions. Layered *inside*
/// [`forward_auth_session`] (see `crate::app`) so it can't shadow a real session.
pub async fn anonymous_session(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let jar = CookieJar::from_headers(req.headers());
    if secret::session_id_from_jar(&jar, &state.config.secret, session_cookie_name(&state))
        .is_some()
    {
        return next.run(req).await;
    }
    // Signature only: an anonymous id has no row to look up.
    let cookie = session_cookie(
        &state.config,
        secret::sign_session(&state.config.secret, &new_session_token()),
    );
    replace_request_cookie(&mut req, &cookie);
    let mut resp = next.run(req).await;
    if let Ok(value) = cookie.to_string().parse() {
        resp.headers_mut().append(header::SET_COOKIE, value);
    }
    resp
}

/// Gives a trusted forward-auth identity a real session so nothing needs a special case.
/// Layered *outside* [`anonymous_session`] and [`csrf_guard`]; the cookie is injected into
/// the request too, so forms rendered now derive a matching token. The short-circuit
/// checks liveness: an anonymous cookie is validly signed but has no row.
pub async fn forward_auth_session(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    if state.config.forward_auth_header.is_none() {
        return next.run(req).await;
    }
    let now = Utc::now();
    let jar = CookieJar::from_headers(req.headers());
    if let Some(id) =
        secret::session_id_from_jar(&jar, &state.config.secret, session_cookie_name(&state))
        && matches!(state.store.find_session_user(&id, now).await, Ok(Some(_)))
    {
        return next.run(req).await;
    }
    let peer = crate::auth::peer_ip(req.extensions());
    let Some(user) = crate::auth::forward_auth_user(&state, req.headers(), peer, now).await else {
        return next.run(req).await;
    };
    let ua = request_user_agent(req.headers());
    let ip = crate::auth::client_ip(req.headers(), peer, &state.config);
    let cookie = match open_session(&state, user.id, ua.as_deref(), ip.as_deref(), true).await {
        Ok(cookie) => cookie,
        Err(e) => {
            tracing::error!("failed to open a session for forward-auth user: {e}");
            return next.run(req).await;
        }
    };
    replace_request_cookie(&mut req, &cookie);
    let mut resp = next.run(req).await;
    if let Ok(value) = cookie.to_string().parse() {
        resp.headers_mut().append(header::SET_COOKIE, value);
    }
    resp
}

/// The stale entry must be dropped, not appended past: `CookieJar::get` returns
/// the first match, so an expired session id would shadow the fresh one.
fn replace_request_cookie(req: &mut Request, cookie: &Cookie<'static>) {
    let prefix = format!("{}=", cookie.name());
    let kept: Vec<String> = req
        .headers()
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .filter(|pair| !pair.is_empty() && !pair.starts_with(&prefix))
        .map(str::to_owned)
        .chain(std::iter::once(format!(
            "{}={}",
            cookie.name(),
            cookie.value()
        )))
        .collect();
    if let Ok(value) = kept.join("; ").parse() {
        req.headers_mut().insert(header::COOKIE, value);
    }
}

/// The `_csrf` form value; empty without a valid session (unsubmittable, not a bypass).
fn current_csrf(state: &AppState, jar: &CookieJar) -> String {
    secret::session_id_from_jar(jar, &state.config.secret, session_cookie_name(state))
        .map(|id| secret::derive_csrf(&state.config.secret, &id))
        .unwrap_or_default()
}

/// Caps what a client can make [`csrf_guard`] buffer; browser forms are far below it.
const CSRF_MAX_BODY_BYTES: usize = 1 << 20;

/// `noisy` (`token_missing`) logs at `debug!`: `csrf_guard` refuses `POST /login` before
/// `login_limiter`, so scanners produce it in bulk. Other reasons mean a token failed.
fn log_csrf_rejection(reason: &'static str, session_id: Option<&str>, noisy: bool) {
    let handle = session_id.map(crate::auth::session_log_handle);
    if noisy {
        tracing::debug!(target: "pingward::auth", reason, handle, "csrf.rejected");
    } else {
        tracing::warn!(target: "pingward::auth", reason, handle, "csrf.rejected");
    }
}

/// Synchronizer-token guard over `web::routes()` only (`/ping/*`, assets, `/healthz` are
/// sibling routers). No path exemptions: [`anonymous_session`] gives everyone a token.
/// Read from `X-CSRF-Token` or the `_csrf` field (body buffered, request rebuilt).
/// The 403 is bodyless so it doesn't tell a scanner what to send.
pub async fn csrf_guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    let jar = CookieJar::from_headers(req.headers());
    let secret = &state.config.secret;
    let Some(session_id) = secret::session_id_from_jar(&jar, secret, session_cookie_name(&state))
    else {
        // Unreachable unless layer ordering broke: `anonymous_session` runs outside.
        log_csrf_rejection("no_session", None, false);
        return StatusCode::FORBIDDEN.into_response();
    };
    // Prefer the header token — this path avoids buffering the body.
    if let Some(submitted) = req
        .headers()
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
    {
        if secret::verify_csrf(secret, &session_id, submitted) {
            return next.run(req).await;
        }
        log_csrf_rejection("header_mismatch", Some(&session_id), false);
        return StatusCode::FORBIDDEN.into_response();
    }
    let (parts, body) = req.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, CSRF_MAX_BODY_BYTES).await else {
        // Too big or truncated; costly to repeat, so it needs no `noisy` demotion.
        log_csrf_rejection("body_unreadable", Some(&session_id), false);
        return StatusCode::FORBIDDEN.into_response();
    };
    let Some(submitted) = form_urlencoded::parse(&bytes)
        .find(|(k, _)| k == "_csrf")
        .map(|(_, v)| v.into_owned())
    else {
        // Never rendered the form; this is the noisy one.
        log_csrf_rejection("token_missing", Some(&session_id), true);
        return StatusCode::FORBIDDEN.into_response();
    };
    if !secret::verify_csrf(secret, &session_id, &submitted) {
        log_csrf_rejection("token_mismatch", Some(&session_id), false);
        return StatusCode::FORBIDDEN.into_response();
    }
    let req = Request::from_parts(parts, axum::body::Body::from(bytes));
    next.run(req).await
}

/// `Cache-Control: no-store` over all of `web::routes()` (`/login`, `/setup` render a
/// cookie-bound `_csrf` too); `api` re-applies it to `/api/docs` and `/api/openapi.json`.
/// A handler's own `Cache-Control` wins.
pub async fn no_store(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    if !resp.headers().contains_key(header::CACHE_CONTROL) {
        resp.headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    resp
}

/// `script-src 'self'` (no `'unsafe-inline'`, no nonce) holds only while every script is
/// an `/assets` file and no template has an inline handler — put behaviour in
/// `assets/app.js`. `style-src 'unsafe-inline'` is for the heartbeat bars' `height:Npx`.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
     script-src 'self'; \
     style-src 'self' 'unsafe-inline'; \
     img-src 'self' data:; \
     font-src 'self'; \
     connect-src 'self'; \
     object-src 'none'; \
     base-uri 'none'; \
     form-action 'self'; \
     frame-ancestors 'none'";

/// `web::routes()` only: `/api/docs` loads Scalar from `cdn.jsdelivr.net`, and admitting
/// that CDN app-wide would weaken every page. It still gets [`security_headers`].
pub async fn content_security_policy(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    if !resp.headers().contains_key(header::CONTENT_SECURITY_POLICY) {
        resp.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        );
    }
    resp
}

/// App-wide, covering what the CSP does not: `nosniff` for JSON/`text/plain` bodies,
/// `X-Frame-Options` for `/api/docs`, `Referrer-Policy` since check URLs identify checks.
const STATIC_SECURITY_HEADERS: &[(&str, &str)] = &[
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "DENY"),
    ("referrer-policy", "same-origin"),
    (
        "permissions-policy",
        "geolocation=(), camera=(), microphone=(), payment=(), usb=()",
    ),
];

pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    for (name, value) in STATIC_SECURITY_HEADERS {
        let name = HeaderName::from_static(name);
        if !headers.contains_key(&name) {
            headers.insert(name, HeaderValue::from_static(value));
        }
    }
    resp
}

/// Off unless `PINGWARD_HSTS_MAX_AGE` is set (TLS terminates at the proxy); app-wide
/// since HSTS covers the origin. No `includeSubDomains`/`preload`: near-irreversible.
pub async fn hsts(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let max_age = state.config.hsts_max_age_secs;
    if max_age > 0
        && let Ok(value) = HeaderValue::from_str(&format!("max-age={max_age}"))
    {
        resp.headers_mut()
            .insert(header::STRICT_TRANSPORT_SECURITY, value);
    }
    resp
}

// --- project templates ---
#[derive(Template)]
#[template(path = "project_form.html")]
struct ProjectFormTemplate {
    show_nav: bool,
    csrf: String,
    is_admin: bool,
    heading: String,
    action: String,
    name: String,
    description: String,
    scan_interval_secs: String,
    nag_interval_secs: String,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "project.html")]
struct ProjectTemplate {
    show_nav: bool,
    csrf: String,
    is_admin: bool,
    admin: bool,
    project: Project,
    description_html: String,
    checks: Vec<ProjectCheckRow>,
    channels: Vec<ProjectChannelRow>,
    test_result: Option<TestResult>,
}

/// `status` is precomputed: it needs `now`.
struct ProjectCheckRow {
    id: i64,
    name: String,
    status: &'static str,
    schedule: String,
    description: String, // markdown::truncate_plain, single-line summary
    /// No bound channels (rendered as a chip).
    no_channel: bool,
}

/// Projected so a [`Channel`]'s `config_json` (delivery secrets) never reaches the
/// template, as with [`ChannelEditView`].
struct ProjectChannelRow {
    id: i64,
    name: String,
    kind: &'static str,
}

struct TestResult {
    ok: bool,
    message: String,
}

#[derive(Deserialize)]
pub(crate) struct ProjectForm {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) scan_interval_secs: String,
    pub(crate) nag_interval_secs: String,
}

/// In characters. Also bounds `markdown::render`'s worst-case O(n²) — raise with care.
const MAX_DESCRIPTION_CHARS: usize = 2000;

/// Trims, then enforces [`MAX_DESCRIPTION_CHARS`].
fn validate_description(s: &str) -> Result<String, String> {
    let trimmed = s.trim();
    if trimmed.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!(
            "description must be at most {MAX_DESCRIPTION_CHARS} characters"
        ));
    }
    Ok(trimmed.to_string())
}

/// Blank is `Ok(None)` (unset: inherit or off); the error names `field`.
fn parse_opt_positive(s: &str, field: &str) -> Result<Option<i64>, String> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    match t.parse::<i64>() {
        Ok(v) if v > 0 => Ok(Some(v)),
        _ => Err(format!("{field} must be a positive integer")),
    }
}

/// As [`parse_opt_positive`], but also accepts `5m` / `1h30m`.
fn parse_opt_positive_duration(s: &str, field: &str) -> Result<Option<i64>, String> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    match crate::duration::parse_duration(t) {
        Some(v) if v > 0 => Ok(Some(v)),
        _ => Err(format!(
            "{field} must be a positive duration (e.g. 30, 5m, 1h30m)"
        )),
    }
}

/// Another user's project is `NotFound`, not `Forbidden`: existence is hidden.
async fn owned_project(store: &Store, id: i64, user_id: i64) -> Result<Project, AppError> {
    let p = store.find_project(id).await?.ok_or(AppError::NotFound)?;
    if p.user_id != user_id {
        return Err(AppError::NotFound);
    }
    Ok(p)
}

/// Only non-GET admin access is audited (page opens would bury what matters; the
/// credential read audits itself as `admin.ping_url_reveal`). Gated in the resolvers
/// below — the choke point for reads *and* writes — so dropping read audits can't
/// silently drop pause/resume/delete/regenerate audits too.
fn audits_as_mutation(method: &str) -> bool {
    !method.eq_ignore_ascii_case("GET")
}

/// No owner filter; the choke point for cross-user project reads and writes.
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
    if audits_as_mutation(method) {
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
    }
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
    if audits_as_mutation(method) {
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
    }
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
    if audits_as_mutation(method) {
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
    }
    Ok(ch)
}

/// Name and description come back trimmed — that is what must be stored.
pub(crate) fn validate_project(
    form: &ProjectForm,
) -> Result<(String, String, Option<i64>, Option<i64>), String> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let description = validate_description(&form.description)?;
    let scan = parse_opt_positive_duration(&form.scan_interval_secs, "scan interval")?;
    let nag = parse_opt_positive_duration(&form.nag_interval_secs, "nag interval")?;
    Ok((name.to_string(), description, scan, nag))
}

/// Preserves the submitted values so the user can fix the invalid field.
fn project_form_with_error(
    heading: &str,
    action: String,
    is_admin: bool,
    csrf: String,
    form: &ProjectForm,
    error: String,
) -> ProjectFormTemplate {
    ProjectFormTemplate {
        show_nav: true,
        csrf,
        is_admin,
        heading: heading.into(),
        action,
        name: form.name.clone(),
        description: form.description.clone(),
        scan_interval_secs: form.scan_interval_secs.clone(),
        nag_interval_secs: form.nag_interval_secs.clone(),
        error: Some(error),
    }
}

async fn project_new(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
) -> Result<Response, AppError> {
    Ok(render(&ProjectFormTemplate {
        show_nav: true,
        csrf: current_csrf(&state, &jar),
        is_admin: user.is_admin,
        heading: "New project".into(),
        action: "/projects".into(),
        name: String::new(),
        description: String::new(),
        scan_interval_secs: String::new(),
        nag_interval_secs: String::new(),
        error: None,
    })?
    .into_response())
}

async fn project_create(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Form(form): Form<ProjectForm>,
) -> Result<Response, AppError> {
    let (name, description, scan, nag) = match validate_project(&form) {
        Ok(v) => v,
        Err(msg) => {
            let csrf = current_csrf(&state, &jar);
            let t = project_form_with_error(
                "New project",
                "/projects".into(),
                user.is_admin,
                csrf,
                &form,
                msg,
            );
            return Ok(render(&t)?.into_response());
        }
    };
    let id = state
        .store
        .create_project(user.id, &name, &description, scan, nag, Utc::now())
        .await?;
    Ok(Redirect::to(&format!("/projects/{id}")).into_response())
}

/// Prefix for links, actions and redirects. Throughout this file `admin` selects the
/// `/admin` surface, while `is_admin` is the viewer's role (only the nav Admin link).
fn admin_prefix(admin: bool) -> &'static str {
    if admin { "/admin" } else { "" }
}

async fn render_project_page(
    store: &Store,
    project: Project,
    test_result: Option<TestResult>,
    admin: bool,
    is_admin: bool,
    csrf: String,
) -> Result<Response, AppError> {
    let now = Utc::now();
    let checks = store.list_checks_for_project(project.id).await?;
    let check_ids: Vec<i64> = checks.iter().map(|c| c.id).collect();
    let with_channels = store.checks_with_channels(&check_ids).await?;
    let checks = checks
        .into_iter()
        .map(|c| ProjectCheckRow {
            id: c.id,
            status: crate::view::display_status(&c, now).as_str(),
            schedule: schedule_label(&c),
            description: crate::markdown::truncate_plain(&c.description, 120),
            no_channel: !with_channels.contains(&c.id),
            name: c.name,
        })
        .collect();
    let channels: Vec<ProjectChannelRow> = store
        .list_channels_for_project(project.id)
        .await?
        .into_iter()
        .map(|c| ProjectChannelRow {
            id: c.id,
            name: c.name,
            kind: c.kind.as_str(),
        })
        .collect();
    let description_html = crate::markdown::render(&project.description);
    Ok(render(&ProjectTemplate {
        show_nav: true,
        csrf,
        is_admin,
        admin,
        project,
        description_html,
        checks,
        channels,
        test_result,
    })?
    .into_response())
}

fn project_edit_form(
    project: Project,
    admin: bool,
    is_admin: bool,
    csrf: String,
) -> ProjectFormTemplate {
    let base = admin_prefix(admin);
    ProjectFormTemplate {
        show_nav: true,
        csrf,
        is_admin,
        heading: "Edit project".into(),
        action: format!("{base}/projects/{}", project.id),
        name: project.name,
        description: project.description,
        scan_interval_secs: project
            .scan_interval_secs
            .map(crate::duration::fmt_duration)
            .unwrap_or_default(),
        nag_interval_secs: project
            .nag_interval_secs
            .map(crate::duration::fmt_duration)
            .unwrap_or_default(),
        error: None,
    }
}

async fn project_show(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = owned_project(&state.store, id, user.id).await?;
    let csrf = current_csrf(&state, &jar);
    render_project_page(&state.store, project, None, false, user.is_admin, csrf).await
}

async fn project_edit(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = owned_project(&state.store, id, user.id).await?;
    let csrf = current_csrf(&state, &jar);
    Ok(render(&project_edit_form(project, false, user.is_admin, csrf))?.into_response())
}

async fn project_update(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<ProjectForm>,
) -> Result<Response, AppError> {
    owned_project(&state.store, id, user.id).await?;
    let (name, description, scan, nag) = match validate_project(&form) {
        Ok(v) => v,
        Err(msg) => {
            let csrf = current_csrf(&state, &jar);
            let t = project_form_with_error(
                "Edit project",
                format!("/projects/{id}"),
                user.is_admin,
                csrf,
                &form,
                msg,
            );
            return Ok(render(&t)?.into_response());
        }
    };
    state
        .store
        .update_project(id, &name, &description, scan, nag)
        .await?;
    Ok(Redirect::to(&format!("/projects/{id}")).into_response())
}

async fn project_delete(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Query(confirm): Query<ConfirmQuery>,
) -> Result<Response, AppError> {
    owned_project(&state.store, id, user.id).await?;
    if !confirm.is_confirmed() {
        return confirmation_page(
            &state,
            &jar,
            user.is_admin,
            &CONFIRM_DELETE_PROJECT,
            &format!("/projects/{id}/delete"),
            format!("/projects/{id}"),
        );
    }
    state.store.delete_project(id).await?;
    Ok(Redirect::to("/").into_response())
}

// --- check templates ---
#[derive(Deserialize)]
pub(crate) struct CheckForm {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) schedule_kind: String,
    pub(crate) period_secs: String,
    pub(crate) cron_expr: String,
    pub(crate) grace_secs: String,
    pub(crate) timezone: String,
    pub(crate) scan_interval_secs: String,
    pub(crate) max_runtime_secs: String,
    pub(crate) nag_interval_secs: String,
}

struct PingRow {
    time: String,             // UTC fallback shown when JS is off
    iso: String,              // RFC3339 UTC; localized to the viewer's zone client-side
    pill_class: &'static str, // "ok"|"fail"|"start"|"log"
    kind_label: &'static str, // "success"|"fail"|"start"|"log"
    exit: String,
    duration: String,
    source: String,
    body: String,
}

/// `Exitcode` never reaches storage (the `/ping/{uuid}/{code}` handler maps it first).
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
    created_at: String,             // UTC fallback shown when JS is off
    iso: String,                    // RFC3339 UTC; localized client-side
    event: &'static str,            // "down"|"up"|"reminder"
    event_pill_class: &'static str, // mirrors the ping-kind pills
    status: &'static str,
    channel: String,
    error: String,
}

/// Reuses the ping-kind palette; `Test` is never stored but kept for exhaustiveness.
fn notif_event_pill_class(e: crate::notify::EventKind) -> &'static str {
    use crate::notify::EventKind;
    match e {
        EventKind::Up => "ok",
        EventKind::Down => "fail",
        EventKind::Reminder => "start",
        EventKind::Test => "log",
    }
}

#[derive(Template)]
#[template(path = "check_form.html")]
struct CheckFormTemplate {
    show_nav: bool,
    csrf: String,
    is_admin: bool,
    heading: String,
    action: String,
    error: Option<String>,
    name: String,
    description: String,
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
    csrf: String,
    is_admin: bool,
    admin: bool,
    check: Check,
    description_html: String,
    project_name: String,
    status: &'static str,
    since: String,
    next_due: crate::view::NextDue,
    schedule: String,
    ping_url: String,
    /// Withheld pending an audited reveal (see [`CheckPageViewer`]).
    ping_url_hidden: bool,
    bars: Vec<crate::view::Bar>,
    channel_boxes: Vec<ChannelBox>,
    /// From [`CheckPingsTemplate`], so page load and JS refresh share markup; `|safe`.
    pings_partial: String,
    /// Likewise, from [`CheckNotifsTemplate`].
    notifs_partial: String,
    flash: Option<String>,
}

/// Served standalone by `GET /checks/{id}/pings` (JS swaps it into
/// `#pings-section`) and inlined into the check page. `base` is `""` or `/admin`.
#[derive(Template)]
#[template(path = "check_pings.html")]
struct CheckPingsTemplate {
    base: String,
    check_id: i64,
    rows: Vec<PingRow>,
    empty: bool,
    /// `""` = all, canonicalized from the query.
    f_kind: String,
    /// `Z`-form RFC3339 UTC (`""` = unset); localized client-side.
    f_from: String,
    f_to: String,
    /// Controls the "Clear" affordance.
    filtered: bool,
    /// The notifications filter, re-sent so a scriptless submit here keeps it.
    carry: Vec<HiddenField>,
    /// Clear link; keeps the other section's filter.
    clear: String,
    newer: Option<String>,
    older: Option<String>,
}

/// Served by `GET /checks/{id}/notifications`; otherwise as
/// [`CheckPingsTemplate`].
#[derive(Template)]
#[template(path = "check_notifs.html")]
struct CheckNotifsTemplate {
    base: String,
    check_id: i64,
    rows: Vec<NotificationRow>,
    empty: bool,
    /// `""` = all: up|down|reminder.
    f_event: String,
    /// `""` = all: ok|error.
    f_status: String,
    f_from: String,
    f_to: String,
    filtered: bool,
    /// The pings section's filter — see [`CheckPingsTemplate::carry`].
    carry: Vec<HiddenField>,
    clear: String,
    newer: Option<String>,
    older: Option<String>,
}

/// Shared by the check page and both fragments: `p*` = pings, `n*` = notifications;
/// `pb`/`pa`, `nb`/`na` are keyset cursors (older/newer). A missing param means the
/// unfiltered "Latest" view; bad filter text is ignored, but a non-numeric cursor is a 400.
#[derive(Deserialize, Default)]
struct CheckPageQuery {
    #[serde(default)]
    pb: Option<i64>,
    #[serde(default)]
    pa: Option<i64>,
    #[serde(default)]
    nb: Option<i64>,
    #[serde(default)]
    na: Option<i64>,
    #[serde(default)]
    pk: Option<String>,
    #[serde(default)]
    pfrom: Option<String>,
    #[serde(default)]
    pto: Option<String>,
    #[serde(default)]
    ne: Option<String>,
    #[serde(default)]
    ns: Option<String>,
    #[serde(default)]
    nfrom: Option<String>,
    #[serde(default)]
    nto: Option<String>,
}

/// Blank or garbage → empty. A `Vec` for the store filters; the UI offers one choice.
fn parse_filter_enum<T: FromStr>(v: Option<&str>) -> Vec<T> {
    v.map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<T>().ok())
        .into_iter()
        .collect()
}

/// Free text (audit actor/action); an unknown value just pages empty.
fn parse_filter_text(v: Option<&str>) -> Option<String> {
    v.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// RFC3339 (from JS) or bare `YYYY-MM-DDTHH:MM[:SS]` (JS off) read as UTC; else `None`.
fn parse_date_bound(v: Option<&str>) -> Option<DateTime<Utc>> {
    let s = v.map(str::trim).filter(|s| !s.is_empty())?;
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return Some(ndt.and_utc());
        }
    }
    None
}

/// Age of an RFC3339 loop stamp; unparseable renders no age rather than a wrong one.
fn relative_setting(raw: Option<&str>, now: DateTime<Utc>) -> Option<String> {
    let at = DateTime::parse_from_rfc3339(raw?).ok()?.with_timezone(&Utc);
    Some(crate::view::fmt_relative(at, now))
}

/// No-JS UTC text for a loop stamp; on `None` the template shows the raw string.
fn absolute_setting(raw: Option<&str>) -> Option<String> {
    let at = DateTime::parse_from_rfc3339(raw?).ok()?.with_timezone(&Utc);
    Some(crate::view::fmt_utc(&at))
}

/// A GET filter submit replaces the whole query string, so the *other* section's
/// filter rides along as hidden fields.
struct HiddenField {
    name: &'static str,
    value: String,
}

/// Skips empties and `mine`, the keys this form's visible controls already send.
fn carry_fields(all: &[(&'static str, String)], mine: &[&str]) -> Vec<HiddenField> {
    all.iter()
        .filter(|(k, v)| !v.is_empty() && !mine.contains(k))
        .map(|(k, v)| HiddenField {
            name: k,
            value: v.clone(),
        })
        .collect()
}

/// [`carry_fields`]'s rule as an href.
fn clear_href(path: &str, carry: &[HiddenField]) -> String {
    use std::fmt::Write as _;
    let mut href = path.to_string();
    for (i, f) in carry.iter().enumerate() {
        let sep = if i == 0 { '?' } else { '&' };
        let _ = write!(href, "{sep}{}={}", f.name, f.value);
    }
    href
}

/// `Z`-form because `+00:00` would need percent-encoding in a pager href.
fn date_bound_token(dt: Option<DateTime<Utc>>) -> String {
    dt.map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// Values are ids, enum tokens or `Z`-form datetimes — query-safe, so no encoding.
fn history_href(path: &str, cursor: (&str, i64), carry: &[(&str, &str)]) -> String {
    use std::fmt::Write as _;
    let mut href = format!("{path}?{}={}", cursor.0, cursor.1);
    for (k, v) in carry {
        if !v.is_empty() {
            let _ = write!(href, "&{k}={v}");
        }
    }
    href
}

/// E.g. "down · 2h 14m ago · not acknowledged", or "updated 3m ago".
fn status_since_label(check: &Check, now: chrono::DateTime<Utc>) -> String {
    if crate::view::display_status(check, now) == crate::view::DisplayStatus::Down {
        let ack = if check.acknowledged {
            "acknowledged"
        } else {
            "not acknowledged"
        };
        // A check can go New -> Down without ever having received a ping.
        let relative = check.last_ping_at.map_or_else(
            || "no pings yet".into(),
            |t| crate::view::fmt_relative(t, now),
        );
        format!("down · {relative} · {ack}")
    } else {
        let relative = check
            .last_ping_at
            .map_or_else(|| "never".into(), |t| crate::view::fmt_relative(t, now));
        format!("updated {relative}")
    }
}

/// Owned via its project; another user's is `NotFound`, as in [`owned_project`].
async fn owned_check(store: &Store, id: i64, user_id: i64) -> Result<Check, AppError> {
    let check = store.find_check(id).await?.ok_or(AppError::NotFound)?;
    owned_project(store, check.project_id, user_id).await?;
    Ok(check)
}

fn empty_check_form(
    heading: &str,
    action: String,
    is_admin: bool,
    csrf: String,
) -> CheckFormTemplate {
    CheckFormTemplate {
        show_nav: true,
        csrf,
        is_admin,
        heading: heading.into(),
        action,
        error: None,
        name: String::new(),
        description: String::new(),
        schedule_kind: "period".into(),
        period_secs: String::new(),
        cron_expr: String::new(),
        grace_secs: "5m".into(),
        timezone: "UTC".into(),
        scan_interval_secs: String::new(),
        max_runtime_secs: String::new(),
        nag_interval_secs: String::new(),
    }
}

#[derive(Debug)]
pub(crate) struct ValidatedCheck {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) kind: ScheduleKind,
    pub(crate) period_secs: Option<i64>,
    pub(crate) grace: i64,
    pub(crate) cron_expr: Option<String>,
    pub(crate) timezone: String,
    pub(crate) scan_interval_secs: Option<i64>,
    pub(crate) max_runtime_secs: Option<i64>,
    pub(crate) nag_interval_secs: Option<i64>,
}

/// Blank means UTC (column default, API's `default_timezone`). A typo is rejected and
/// echoed back: `due_time` would silently fall back to UTC.
fn validate_timezone(raw: &str) -> Result<String, String> {
    let tz = raw.trim();
    if tz.is_empty() {
        return Ok("UTC".to_string());
    }
    match tz.parse::<chrono_tz::Tz>() {
        Ok(parsed) => Ok(parsed.name().to_string()),
        Err(_) => Err(format!(
            "unknown timezone \"{tz}\" — use an IANA name such as UTC or Asia/Taipei"
        )),
    }
}

/// A non-blank override that isn't a positive duration is rejected, not dropped.
pub(crate) fn validate_check(form: &CheckForm) -> Result<ValidatedCheck, String> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let description = validate_description(&form.description)?;
    let grace = crate::duration::parse_duration(&form.grace_secs)
        .ok_or("grace_secs must be a duration (e.g. 30, 5m, 1h30m)")?;
    if grace < 0 {
        return Err("grace_secs must be >= 0".into());
    }
    let kind = ScheduleKind::from_str(&form.schedule_kind)
        .map_err(|_e| "invalid schedule kind".to_string())?;
    let (period_secs, cron_expr) = match kind {
        ScheduleKind::Period => {
            if form.period_secs.trim().is_empty() {
                return Err("period_secs required for period mode".into());
            }
            let secs = crate::duration::parse_duration(&form.period_secs)
                .ok_or("period_secs must be a duration (e.g. 30, 5m, 1h30m)")?;
            if secs <= 0 {
                return Err("period_secs must be > 0".into());
            }
            (Some(secs), None)
        }
        ScheduleKind::Cron => {
            let expr = form.cron_expr.trim();
            if expr.is_empty() {
                return Err("cron_expr required for cron mode".into());
            }
            Schedule::from_str(expr).map_err(|e| format!("invalid cron expression: {e}"))?;
            (None, Some(expr.to_string()))
        }
    };
    let timezone = validate_timezone(&form.timezone)?;
    let scan_interval_secs =
        parse_opt_positive_duration(&form.scan_interval_secs, "scan interval")?;
    let max_runtime_secs = parse_opt_positive_duration(&form.max_runtime_secs, "max runtime")?;
    let nag_interval_secs = parse_opt_positive_duration(&form.nag_interval_secs, "nag interval")?;
    Ok(ValidatedCheck {
        name: name.to_string(),
        description,
        kind,
        period_secs,
        grace,
        cron_expr,
        timezone,
        scan_interval_secs,
        max_runtime_secs,
        nag_interval_secs,
    })
}

async fn check_new(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    let csrf = current_csrf(&state, &jar);
    let form = empty_check_form(
        "New check",
        format!("/projects/{pid}/checks"),
        user.is_admin,
        csrf,
    );
    Ok(render(&form)?.into_response())
}

async fn check_create_core(
    state: &AppState,
    pid: i64,
    form: CheckForm,
    admin: bool,
    is_admin: bool,
    csrf: String,
) -> Result<Response, AppError> {
    let base = admin_prefix(admin);
    let v = match validate_check(&form) {
        Ok(v) => v,
        Err(msg) => {
            let mut t = empty_check_form(
                "New check",
                format!("{base}/projects/{pid}/checks"),
                is_admin,
                csrf,
            );
            t.error = Some(msg);
            t.name = form.name;
            t.description = form.description;
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
        .create_check(&crate::store::NewCheck {
            project_id: pid,
            name: &v.name,
            description: &v.description,
            ping_uuid: &uuid,
            kind: v.kind,
            period_secs: v.period_secs,
            grace_secs: v.grace,
            cron_expr: v.cron_expr.as_deref(),
            timezone: &v.timezone,
            scan_interval_secs: v.scan_interval_secs,
            max_runtime_secs: v.max_runtime_secs,
            nag_interval_secs: v.nag_interval_secs,
        })
        .await?;
    state.store.bind_all_project_channels(id, pid).await?;
    Ok(Redirect::to(&format!("{base}/checks/{id}")).into_response())
}

async fn check_create(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
    Form(form): Form<CheckForm>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    let csrf = current_csrf(&state, &jar);
    check_create_core(&state, pid, form, false, user.is_admin, csrf).await
}

const FLASH_COOKIE_BASE: &str = "pingward_flash";

/// With `PINGWARD_COOKIE_SECURE` (`Secure`, `Path=/`, no `Domain`, as `__Host-` requires).
const FLASH_COOKIE_HOST_PREFIXED: &str = "__Host-pingward_flash";

/// The flash carries no authority; the `__Host-` prefix (HTTPS) and
/// [`secret::sign_flash`] (plain HTTP) stop a sibling subdomain planting a message.
fn flash_cookie_name(config: &crate::config::Config) -> &'static str {
    if config.cookie_secure {
        FLASH_COOKIE_HOST_PREFIXED
    } else {
        FLASH_COOKIE_BASE
    }
}

/// Consumes the flash only if it was set for `surface` (else it's left for its page);
/// only known keys map to text, so a cookie value never renders verbatim.
fn take_flash(
    config: &crate::config::Config,
    jar: CookieJar,
    surface: &str,
) -> (CookieJar, Option<String>) {
    let Some(value) = flash_payload(config, &jar) else {
        return (jar, None);
    };
    if value != surface {
        return (jar, None);
    }
    let message = match surface {
        "channels" => "Notify channels saved.",
        "settings" => "Settings saved.",
        "users_blocked" => {
            "That action was refused: you cannot remove your own access, and the last enabled admin cannot be removed."
        }
        "password_changed" => {
            "Password changed. Any other signed-in sessions were signed out; API keys are unaffected."
        }
        "admin_locked" => {
            "That action wasn't performed — it grants access, so it needs confirming first."
        }
        // Names no action: a refused one is dropped, not replayed, and this page is
        // also reached by plain navigation.
        "admin_unlocked" => {
            "Confirmed. If an action was refused a moment ago it was not performed — do it again now."
        }
        "forward_auth_logout" => {
            "Signed out locally, but you're authenticated through your reverse proxy — this app can't end that session. To sign out completely, log out at your proxy or SSO provider."
        }
        _ => return (jar, None),
    };
    (
        jar.remove(flash_removal_cookie(config)),
        Some(message.to_string()),
    )
}

/// `None` if absent, malformed, or not signed with this process's secret.
fn flash_payload(config: &crate::config::Config, jar: &CookieJar) -> Option<String> {
    let cookie = jar.get(flash_cookie_name(config))?;
    secret::verify_flash(&config.secret, cookie.value())
}

/// Stores `<payload>.<hmac>`; see [`flash_cookie_name`].
fn flash_cookie_value(config: &crate::config::Config, value: String) -> Cookie<'static> {
    flash_cookie_raw(config, secret::sign_flash(&config.secret, &value))
}

/// Shared by setter and remover so attributes can't drift (see [`session_cookie`]).
fn flash_cookie_raw(config: &crate::config::Config, raw: String) -> Cookie<'static> {
    Cookie::build((flash_cookie_name(config), raw))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .secure(config.cookie_secure)
        .build()
}

/// `surface` is a fixed key, never user input.
fn flash_cookie(config: &crate::config::Config, surface: &'static str) -> Cookie<'static> {
    flash_cookie_value(config, surface.to_string())
}

/// Unsigned: removal works by attributes, and [`flash_payload`] rejects it anyway.
fn flash_removal_cookie(config: &crate::config::Config) -> Cookie<'static> {
    flash_cookie_raw(config, String::new())
}

/// The refusal for the self-guard and last-enabled-admin-guard branches.
fn users_blocked(config: &crate::config::Config, jar: CookieJar) -> Response {
    let jar = jar.add(flash_cookie(config, "users_blocked"));
    (jar, Redirect::to("/admin")).into_response()
}

/// `password_reset_keys:<revoked>:<keys>`, separate from [`take_flash`]'s fixed keys.
const PASSWORD_RESET_KEYS_PREFIX: &str = "password_reset_keys:";

/// Warns that a reset revoked sessions but not API keys. Counts are server-computed.
fn password_reset_keys_flash(
    config: &crate::config::Config,
    jar: CookieJar,
    revoked: u64,
    keys: u64,
) -> Response {
    let value = format!("{PASSWORD_RESET_KEYS_PREFIX}{revoked}:{keys}");
    let jar = jar.add(flash_cookie_value(config, value));
    (jar, Redirect::to("/admin")).into_response()
}

/// Like [`take_flash`], but decodes the two counts.
fn take_password_reset_keys_flash(
    config: &crate::config::Config,
    jar: CookieJar,
) -> (CookieJar, Option<String>) {
    let Some(value) = flash_payload(config, &jar) else {
        return (jar, None);
    };
    let Some(rest) = value.strip_prefix(PASSWORD_RESET_KEYS_PREFIX) else {
        return (jar, None);
    };
    let Some((revoked, keys)) = rest.split_once(':') else {
        return (jar, None);
    };
    let (Ok(revoked), Ok(keys)) = (revoked.parse::<u64>(), keys.parse::<u64>()) else {
        return (jar, None);
    };
    let sessions_word = if revoked == 1 { "session" } else { "sessions" };
    let (keys_word, keys_verb) = if keys == 1 {
        ("key", "continues")
    } else {
        ("keys", "continue")
    };
    // Fits self- and other-targeted resets: disabling is offered only for *another*
    // account, since `users_set_disabled` refuses self-targeting.
    let message = format!(
        "Password reset revoked {revoked} {sessions_word}, but this account still has {keys} API {keys_word} that {keys_verb} to work. An API key can only be revoked from its owner's own /account page — to cut off another user's access immediately, disable their account instead."
    );
    (jar.remove(flash_removal_cookie(config)), Some(message))
}

async fn check_show(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Query(page): Query<CheckPageQuery>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    let csrf = current_csrf(&state, &jar);
    let (jar, flash) = take_flash(&state.config, jar, "channels");
    let resp = render_check_page(
        &state,
        check,
        user.is_admin,
        csrf,
        flash,
        page,
        CheckPageViewer::Owner,
    )
    .await?;
    Ok((jar, resp).into_response())
}

/// Route prefix plus whether the ping URL (a bearer credential) may be printed, in one
/// value so they can't contradict. An admin must reveal another user's URL (audited as
/// `admin.ping_url_reveal`); `viewer_id == owner_id` exempts their own checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckPageViewer {
    /// An owner-route render: the URL is the page's whole point.
    Owner,
    Admin {
        viewer_id: i64,
        ping_url_revealed: bool,
    },
}

impl CheckPageViewer {
    fn is_admin_route(self) -> bool {
        matches!(self, Self::Admin { .. })
    }

    fn shows_url(self, owner_id: i64) -> bool {
        match self {
            Self::Owner => true,
            Self::Admin {
                viewer_id,
                ping_url_revealed,
            } => ping_url_revealed || viewer_id == owner_id,
        }
    }
}

/// The route prefix comes from `viewer` (see [`admin_prefix`] for `is_admin`).
async fn render_check_page(
    state: &AppState,
    check: Check,
    is_admin: bool,
    csrf: String,
    flash: Option<String>,
    page: CheckPageQuery,
    viewer: CheckPageViewer,
) -> Result<Response, AppError> {
    let id = check.id;
    let admin = viewer.is_admin_route();
    let base = admin_prefix(admin);
    let project = state
        .store
        .find_project(check.project_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let now = Utc::now();
    let show_ping_url = viewer.shows_url(project.user_id);
    let ping_url = if show_ping_url {
        format!(
            "{}/ping/{}",
            state.config.base_url.trim_end_matches('/'),
            check.ping_uuid
        )
    } else {
        String::new()
    };
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
    // Always the latest window, never the table's page; summaries, so no bodies load.
    let recent = state
        .store
        .list_recent_ping_summaries(id, HEARTBEAT_WINDOW)
        .await?;
    let bars = crate::view::heartbeat(
        &recent,
        check.max_runtime_secs,
        check.status == CheckStatus::Paused,
        HEARTBEAT_BARS,
    );

    let status = crate::view::display_status(&check, now).as_str();
    let since = status_since_label(&check, now);
    let next_due = crate::view::next_due(&check, now);
    let schedule = schedule_label(&check);
    let description_html = crate::markdown::render(&check.description);

    // The same fragments the partial endpoints serve: one source of markup.
    let pings_partial =
        render(&build_pings_partial(state, id, base, &page, Some(&recent)).await?)?.0;
    let notifs_partial =
        render(&build_notifs_partial(state, id, base, &page, &channel_names).await?)?.0;

    Ok(render(&CheckTemplate {
        show_nav: true,
        csrf,
        is_admin,
        admin,
        check,
        description_html,
        project_name: project.name,
        status,
        since,
        next_due,
        schedule,
        ping_url,
        ping_url_hidden: !show_ping_url,
        bars,
        channel_boxes,
        pings_partial,
        notifs_partial,
        flash,
    })?
    .into_response())
}

/// `recent` is the page's already-fetched heartbeat window; `None` re-fetches if needed.
async fn build_pings_partial(
    state: &AppState,
    check_id: i64,
    base: &str,
    page: &CheckPageQuery,
    recent: Option<&[crate::models::PingSummary]>,
) -> Result<CheckPingsTemplate, AppError> {
    let filter = PingFilter {
        kinds: parse_filter_enum(page.pk.as_deref()),
        from: parse_date_bound(page.pfrom.as_deref()),
        to: parse_date_bound(page.pto.as_deref()),
    };
    let cursor = match (page.pb, page.pa) {
        (Some(b), _) => PageCursor::Before(b),
        (None, Some(a)) => PageCursor::After(a),
        (None, None) => PageCursor::Latest,
    };
    let ping_page = state
        .store
        .list_pings_page(check_id, cursor, 20, &filter)
        .await?;

    // Default view pairs over `HEARTBEAT_WINDOW`, so an off-page start still yields a
    // duration; elsewhere pairing is best-effort (a start may be filtered out).
    let durations = if matches!(cursor, PageCursor::Latest) && filter.is_empty() {
        if let Some(r) = recent {
            crate::view::run_durations(r)
        } else {
            let r = state
                .store
                .list_recent_ping_summaries(check_id, HEARTBEAT_WINDOW)
                .await?;
            crate::view::run_durations(&r)
        }
    } else {
        // Project to summaries so no captured body is cloned.
        let summaries: Vec<crate::models::PingSummary> =
            ping_page.items.iter().map(Into::into).collect();
        crate::view::run_durations(&summaries)
    };

    let rows: Vec<PingRow> = ping_page
        .items
        .iter()
        .map(|p| PingRow {
            time: p.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            iso: p.created_at.to_rfc3339(),
            pill_class: ping_pill_class(p.kind),
            kind_label: p.kind.as_str(),
            exit: p
                .exit_code
                .map_or_else(|| "—".into(), |c| format!("exit {c}")),
            duration: durations
                .get(&p.id)
                .map_or_else(|| "—".into(), |d| crate::view::fmt_secs(*d)),
            source: p.source_ip.clone().unwrap_or_else(|| "—".into()),
            body: p.body.clone(),
        })
        .collect();

    let f_kind = filter
        .kinds
        .first()
        .map(|k| k.as_str().to_string())
        .unwrap_or_default();
    let f_from = date_bound_token(filter.from);
    let f_to = date_bound_token(filter.to);
    let carry = [
        ("pk", f_kind.as_str()),
        ("pfrom", f_from.as_str()),
        ("pto", f_to.as_str()),
    ];
    let endpoint = format!("{base}/checks/{check_id}/pings");
    let older = ping_page
        .has_older
        .then(|| ping_page.items.last())
        .flatten()
        .map(|p| history_href(&endpoint, ("pb", p.id), &carry));
    let newer = ping_page
        .has_newer
        .then(|| ping_page.items.first())
        .flatten()
        .map(|p| history_href(&endpoint, ("pa", p.id), &carry));

    // The notifications half of the query, which this form must preserve.
    let notif_filter = NotifFilter {
        events: parse_filter_enum(page.ne.as_deref()),
        statuses: parse_filter_enum(page.ns.as_deref()),
        from: parse_date_bound(page.nfrom.as_deref()),
        to: parse_date_bound(page.nto.as_deref()),
    };
    let tokens = check_page_filter_tokens(
        &f_kind,
        &f_from,
        &f_to,
        &notif_filter
            .events
            .first()
            .map(|e| e.as_str().to_string())
            .unwrap_or_default(),
        &notif_filter
            .statuses
            .first()
            .map(|s| s.as_str().to_string())
            .unwrap_or_default(),
        &date_bound_token(notif_filter.from),
        &date_bound_token(notif_filter.to),
    );
    let hidden = carry_fields(&tokens, &PINGS_FILTER_KEYS);
    // Clear targets the fragment endpoint: `wireSection` expects a partial, and
    // without JS it redirects to the page (`fragment_page_redirect`).
    let clear = clear_href(&endpoint, &hidden);

    Ok(CheckPingsTemplate {
        base: base.to_string(),
        check_id,
        empty: rows.is_empty(),
        rows,
        f_kind,
        f_from,
        f_to,
        filtered: !filter.is_empty(),
        carry: hidden,
        clear,
        newer,
        older,
    })
}

/// [`build_pings_partial`]'s twin over the `n*` params.
async fn build_notifs_partial(
    state: &AppState,
    check_id: i64,
    base: &str,
    page: &CheckPageQuery,
    channel_names: &std::collections::HashMap<i64, String>,
) -> Result<CheckNotifsTemplate, AppError> {
    let filter = NotifFilter {
        events: parse_filter_enum(page.ne.as_deref()),
        statuses: parse_filter_enum(page.ns.as_deref()),
        from: parse_date_bound(page.nfrom.as_deref()),
        to: parse_date_bound(page.nto.as_deref()),
    };
    let cursor = match (page.nb, page.na) {
        (Some(b), _) => PageCursor::Before(b),
        (None, Some(a)) => PageCursor::After(a),
        (None, None) => PageCursor::Latest,
    };
    let notif_page = state
        .store
        .list_notifications_page(check_id, cursor, 20, &filter)
        .await?;

    let rows: Vec<NotificationRow> = notif_page
        .items
        .iter()
        .map(|n| NotificationRow {
            created_at: n.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            iso: n.created_at.to_rfc3339(),
            event: n.event.as_str(),
            event_pill_class: notif_event_pill_class(n.event),
            status: n.status.as_str(),
            channel: channel_names
                .get(&n.channel_id)
                .cloned()
                .unwrap_or_else(|| "(deleted)".into()),
            error: n.error.clone().unwrap_or_default(),
        })
        .collect();

    let f_event = filter
        .events
        .first()
        .map(|e| e.as_str().to_string())
        .unwrap_or_default();
    let f_status = filter
        .statuses
        .first()
        .map(|s| s.as_str().to_string())
        .unwrap_or_default();
    let f_from = date_bound_token(filter.from);
    let f_to = date_bound_token(filter.to);
    let carry = [
        ("ne", f_event.as_str()),
        ("ns", f_status.as_str()),
        ("nfrom", f_from.as_str()),
        ("nto", f_to.as_str()),
    ];
    let endpoint = format!("{base}/checks/{check_id}/notifications");
    let older = notif_page
        .has_older
        .then(|| notif_page.items.last())
        .flatten()
        .map(|n| history_href(&endpoint, ("nb", n.id), &carry));
    let newer = notif_page
        .has_newer
        .then(|| notif_page.items.first())
        .flatten()
        .map(|n| history_href(&endpoint, ("na", n.id), &carry));

    // The pings half of the query, which this form must preserve.
    let ping_filter = PingFilter {
        kinds: parse_filter_enum(page.pk.as_deref()),
        from: parse_date_bound(page.pfrom.as_deref()),
        to: parse_date_bound(page.pto.as_deref()),
    };
    let tokens = check_page_filter_tokens(
        &ping_filter
            .kinds
            .first()
            .map(|k| k.as_str().to_string())
            .unwrap_or_default(),
        &date_bound_token(ping_filter.from),
        &date_bound_token(ping_filter.to),
        &f_event,
        &f_status,
        &f_from,
        &f_to,
    );
    let hidden = carry_fields(&tokens, &NOTIFS_FILTER_KEYS);
    // Fragment endpoint, for the reason spelled out in `build_pings_partial`.
    let clear = clear_href(&endpoint, &hidden);

    Ok(CheckNotifsTemplate {
        base: base.to_string(),
        check_id,
        empty: rows.is_empty(),
        rows,
        f_event,
        f_status,
        f_from,
        f_to,
        filtered: !filter.is_empty(),
        carry: hidden,
        clear,
        newer,
        older,
    })
}

/// For the standalone notifications partial; the full page reuses its own map.
async fn channel_name_map(
    state: &AppState,
    project_id: i64,
) -> Result<std::collections::HashMap<i64, String>, AppError> {
    Ok(state
        .store
        .list_channels_for_project(project_id)
        .await?
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect())
}

/// A data-less "changed" event per matching broadcast; the browser re-fetches.
fn sse_for_check(
    events: &broadcast::Sender<i64>,
    check_id: i64,
) -> Sse<impl Stream<Item = Result<Event, Infallible>> + use<>> {
    let stream = BroadcastStream::new(events.subscribe()).filter_map(move |res| match res {
        Ok(id) if id == check_id => Some(Ok(Event::default().data("changed"))),
        Ok(_) => None,
        // Lagged: coalesce into one refresh, or the page would stay stale.
        Err(_) => Some(Ok(Event::default().data("changed"))),
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// `X-Requested-With: fetch` (set by `app.js`); absent means a real navigation.
fn wants_fragment(headers: &HeaderMap) -> bool {
    headers
        .get("x-requested-with")
        .is_some_and(|v| v.as_bytes() == b"fetch")
}
/// JS-off pager/Clear links hit the fragment endpoint; redirect to the embedding page
/// (same query struct) rather than serve a bare partial. Safe to vary: [`no_store`].
fn fragment_page_redirect(path: &str, anchor: &str, uri: &axum::http::Uri) -> Response {
    let query = uri.query().unwrap_or_default();
    let sep = if query.is_empty() { "" } else { "?" };
    Redirect::to(&format!("{path}{sep}{query}#{anchor}")).into_response()
}

async fn check_pings(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Query(page): Query<CheckPageQuery>,
) -> Result<Response, AppError> {
    // Ownership first, or the redirect would disclose that the check exists.
    let check = owned_check(&state.store, id, user.id).await?;
    if !wants_fragment(&headers) {
        return Ok(fragment_page_redirect(
            &format!("/checks/{}", check.id),
            "pings-section",
            &uri,
        ));
    }
    Ok(render(&build_pings_partial(&state, check.id, "", &page, None).await?)?.into_response())
}

/// SSE: a ping arrived, or the scan loop transitioned this check.
async fn check_events(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    Ok(sse_for_check(&state.events, check.id))
}

async fn check_notifications(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Query(page): Query<CheckPageQuery>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    if !wants_fragment(&headers) {
        return Ok(fragment_page_redirect(
            &format!("/checks/{}", check.id),
            "notifs-section",
            &uri,
        ));
    }
    let names = channel_name_map(&state, check.project_id).await?;
    Ok(render(&build_notifs_partial(&state, check.id, "", &page, &names).await?)?.into_response())
}

async fn admin_check_pings(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    headers: HeaderMap,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Query(page): Query<CheckPageQuery>,
) -> Result<Response, AppError> {
    let check = admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    if !wants_fragment(&headers) {
        return Ok(fragment_page_redirect(
            &format!("/admin/checks/{}", check.id),
            "pings-section",
            &uri,
        ));
    }
    Ok(
        render(&build_pings_partial(&state, check.id, "/admin", &page, None).await?)?
            .into_response(),
    )
}

async fn admin_check_events(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let check = admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    Ok(sse_for_check(&state.events, check.id))
}

async fn admin_check_notifications(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    headers: HeaderMap,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Query(page): Query<CheckPageQuery>,
) -> Result<Response, AppError> {
    let check = admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    if !wants_fragment(&headers) {
        return Ok(fragment_page_redirect(
            &format!("/admin/checks/{}", check.id),
            "notifs-section",
            &uri,
        ));
    }
    let names = channel_name_map(&state, check.project_id).await?;
    Ok(
        render(&build_notifs_partial(&state, check.id, "/admin", &page, &names).await?)?
            .into_response(),
    )
}

fn check_edit_form(check: Check, admin: bool, is_admin: bool, csrf: String) -> CheckFormTemplate {
    let base = admin_prefix(admin);
    CheckFormTemplate {
        show_nav: true,
        csrf,
        is_admin,
        heading: "Edit check".into(),
        action: format!("{base}/checks/{}", check.id),
        error: None,
        name: check.name,
        description: check.description,
        schedule_kind: check.schedule_kind.as_str().into(),
        period_secs: check
            .period_secs
            .map(crate::duration::fmt_duration)
            .unwrap_or_default(),
        cron_expr: check.cron_expr.unwrap_or_default(),
        grace_secs: crate::duration::fmt_duration(check.grace_secs),
        timezone: check.timezone,
        scan_interval_secs: check
            .scan_interval_secs
            .map(crate::duration::fmt_duration)
            .unwrap_or_default(),
        max_runtime_secs: check
            .max_runtime_secs
            .map(crate::duration::fmt_duration)
            .unwrap_or_default(),
        nag_interval_secs: check
            .nag_interval_secs
            .map(crate::duration::fmt_duration)
            .unwrap_or_default(),
    }
}

async fn check_edit(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    let csrf = current_csrf(&state, &jar);
    Ok(render(&check_edit_form(check, false, user.is_admin, csrf))?.into_response())
}

async fn check_update_core(
    state: &AppState,
    id: i64,
    form: CheckForm,
    admin: bool,
    is_admin: bool,
    csrf: String,
) -> Result<Response, AppError> {
    let base = admin_prefix(admin);
    let v = match validate_check(&form) {
        Ok(v) => v,
        Err(msg) => {
            let t = CheckFormTemplate {
                show_nav: true,
                csrf,
                is_admin,
                heading: "Edit check".into(),
                action: format!("{base}/checks/{id}"),
                error: Some(msg),
                name: form.name,
                description: form.description,
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
            &crate::store::UpdateCheck {
                name: &v.name,
                description: &v.description,
                kind: v.kind,
                period_secs: v.period_secs,
                grace_secs: v.grace,
                cron_expr: v.cron_expr.as_deref(),
                timezone: &v.timezone,
                scan_interval_secs: v.scan_interval_secs,
                max_runtime_secs: v.max_runtime_secs,
                nag_interval_secs: v.nag_interval_secs,
            },
        )
        .await?;
    Ok(Redirect::to(&format!("{base}/checks/{id}")).into_response())
}

async fn check_update(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<CheckForm>,
) -> Result<Response, AppError> {
    owned_check(&state.store, id, user.id).await?;
    let csrf = current_csrf(&state, &jar);
    check_update_core(&state, id, form, false, user.is_admin, csrf).await
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
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Query(confirm): Query<ConfirmQuery>,
) -> Result<Response, AppError> {
    owned_check(&state.store, id, user.id).await?;
    if !confirm.is_confirmed() {
        return confirmation_page(
            &state,
            &jar,
            user.is_admin,
            &CONFIRM_REGENERATE_URL,
            &format!("/checks/{id}/regenerate"),
            format!("/checks/{id}"),
        );
    }
    state
        .store
        .regenerate_uuid(id, &uuid::Uuid::new_v4().to_string())
        .await?;
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}

async fn check_delete(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Query(confirm): Query<ConfirmQuery>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    if !confirm.is_confirmed() {
        return confirmation_page(
            &state,
            &jar,
            user.is_admin,
            &CONFIRM_DELETE_CHECK,
            &format!("/checks/{id}/delete"),
            format!("/checks/{id}"),
        );
    }
    state.store.delete_check(id).await?;
    Ok(Redirect::to(&format!("/projects/{}", check.project_id)).into_response())
}

// --- channel templates ---
#[derive(Template)]
#[template(path = "channel_form.html")]
struct ChannelFormTemplate {
    show_nav: bool,
    csrf: String,
    is_admin: bool,
    admin: bool,
    project_id: i64,
    error: Option<String>,
    smtp_available: bool,
    /// `Some` when editing (heading, action, fixed kind, config block).
    edit: Option<ChannelEditView>,
}

/// The edit template's only view of a stored config, making non-leakage a type property
/// (`ChannelDto` does the same for the API). Webhook/Slack URLs are capability secrets;
/// chat ids, ntfy server/topic and email recipients are safe to pre-fill.
struct ChannelEditView {
    id: i64,
    kind: &'static str,
    name: String,
    // -- pre-filled, non-secret --
    telegram_chat_id: String,
    ntfy_base_url: String,
    ntfy_topic: String,
    email_to: String,
    // -- configured/not-set pills only. Shared `url`/`token` keys can't collide:
    //    only `kind`'s block renders.
    has_webhook_url: bool,
    has_slack_url: bool,
    has_telegram_token: bool,
    has_ntfy_token: bool,
    has_pushover_token: bool,
    has_pushover_user: bool,
}

impl ChannelEditView {
    fn new(ch: &Channel) -> Self {
        let cfg: serde_json::Value =
            serde_json::from_str(&ch.config_json).unwrap_or(serde_json::Value::Null);
        let value = |key: &str| -> String {
            cfg.get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let is_set = |key: &str| -> bool { !value(key).is_empty() };
        Self {
            id: ch.id,
            kind: ch.kind.as_str(),
            name: ch.name.clone(),
            telegram_chat_id: value("chat_id"),
            ntfy_base_url: value("base_url"),
            ntfy_topic: value("topic"),
            email_to: value("to"),
            has_webhook_url: is_set("url"),
            has_slack_url: is_set("url"),
            has_telegram_token: is_set("token"),
            has_ntfy_token: is_set("token"),
            has_pushover_token: is_set("token"),
            has_pushover_user: is_set("user"),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct ChannelForm {
    /// Blank keeps the stored name when editing; required when creating.
    #[serde(default)]
    pub(crate) name: String,
    /// Ignored when editing: immutable, see [`crate::store::Store::update_channel`].
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) webhook_url: String,
    #[serde(default)]
    pub(crate) slack_url: String,
    #[serde(default)]
    pub(crate) telegram_token: String,
    #[serde(default)]
    pub(crate) telegram_chat_id: String,
    #[serde(default)]
    pub(crate) ntfy_base_url: String, // optional, defaults to https://ntfy.sh
    #[serde(default)]
    pub(crate) ntfy_topic: String,
    #[serde(default)]
    pub(crate) ntfy_token: String, // optional
    /// The one exception to blank-means-unchanged, or a stored ntfy token is unremovable.
    #[serde(default)]
    pub(crate) ntfy_token_clear: bool,
    #[serde(default)]
    pub(crate) pushover_token: String, // application token
    #[serde(default)]
    pub(crate) pushover_user: String, // user/group key
    #[serde(default)]
    pub(crate) email_to: String,
}

/// Create-side validation, shared with the API.
pub(crate) fn validate_channel(
    form: &ChannelForm,
) -> Result<(ChannelKind, String, String), String> {
    validate_channel_update(form, None)
}

/// A blank field keeps the stored value (so secrets render as empty inputs); required
/// fields are still enforced. `existing.kind` always wins: the kind is immutable.
pub(crate) fn validate_channel_update(
    form: &ChannelForm,
    existing: Option<&Channel>,
) -> Result<(ChannelKind, String, String), String> {
    // An unparseable stored config counts as empty (create rules), not an error.
    let stored: Option<serde_json::Value> =
        existing.and_then(|c| serde_json::from_str(&c.config_json).ok());
    let stored_str = |key: &str| -> String {
        stored
            .as_ref()
            .and_then(|v| v.get(key))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    // `submitted` blank ⇒ whatever is stored under `key` (empty when creating).
    let merged = |submitted: &str, key: &str| -> String {
        let s = submitted.trim();
        if s.is_empty() {
            stored_str(key)
        } else {
            s.to_string()
        }
    };

    let name = match (form.name.trim(), existing) {
        ("", None) => return Err("a channel name is required".into()),
        ("", Some(c)) => c.name.clone(),
        (n, _) => n.to_string(),
    };
    let kind = if let Some(c) = existing {
        c.kind
    } else {
        let k = form.kind.trim();
        if k.is_empty() {
            return Err("a channel kind is required".into());
        }
        ChannelKind::from_str(k).map_err(|_e| "unknown channel kind".to_string())?
    };
    let config = match kind {
        ChannelKind::Webhook => {
            let url = merged(&form.webhook_url, "url");
            if url.is_empty() {
                return Err("a webhook URL is required".into());
            }
            serde_json::json!({ "url": url }).to_string()
        }
        ChannelKind::Slack => {
            let url = merged(&form.slack_url, "url");
            if url.is_empty() {
                return Err("a Slack incoming-webhook URL is required".into());
            }
            serde_json::json!({ "url": url }).to_string()
        }
        ChannelKind::Telegram => {
            let token = merged(&form.telegram_token, "token");
            let chat_id = merged(&form.telegram_chat_id, "chat_id");
            if token.is_empty() || chat_id.is_empty() {
                return Err("Telegram requires both a bot token and a chat id".into());
            }
            serde_json::json!({ "token": token, "chat_id": chat_id }).to_string()
        }
        ChannelKind::Ntfy => {
            let topic = merged(&form.ntfy_topic, "topic");
            if topic.is_empty() {
                return Err("ntfy requires a topic".into());
            }
            let base_url = {
                let b = merged(&form.ntfy_base_url, "base_url");
                if b.is_empty() {
                    "https://ntfy.sh".to_string()
                } else {
                    b
                }
            };
            let token = if form.ntfy_token_clear {
                String::new()
            } else {
                merged(&form.ntfy_token, "token")
            };
            serde_json::json!({
                "base_url": base_url,
                "topic": topic,
                "token": token,
            })
            .to_string()
        }
        ChannelKind::Pushover => {
            let token = merged(&form.pushover_token, "token");
            let user = merged(&form.pushover_user, "user");
            if token.is_empty() || user.is_empty() {
                return Err("Pushover requires both an application token and a user key".into());
            }
            serde_json::json!({ "token": token, "user": user }).to_string()
        }
        ChannelKind::Email => {
            let to = merged(&form.email_to, "to");
            if to.is_empty() {
                return Err("an email recipient address is required".into());
            }
            serde_json::json!({ "to": to }).to_string()
        }
    };
    Ok((kind, name, config))
}

#[derive(Deserialize)]
struct BindForm {
    #[serde(default)]
    channel_ids: Vec<i64>,
}

/// Shared by create and edit; `edit` is the only difference.
fn channel_form_template(
    state: &AppState,
    project_id: i64,
    admin: bool,
    is_admin: bool,
    csrf: String,
    error: Option<String>,
    edit: Option<ChannelEditView>,
) -> ChannelFormTemplate {
    ChannelFormTemplate {
        show_nav: true,
        csrf,
        is_admin,
        admin,
        project_id,
        error,
        smtp_available: state.config.smtp.is_some(),
        edit,
    }
}

async fn channel_new(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    let csrf = current_csrf(&state, &jar);
    Ok(render(&channel_form_template(
        &state,
        pid,
        false,
        user.is_admin,
        csrf,
        None,
        None,
    ))?
    .into_response())
}

async fn channel_create_core(
    state: &AppState,
    pid: i64,
    form: ChannelForm,
    admin: bool,
    is_admin: bool,
    csrf: String,
) -> Result<Response, AppError> {
    let base = admin_prefix(admin);

    let (kind, name, config) = match validate_channel(&form) {
        Ok(v) => v,
        Err(msg) => {
            return Ok(render(&channel_form_template(
                state,
                pid,
                admin,
                is_admin,
                csrf,
                Some(msg),
                None,
            ))?
            .into_response());
        }
    };

    state
        .store
        .create_channel(pid, kind, &name, &config, Utc::now())
        .await?;
    Ok(Redirect::to(&format!("{base}/projects/{pid}")).into_response())
}

/// Merges over the stored config via [`validate_channel_update`].
async fn channel_update_core(
    state: &AppState,
    channel: &Channel,
    form: ChannelForm,
    admin: bool,
    is_admin: bool,
    csrf: String,
) -> Result<Response, AppError> {
    let base = admin_prefix(admin);
    let pid = channel.project_id;

    let (_kind, name, config) = match validate_channel_update(&form, Some(channel)) {
        Ok(v) => v,
        Err(msg) => {
            // From the *stored* channel, so a typed secret is never echoed back.
            return Ok(render(&channel_form_template(
                state,
                pid,
                admin,
                is_admin,
                csrf,
                Some(msg),
                Some(ChannelEditView::new(channel)),
            ))?
            .into_response());
        }
    };

    state
        .store
        .update_channel(channel.id, &name, &config)
        .await?;
    Ok(Redirect::to(&format!("{base}/projects/{pid}")).into_response())
}

async fn channel_create(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
    Form(form): Form<ChannelForm>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    let csrf = current_csrf(&state, &jar);
    channel_create_core(&state, pid, form, false, user.is_admin, csrf).await
}

/// Owned via its project; another user's is `NotFound`, as in [`owned_project`].
async fn owned_channel(
    store: &crate::store::Store,
    id: i64,
    user_id: i64,
) -> Result<Channel, AppError> {
    let channel = store.find_channel(id).await?.ok_or(AppError::NotFound)?;
    owned_project(store, channel.project_id, user_id).await?;
    Ok(channel)
}

async fn channel_edit(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let channel = owned_channel(&state.store, id, user.id).await?;
    let csrf = current_csrf(&state, &jar);
    Ok(render(&channel_form_template(
        &state,
        channel.project_id,
        false,
        user.is_admin,
        csrf,
        None,
        Some(ChannelEditView::new(&channel)),
    ))?
    .into_response())
}

async fn channel_update(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<ChannelForm>,
) -> Result<Response, AppError> {
    let channel = owned_channel(&state.store, id, user.id).await?;
    let csrf = current_csrf(&state, &jar);
    channel_update_core(&state, &channel, form, false, user.is_admin, csrf).await
}

async fn channel_delete(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let channel = owned_channel(&state.store, id, user.id).await?;
    state.store.delete_channel(id).await?;
    Ok(Redirect::to(&format!("/projects/{}", channel.project_id)).into_response())
}

/// Sends once (no retry) and records nothing in the notification history.
async fn run_channel_test(state: &AppState, channel: &Channel) -> TestResult {
    // No check involved; the project is the only context.
    let project_name = state
        .store
        .find_project(channel.project_id)
        .await
        .ok()
        .flatten()
        .map(|p| p.name);
    let ev = NotificationEvent {
        check_id: 0,
        check_name: channel.name.clone(),
        event: EventKind::Test,
        at: Utc::now(),
        project_id: channel.project_id,
        detail: EventDetail {
            project_name,
            ..Default::default()
        }
        .with_display_timezone(state.store.display_timezone().await.as_deref()),
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

async fn channel_test(
    State(state): State<AppState>,
    jar: CookieJar,
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
    let csrf = current_csrf(&state, &jar);
    render_project_page(
        &state.store,
        project,
        Some(result),
        false,
        user.is_admin,
        csrf,
    )
    .await
}

/// Only ids belonging to the check's own project are honoured.
async fn set_channels_core(
    state: &AppState,
    check: &Check,
    form: BindForm,
    admin: bool,
    jar: CookieJar,
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
    let jar = jar.add(flash_cookie(&state.config, "channels"));
    Ok((jar, Redirect::to(&format!("{base}/checks/{id}"))).into_response())
}

async fn check_set_channels(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    jar: CookieJar,
    HtmlForm(form): HtmlForm<BindForm>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    set_channels_core(&state, &check, form, false, jar).await
}

// --- settings / user administration (admin only) ---
#[derive(Deserialize)]
struct SettingsForm {
    scan_interval: String,
    nag_interval: String,
    pings_retention_days: String,
    notifications_retention_days: String,
    audit_retention_days: String,
    #[serde(default)]
    display_timezone: String,
}

/// How a setting is validated before being stored as a string.
#[derive(Clone, Copy)]
enum SettingKind {
    /// Raw seconds or a human duration (`5m`, `1h30m`).
    Duration,
    /// A plain positive integer count of days.
    Days,
    /// An IANA timezone name.
    Timezone,
}

/// Blank clears the setting, which is how every numeric setting spells "unset".
fn fmt_opt_setting(v: Option<i64>) -> String {
    v.map(|n| n.to_string()).unwrap_or_default()
}

/// Blank = unset (use the check's own zone), unlike a check's field where blank = UTC.
fn validate_opt_timezone(raw: &str) -> Result<String, String> {
    if raw.trim().is_empty() {
        return Ok(String::new());
    }
    validate_timezone(raw)
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

/// Seconds → readable duration; anything else passes through untouched.
fn readable_setting_duration(raw: String) -> String {
    match raw.trim().parse::<i64>() {
        Ok(v) if v > 0 => crate::duration::fmt_duration(v),
        _ => raw,
    }
}

/// Stored settings for the form; used by `admin_page` and [`render_admin_user_error`].
async fn load_settings_fields(state: &AppState) -> Result<SettingsFields, AppError> {
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
    let audit_retention_days = state
        .store
        .get_setting("audit_retention_days")
        .await?
        .unwrap_or_default();
    let display_timezone = state
        .store
        .get_setting("display_timezone")
        .await?
        .unwrap_or_default();
    Ok(SettingsFields {
        scan_interval: readable_setting_duration(scan_interval),
        nag_interval: readable_setting_duration(nag_interval),
        display_timezone,
        pings_retention_days,
        notifications_retention_days,
        audit_retention_days,
    })
}

/// Persisted values, or an invalid save's raw input so it can be fixed.
struct SettingsFields {
    scan_interval: String,
    nag_interval: String,
    pings_retention_days: String,
    notifications_retention_days: String,
    audit_retention_days: String,
    display_timezone: String,
}

/// The per-request parts of [`render_admin`]; it loads everything else.
struct AdminRender {
    settings: SettingsFields,
    settings_error: Option<String>,
    settings_flash: Option<String>,
    user_flash: Option<String>,
    /// Separate from `user_flash`, which is the error slot.
    elevation_flash: Option<String>,
    password_reset_flash: Option<String>,
    user_error: Option<String>,
}

async fn admin_page(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    Query(audit): Query<AdminAuditQuery>,
) -> Result<Response, AppError> {
    let settings = load_settings_fields(&state).await?;
    // Chained: each `take_flash` consumes only its own surface's cookie.
    let (jar, settings_flash) = take_flash(&state.config, jar, "settings");
    let (jar, user_flash) = take_flash(&state.config, jar, "users_blocked");
    // `admin_locked` is left for `/admin/unlock`.
    let (jar, elevation_flash) = take_flash(&state.config, jar, "admin_unlocked");
    let (jar, password_reset_flash) = take_password_reset_keys_flash(&state.config, jar);
    let resp = render_admin(
        &state,
        &jar,
        &admin,
        AdminRender {
            settings,
            settings_error: None,
            settings_flash,
            user_flash,
            elevation_flash,
            password_reset_flash,
            user_error: None,
        },
        &audit,
    )
    .await?;
    Ok((jar, resp).into_response())
}

#[derive(Template)]
#[template(path = "admin_unlock.html")]
struct AdminUnlockTemplate {
    show_nav: bool,
    csrf: String,
    is_admin: bool,
    /// False for a passwordless forward-auth admin: the gate doesn't apply.
    applies: bool,
    /// `Some(readable duration)` when already confirmed.
    remaining: Option<String>,
    /// From `elevate::ELEVATION_TTL_SECS`, so the copy can't drift.
    ttl: String,
    /// Set when bounced here by a refused action.
    bounced_flash: Option<String>,
    error: Option<String>,
}

/// A page, not an `/admin` field: it must explain why a signed-in admin is asked
/// again and that it is the same password, not a second factor.
fn render_admin_unlock(
    state: &AppState,
    jar: &CookieJar,
    admin: &User,
    bounced_flash: Option<String>,
    error: Option<String>,
) -> Result<Response, AppError> {
    let elevation = elevation(state, jar, admin);
    Ok(render(&AdminUnlockTemplate {
        show_nav: true,
        csrf: current_csrf(state, jar),
        is_admin: true,
        applies: !elevation.not_applicable,
        remaining: elevation.remaining_secs.map(fmt_elevation_secs),
        ttl: fmt_elevation_secs(crate::elevate::ELEVATION_TTL_SECS),
        bounced_flash,
        error,
    })?
    .into_response())
}

/// The same readable form the duration fields use.
fn fmt_elevation_secs(secs: u64) -> String {
    crate::duration::fmt_duration(secs.try_into().unwrap_or(i64::MAX))
}

async fn admin_unlock_page(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
) -> Result<Response, AppError> {
    let (jar, bounced) = take_flash(&state.config, jar, "admin_locked");
    let resp = render_admin_unlock(&state, &jar, &admin, bounced, None)?;
    Ok((jar, resp).into_response())
}

/// No-op without a session handle — harmless, as passwordless accounts already pass
/// via `Elevation::not_applicable`.
fn grant_elevation(state: &AppState, jar: &CookieJar) {
    if let Some(handle) = current_session_handle(state, jar) {
        state.elevations.grant(&handle);
    }
}

/// Unlocks access-granting controls for `elevate::ELEVATION_TTL_SECS`, through
/// [`reauthenticate`] so wrong passwords are metered here too.
async fn admin_unlock(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    AdminUser(admin): AdminUser,
    Form(form): Form<PasswordForm>,
) -> Result<Response, AppError> {
    let outcome = reauthenticate(&state, &admin, &form.password, "admin_unlock");
    // The JS dialog wants a status, not HTML. Presentation only; already decided.
    if wants_fragment(&headers) {
        return Ok(match outcome {
            Reauth::Passed => {
                grant_elevation(&state, &jar);
                StatusCode::NO_CONTENT.into_response()
            }
            Reauth::Failed => StatusCode::FORBIDDEN.into_response(),
            Reauth::Throttled => (
                StatusCode::TOO_MANY_REQUESTS,
                [(
                    header::RETRY_AFTER,
                    crate::ratelimit::ACCOUNT_WINDOW_SECS.to_string(),
                )],
            )
                .into_response(),
        });
    }
    let refusal = match outcome {
        Reauth::Passed => None,
        Reauth::Failed => Some("That password is not correct."),
        Reauth::Throttled => Some("Too many attempts — try again later."),
    };
    if let Some(msg) = refusal {
        return render_admin_unlock(&state, &jar, &admin, None, Some(msg.to_string()));
    }
    grant_elevation(&state, &jar);
    let jar = jar.add(flash_cookie(&state.config, "admin_unlocked"));
    Ok((jar, Redirect::to("/admin")).into_response())
}

async fn settings_save(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    Form(form): Form<SettingsForm>,
) -> Result<Response, AppError> {
    let fields = [
        (
            "scan_interval",
            form.scan_interval.as_str(),
            "Global scan interval",
            SettingKind::Duration,
        ),
        (
            "nag_interval",
            form.nag_interval.as_str(),
            "Global nag interval",
            SettingKind::Duration,
        ),
        (
            "pings_retention_days",
            form.pings_retention_days.as_str(),
            "Pings retention",
            SettingKind::Days,
        ),
        (
            "notifications_retention_days",
            form.notifications_retention_days.as_str(),
            "Notifications retention",
            SettingKind::Days,
        ),
        (
            "audit_retention_days",
            form.audit_retention_days.as_str(),
            "Audit trail retention",
            SettingKind::Days,
        ),
        (
            "display_timezone",
            form.display_timezone.as_str(),
            "Notification timezone",
            SettingKind::Timezone,
        ),
    ];
    // Atomic: validate and normalise every field before writing any.
    let mut parsed: Vec<(&str, String)> = Vec::with_capacity(fields.len());
    for (key, raw, label, kind) in fields {
        let result = match kind {
            SettingKind::Duration => parse_opt_positive_duration(raw, label).map(fmt_opt_setting),
            SettingKind::Days => parse_opt_positive(raw, label).map(fmt_opt_setting),
            SettingKind::Timezone => validate_opt_timezone(raw),
        };
        match result {
            Ok(v) => parsed.push((key, v)),
            Err(msg) => {
                let resp = render_admin(
                    &state,
                    &jar,
                    &admin,
                    AdminRender {
                        settings: SettingsFields {
                            scan_interval: form.scan_interval.clone(),
                            nag_interval: form.nag_interval.clone(),
                            pings_retention_days: form.pings_retention_days.clone(),
                            notifications_retention_days: form.notifications_retention_days.clone(),
                            audit_retention_days: form.audit_retention_days.clone(),
                            display_timezone: form.display_timezone.clone(),
                        },
                        settings_error: Some(msg),
                        elevation_flash: None,
                        settings_flash: None,
                        user_flash: None,
                        password_reset_flash: None,
                        user_error: None,
                    },
                    // The audit card just comes back on its default latest page.
                    &AdminAuditQuery::default(),
                )
                .await?;
                return Ok(resp);
            }
        }
    }
    // Diffed before writing, so `detail` names the fields actually touched.
    let mut changed: Vec<String> = Vec::new();
    for (key, value) in &parsed {
        let previous = state.store.get_setting(key).await?.unwrap_or_default();
        if &previous != value {
            let shown = if value.is_empty() { "unset" } else { value };
            changed.push(format!("{key}={shown}"));
        }
    }
    for (key, value) in parsed {
        state.store.set_setting(key, &value).await?;
    }
    // Shortening `audit_retention_days` could erase an admin's trail; record it.
    if !changed.is_empty() {
        state
            .store
            .record_audit(
                &crate::store::NewAudit {
                    actor_user_id: admin.id,
                    actor_username: &admin.username,
                    action: "settings.update",
                    method: Some("POST"),
                    path: Some("/admin/settings"),
                    detail: Some(&changed.join(" ")),
                    ..Default::default()
                },
                Utc::now(),
            )
            .await?;
    }
    let jar = jar.add(flash_cookie(&state.config, "settings"));
    Ok((jar, Redirect::to("/admin")).into_response())
}

/// Worded once, so the pre-check and the constraint race cannot drift apart.
fn username_taken(username: &str) -> String {
    format!("A user named \"{username}\" already exists.")
}

/// The shared re-render for user-management refusals.
async fn render_admin_user_error(
    state: &AppState,
    jar: &CookieJar,
    admin: &User,
    error: String,
) -> Result<Response, AppError> {
    let settings = load_settings_fields(state).await?;
    render_admin(
        state,
        jar,
        admin,
        AdminRender {
            settings,
            settings_error: None,
            settings_flash: None,
            user_flash: None,
            elevation_flash: None,
            password_reset_flash: None,
            user_error: Some(error),
        },
        &AdminAuditQuery::default(),
    )
    .await
}

async fn users_create(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    Form(form): Form<NewUserForm>,
) -> Result<Response, AppError> {
    let username = form.username.trim();
    // Read-only checks sit above the elevation gate: a doomed submission should
    // say why rather than demand confirmation first.
    let policy = crate::auth::validate_password(&form.password);
    let taken =
        !username.is_empty() && state.store.find_user_by_username(username).await?.is_some();
    let error = if username.is_empty() {
        Some("username and password are required".to_string())
    } else if let Err(msg) = policy {
        Some(msg)
    } else if taken {
        // Exact match, like the `UNIQUE` constraint: `Admin` and `admin` differ.
        Some(username_taken(username))
    } else {
        None
    };
    if let Some(error) = error {
        return render_admin_user_error(&state, &jar, &admin, error).await;
    }
    // Gated: a new account outlives this session. Keep directly above the first write.
    if !elevation(&state, &jar, &admin).allows() {
        return Ok(admin_locked(&state.config, jar));
    }
    let phc = hash_password(&form.password).map_err(|e| AppError::Other(e.to_string().into()))?;
    // Absent or empty both mean an unchecked box.
    let is_admin = form.is_admin.as_deref().is_some_and(|s| !s.is_empty());
    // Two admins can race the pre-check; the constraint arbitrates, same message.
    let new_id = match state
        .store
        .create_user(username, Some(&phc), is_admin, Utc::now())
        .await
    {
        Ok(id) => id,
        Err(crate::store::CreateUserError::UsernameTaken) => {
            return render_admin_user_error(&state, &jar, &admin, username_taken(username)).await;
        }
        Err(crate::store::CreateUserError::Db(e)) => return Err(e.into()),
    };
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
    Ok(Redirect::to("/admin").into_response())
}

async fn users_delete(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
    Query(confirm): Query<ConfirmQuery>,
) -> Result<Response, AppError> {
    if id == admin.id {
        return Ok(users_blocked(&state.config, jar));
    }
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/admin").into_response());
    };
    // Unreachable (the actor is another enabled admin); defence in depth.
    if target.is_admin && !target.disabled && state.store.count_enabled_admins().await? <= 1 {
        return Ok(users_blocked(&state.config, jar));
    }
    // Below both refusals, so a blocked delete isn't asked to confirm first.
    if !confirm.is_confirmed() {
        return confirmation_page(
            &state,
            &jar,
            true,
            &CONFIRM_DELETE_USER,
            &format!("/admin/users/{id}/delete"),
            "/admin".to_string(),
        );
    }
    state.store.delete_user(id).await?;
    // No `count`: sessions go via ON DELETE CASCADE.
    tracing::info!(
        target: "pingward::session",
        reason = "user_deleted",
        user_id = id,
        actor_user_id = admin.id,
        "session.destroyed"
    );
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
    Ok(Redirect::to("/admin").into_response())
}

async fn users_set_password(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
    jar: CookieJar,
    Form(form): Form<PasswordForm>,
) -> Result<Response, AppError> {
    // Mints a credential this admin knows, outliving the session that set it.
    if !elevation(&state, &jar, &admin).allows() {
        return Ok(admin_locked(&state.config, jar));
    }
    // Rendered, not a silent redirect: an unchanged page would read as success.
    if let Err(msg) = crate::auth::validate_password(&form.password) {
        return render_admin_user_error(&state, &jar, &admin, msg).await;
    }
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/admin").into_response());
    };
    let phc = hash_password(&form.password).map_err(|e| AppError::Other(e.to_string().into()))?;
    state.store.set_user_password(id, &phc).await?;
    // End sessions, or an intruder's cookie survives the reset. A self-reset spares
    // the current session; evicting an attacker on it also needs a logout.
    let revoked = if id == admin.id {
        match secret::session_id_from_jar(&jar, &state.config.secret, session_cookie_name(&state)) {
            Some(current) => {
                state
                    .store
                    .delete_other_sessions_for_user(id, &current)
                    .await?
            }
            // Unreachable for an authenticated `AdminUser`; fail safe anyway.
            None => state.store.delete_sessions_for_user(id).await?,
        }
    } else {
        state.store.delete_sessions_for_user(id).await?
    };
    tracing::info!(
        target: "pingward::session",
        reason = "password_reset",
        user_id = id,
        count = revoked,
        actor_user_id = admin.id,
        "session.destroyed"
    );
    let detail = format!("sessions_revoked={revoked}");
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "user.password_reset",
                target_type: Some("user"),
                target_id: Some(id),
                detail: Some(&detail),
                ..Default::default()
            },
            Utc::now(),
        )
        .await?;
    // Warn about API keys, which survive the reset. Expired ones don't count:
    // `validate_api_key` already refuses them.
    let now = Utc::now();
    let key_count = state
        .store
        .list_api_keys_for_user(id)
        .await?
        .iter()
        .filter(|k| k.expires_at.is_none_or(|e| e > now))
        .count() as u64;
    // A disabled account's keys are already inert (`ApiUser` re-checks).
    if key_count > 0 && !target.disabled {
        return Ok(password_reset_keys_flash(
            &state.config,
            jar,
            revoked,
            key_count,
        ));
    }
    Ok(Redirect::to("/admin").into_response())
}

async fn users_toggle_admin(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
    Query(confirm): Query<ConfirmQuery>,
) -> Result<Response, AppError> {
    // Would lock this admin out of `/admin` on the very next request.
    if id == admin.id {
        return Ok(users_blocked(&state.config, jar));
    }
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/admin").into_response());
    };
    let new_admin = !target.is_admin;
    // Gated only when *granting*: removing access must not need a password.
    if new_admin && !elevation(&state, &jar, &admin).allows() {
        return Ok(admin_locked(&state.config, jar));
    }
    // Unreachable, as in `users_delete`; defence in depth.
    if !new_admin
        && target.is_admin
        && !target.disabled
        && state.store.count_enabled_admins().await? <= 1
    {
        return Ok(users_blocked(&state.config, jar));
    }
    // Confirm demotion only (as the template); promotion passed the elevation gate.
    if !new_admin && !confirm.is_confirmed() {
        return confirmation_page(
            &state,
            &jar,
            true,
            &CONFIRM_DEMOTE_ADMIN,
            &format!("/admin/users/{id}/admin"),
            "/admin".to_string(),
        );
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
    Ok(Redirect::to("/admin").into_response())
}

async fn users_set_disabled(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
    Query(confirm): Query<ConfirmQuery>,
) -> Result<Response, AppError> {
    if id == admin.id {
        return Ok(users_blocked(&state.config, jar));
    }
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/admin").into_response());
    };
    let new_disabled = !target.disabled;
    // Unreachable, as in `users_delete`; defence in depth.
    if new_disabled
        && target.is_admin
        && !target.disabled
        && state.store.count_enabled_admins().await? <= 1
    {
        return Ok(users_blocked(&state.config, jar));
    }
    // Disable only, matching the template: re-enabling needs no ceremony.
    if new_disabled && !confirm.is_confirmed() {
        return confirmation_page(
            &state,
            &jar,
            true,
            &CONFIRM_DISABLE_USER,
            &format!("/admin/users/{id}/disabled"),
            "/admin".to_string(),
        );
    }
    state.store.set_user_disabled(id, new_disabled).await?;
    // On disable, or disable→enable resurrects sessions (`resolve_user` blocks only
    // while disabled).
    let revoked = if new_disabled {
        state.store.delete_sessions_for_user(id).await?
    } else {
        0
    };
    if new_disabled {
        tracing::info!(
            target: "pingward::session",
            reason = "user_disabled",
            user_id = id,
            count = revoked,
            actor_user_id = admin.id,
            "session.destroyed"
        );
    }
    let detail = if new_disabled {
        format!("disable sessions_revoked={revoked}")
    } else {
        "enable".to_string()
    };
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "user.set_disabled",
                target_type: Some("user"),
                target_id: Some(id),
                detail: Some(&detail),
                ..Default::default()
            },
            Utc::now(),
        )
        .await?;
    Ok(Redirect::to("/admin").into_response())
}

// --- account: sessions, password, API keys ---
// `sessions.id` is the cookie's bearer secret, never rendered or put in a URL; rows
// are named by `handle` (SHA-256 of the id), resolved back by a linear scan.
#[derive(Template)]
#[template(path = "account.html")]
struct AccountTemplate {
    show_nav: bool,
    csrf: String,
    is_admin: bool,
    // sessions section
    sessions: Vec<SessionRow>,
    /// Hides the "revoke others" control when there is nothing else to revoke.
    other_count: usize,
    /// False for a passwordless forward-auth account: hides the password card and the
    /// API-key re-auth field ([`reauthenticate`] passes it unchallenged).
    has_password: bool,
    // password section
    password_error: Option<String>,
    password_flash: Option<String>,
    // api-keys section
    keys: Vec<ApiKeyRow>,
    /// Rendered exactly once, right after creation; never recoverable after.
    new_token: Option<String>,
    key_error: Option<String>,
}

/// The per-request parts of [`render_account`].
#[derive(Default)]
struct AccountRender {
    new_token: Option<String>,
    key_error: Option<String>,
    password_error: Option<String>,
    password_flash: Option<String>,
}

/// [`crate::models::Session`] minus the raw `id`, plus `handle` and `current`.
struct SessionRow {
    handle: String,
    created_at: Option<DateTime<Utc>>,
    last_seen_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
    user_agent: Option<String>,
    ip: Option<String>,
    current: bool,
    sso: bool,
}

/// An expired key still lists so it can be revoked, hence the `expired` flag.
struct ApiKeyRow {
    id: i64,
    name: String,
    prefix: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    expired: bool,
}

impl ApiKeyRow {
    fn from_key(k: crate::models::ApiKey, now: DateTime<Utc>) -> Self {
        let expired = k.expires_at.is_some_and(|t| t <= now);
        Self {
            id: k.id,
            name: k.name,
            prefix: k.prefix,
            created_at: k.created_at,
            last_used_at: k.last_used_at,
            expires_at: k.expires_at,
            expired,
        }
    }
}

#[derive(Deserialize)]
struct NewApiKeyForm {
    name: String,
    #[serde(default)]
    expires_in: String,
    /// See [`reauthenticate`]. Defaulted: a passwordless account's form omits it.
    #[serde(default)]
    current_password: String,
}

async fn account_page(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
) -> Result<Response, AppError> {
    let (jar, password_flash) = take_flash(&state.config, jar, "password_changed");
    let resp = render_account(
        &state,
        &jar,
        &user,
        AccountRender {
            password_flash,
            ..Default::default()
        },
    )
    .await?;
    Ok((jar, resp).into_response())
}

async fn render_account(
    state: &AppState,
    jar: &CookieJar,
    user: &User,
    parts: AccountRender,
) -> Result<Response, AppError> {
    let now = Utc::now();

    let current_handle = current_session_handle(state, jar);
    // Reap past-the-cap rows so "not listed" means "gone".
    state
        .store
        .delete_capped_sessions_for_user(user.id, now)
        .await?;
    let mut sessions: Vec<SessionRow> = state
        .store
        .list_sessions_for_user(user.id, now)
        .await?
        .into_iter()
        .map(|s| {
            let handle = crate::apikey::hash_api_key(&s.id);
            let current = current_handle.as_deref() == Some(handle.as_str());
            SessionRow {
                handle,
                created_at: s.created_at,
                last_seen_at: s.last_seen_at,
                expires_at: s.expires_at,
                user_agent: s.user_agent,
                ip: s.ip,
                current,
                sso: s.sso,
            }
        })
        .collect();
    // Stable: current first, newest-first kept within each group.
    sessions.sort_by_key(|r| !r.current);
    let other_count = sessions.iter().filter(|r| !r.current).count();

    let keys = state
        .store
        .list_api_keys_for_user(user.id)
        .await?
        .into_iter()
        .map(|k| ApiKeyRow::from_key(k, now))
        .collect();

    Ok(render(&AccountTemplate {
        show_nav: true,
        csrf: current_csrf(state, jar),
        is_admin: user.is_admin,
        sessions,
        other_count,
        has_password: user.password_hash.is_some(),
        password_error: parts.password_error,
        password_flash: parts.password_flash,
        keys,
        new_token: parts.new_token,
        key_error: parts.key_error,
    })?
    .into_response())
}

#[derive(Deserialize)]
struct ChangePasswordForm {
    current_password: String,
    new_password: String,
    confirm_password: String,
}

/// Requires the current password, which a hijacked cookie can't supply. Passwordless
/// forward-auth accounts are refused: a local password would be a way in the gateway's
/// sign-out couldn't end. Success revokes every *other* session; API keys survive.
async fn account_password(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Form(form): Form<ChangePasswordForm>,
) -> Result<Response, AppError> {
    let error = |msg: &str| AccountRender {
        password_error: Some(msg.to_string()),
        ..Default::default()
    };
    // 403, not a message: the card is hidden, so only a crafted request gets here.
    if user.password_hash.is_none() {
        return Ok((StatusCode::FORBIDDEN, "this account has no local password").into_response());
    }
    // The shared, metered gate, not a bare `verify_password`.
    match reauthenticate(&state, &user, &form.current_password, "password_change") {
        Reauth::Passed => {}
        Reauth::Failed => {
            let parts = error("Current password is incorrect.");
            return render_account(&state, &jar, &user, parts).await;
        }
        Reauth::Throttled => {
            let parts = error("Too many attempts — try again later.");
            return render_account(&state, &jar, &user, parts).await;
        }
    }
    // Policy before the match check, so the more useful error wins.
    if let Err(msg) = crate::auth::validate_password(&form.new_password) {
        let parts = error(&msg);
        return render_account(&state, &jar, &user, parts).await;
    }
    if form.new_password != form.confirm_password {
        let parts = error("The new passwords do not match.");
        return render_account(&state, &jar, &user, parts).await;
    }
    let phc =
        hash_password(&form.new_password).map_err(|e| AppError::Other(e.to_string().into()))?;
    state.store.set_user_password(user.id, &phc).await?;
    let revoked = match secret::session_id_from_jar(
        &jar,
        &state.config.secret,
        session_cookie_name(&state),
    ) {
        Some(current) => {
            state
                .store
                .delete_other_sessions_for_user(user.id, &current)
                .await?
        }
        // Unreachable here; revoking everything is the safe direction anyway.
        None => state.store.delete_sessions_for_user(user.id).await?,
    };
    tracing::info!(
        target: "pingward::session",
        reason = "password_change",
        user_id = user.id,
        count = revoked,
        "session.destroyed"
    );
    let detail = format!("sessions_revoked={revoked}");
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: user.id,
                actor_username: &user.username,
                action: "user.password_change",
                target_type: Some("user"),
                target_id: Some(user.id),
                detail: Some(&detail),
                ..Default::default()
            },
            Utc::now(),
        )
        .await?;
    let jar = jar.add(flash_cookie(&state.config, "password_changed"));
    Ok((jar, Redirect::to("/account")).into_response())
}

async fn api_keys_create(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Form(form): Form<NewApiKeyForm>,
) -> Result<Response, AppError> {
    let key_error = |msg: &str| AccountRender {
        key_error: Some(msg.to_string()),
        ..Default::default()
    };
    // A key escapes both session caps and survives password resets, so a borrowed
    // browser would otherwise buy permanent access.
    match reauthenticate(&state, &user, &form.current_password, "api_key_create") {
        Reauth::Passed => {}
        Reauth::Failed => {
            return render_account(
                &state,
                &jar,
                &user,
                key_error("Current password is incorrect."),
            )
            .await;
        }
        Reauth::Throttled => {
            return render_account(
                &state,
                &jar,
                &user,
                key_error("Too many attempts — try again later."),
            )
            .await;
        }
    }
    let name = form.name.trim();
    if name.is_empty() {
        return render_account(&state, &jar, &user, key_error("a name is required")).await;
    }
    // Blank = never; otherwise a duration from now.
    let expires_at = {
        let raw = form.expires_in.trim();
        if raw.is_empty() {
            None
        } else {
            match crate::duration::parse_duration(raw) {
                Some(secs) if secs > 0 => Some(Utc::now() + Duration::seconds(secs)),
                _ => {
                    return render_account(
                        &state,
                        &jar,
                        &user,
                        key_error("expiry must be a duration like 30d, or blank for never"),
                    )
                    .await;
                }
            }
        }
    };
    let (full, prefix, hash) = crate::apikey::generate_api_key();
    state
        .store
        .insert_api_key(user.id, name, &hash, &prefix, expires_at, Utc::now())
        .await?;
    let parts = AccountRender {
        new_token: Some(full),
        ..Default::default()
    };
    render_account(&state, &jar, &user, parts).await
}

async fn api_keys_delete(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Query(confirm): Query<ConfirmQuery>,
) -> Result<Response, AppError> {
    if !confirm.is_confirmed() {
        return confirmation_page(
            &state,
            &jar,
            user.is_admin,
            &CONFIRM_REVOKE_API_KEY,
            &format!("/account/api-keys/{id}/delete"),
            "/account".to_string(),
        );
    }
    // Owner-scoped; a key the caller does not own is silently a no-op.
    state.store.delete_api_key(id, user.id).await?;
    Ok(Redirect::to("/account").into_response())
}

async fn sessions_revoke(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(handle): Path<String>,
    Query(confirm): Query<ConfirmQuery>,
) -> Result<Response, AppError> {
    // Owner-scoped: an unknown or foreign handle is a silent no-op, never a 500.
    let sessions = state
        .store
        .list_sessions_for_user(user.id, Utc::now())
        .await?;
    let Some(target) = sessions
        .iter()
        .find(|s| crate::apikey::hash_api_key(&s.id) == handle)
    else {
        return Ok((jar, Redirect::to("/account")).into_response());
    };
    // After the lookup, so an unknown handle gets no confirmation page.
    if !confirm.is_confirmed() {
        return confirmation_page(
            &state,
            &jar,
            user.is_admin,
            &CONFIRM_REVOKE_SESSION,
            &format!("/account/sessions/{handle}/revoke"),
            "/account".to_string(),
        );
    }
    let is_current =
        secret::session_id_from_jar(&jar, &state.config.secret, session_cookie_name(&state))
            .is_some_and(|id| id == target.id);
    state
        .store
        .delete_session_owned(&target.id, user.id)
        .await?;
    tracing::info!(
        target: "pingward::session",
        reason = "revoked",
        handle = %crate::auth::session_log_handle(&target.id),
        user_id = user.id,
        is_current,
        "session.destroyed"
    );
    if is_current {
        // Removal must carry `path("/")` like the original, or it clears nothing.
        let jar = jar.remove(session_removal_cookie(&state.config));
        return Ok((jar, Redirect::to("/login")).into_response());
    }
    Ok((jar, Redirect::to("/account")).into_response())
}

async fn sessions_revoke_others(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Query(confirm): Query<ConfirmQuery>,
) -> Result<Response, AppError> {
    if !confirm.is_confirmed() {
        return confirmation_page(
            &state,
            &jar,
            user.is_admin,
            &CONFIRM_REVOKE_OTHER_SESSIONS,
            "/account/sessions/revoke-others",
            "/account".to_string(),
        );
    }
    if let Some(id) =
        secret::session_id_from_jar(&jar, &state.config.secret, session_cookie_name(&state))
    {
        let count = state
            .store
            .delete_other_sessions_for_user(user.id, &id)
            .await?;
        tracing::info!(
            target: "pingward::session",
            reason = "revoke_others",
            user_id = user.id,
            count,
            "session.destroyed"
        );
    }
    Ok(Redirect::to("/account").into_response())
}

/// Secrets are redacted in Rust before reaching here, so no template can print one.
struct EnvSetting {
    var: &'static str,
    value: EnvValue,
    default: &'static str,
    description: &'static str,
}

/// `Secret` carries only whether something is configured, never the value.
enum EnvValue {
    Set(String),
    Unset,
    Secret(bool),
}

/// The `/admin` "Environment" card, from the effective config (what is running).
fn env_settings(config: &crate::config::Config) -> Vec<(&'static str, Vec<EnvSetting>)> {
    let log_format = match config.log_format {
        crate::config::LogFormat::Full => "full",
        crate::config::LogFormat::Compact => "compact",
        crate::config::LogFormat::Pretty => "pretty",
        crate::config::LogFormat::Json => "json",
    };
    let server = vec![
        EnvSetting {
            var: "DATABASE_URL",
            value: EnvValue::Set(redact_db_url(&config.database_url)),
            default: "sqlite://pingward.sqlite3?mode=rwc",
            description: "The database connection string (SQLite or Postgres).",
        },
        EnvSetting {
            var: "PINGWARD_BIND",
            value: EnvValue::Set(config.bind.clone()),
            default: "127.0.0.1:8080",
            description: "The socket address the server listens on.",
        },
        EnvSetting {
            var: "PINGWARD_BASE_URL",
            value: EnvValue::Set(config.base_url.clone()),
            default: "http://localhost:8080",
            description: "Used to render absolute ping URLs.",
        },
        EnvSetting {
            var: "PINGWARD_LOG_FORMAT",
            value: EnvValue::Set(log_format.to_string()),
            default: "full",
            description: "Log line format (full, compact, pretty or json); applied at process startup — changing it requires a restart.",
        },
        EnvSetting {
            var: "PINGWARD_HSTS_MAX_AGE",
            value: EnvValue::Set(config.hsts_max_age_secs.to_string()),
            default: "0 (off)",
            description: "Strict-Transport-Security max-age in seconds; 0 sends no header. Prefer setting this on the reverse proxy — see README.",
        },
    ];
    let scheduling = vec![
        EnvSetting {
            var: "PINGWARD_SCAN_INTERVAL",
            value: EnvValue::Set(config.scan_interval_secs.to_string()),
            default: "30",
            description: "Fallback scan interval — only used when no check, project, or global setting overrides it (check → project → global setting → env cascade).",
        },
        EnvSetting {
            var: "PINGWARD_PRUNE_INTERVAL_SECS",
            value: EnvValue::Set(config.prune_interval_secs.to_string()),
            default: "3600",
            description: "How often old pings/notifications are deleted.",
        },
    ];
    let auth = vec![
        EnvSetting {
            var: "PINGWARD_FORWARD_AUTH_HEADER",
            value: match &config.forward_auth_header {
                Some(h) => EnvValue::Set(h.clone()),
                None => EnvValue::Unset,
            },
            default: "(unset)",
            description: "Header name for the trusted-proxy forward-auth mechanism.",
        },
        EnvSetting {
            var: "PINGWARD_TRUSTED_PROXIES",
            value: if config.trusted_proxies.is_empty() {
                EnvValue::Unset
            } else {
                EnvValue::Set(config.trusted_proxies.join(", "))
            },
            default: "(unset)",
            description: "Proxy addresses trusted to set the forward-auth header.",
        },
        EnvSetting {
            var: "PINGWARD_FORWARD_AUTH_LOGOUT_URL",
            value: match &config.forward_auth_logout_url {
                Some(u) => EnvValue::Set(u.clone()),
                None => EnvValue::Unset,
            },
            default: "(unset)",
            description: "Where logging out sends the browser — point it at the gateway's sign-out endpoint to end the SSO session too. Unset means /login.",
        },
        EnvSetting {
            var: "PINGWARD_COOKIE_SECURE",
            value: EnvValue::Set(config.cookie_secure.to_string()),
            default: "derived from PINGWARD_BASE_URL's scheme",
            description: "Whether the session cookie carries `Secure`. Leave unset unless TLS terminates upstream and PINGWARD_BASE_URL cannot say so.",
        },
    ];
    let smtp = &config.smtp;
    let email = vec![
        EnvSetting {
            var: "PINGWARD_SMTP_HOST",
            value: match smtp {
                Some(s) => EnvValue::Set(s.host.clone()),
                None => EnvValue::Unset,
            },
            default: "(unset)",
            description: "Instance SMTP server host.",
        },
        EnvSetting {
            var: "PINGWARD_SMTP_PORT",
            value: match smtp {
                Some(s) => EnvValue::Set(s.port.to_string()),
                None => EnvValue::Unset,
            },
            default: "(unset)",
            description: "Instance SMTP server port.",
        },
        EnvSetting {
            var: "PINGWARD_SMTP_FROM",
            value: match smtp {
                Some(s) => EnvValue::Set(s.from.clone()),
                None => EnvValue::Unset,
            },
            default: "(unset)",
            description: "The From address used for outgoing email notifications.",
        },
        EnvSetting {
            var: "PINGWARD_SMTP_TLS",
            value: match smtp {
                Some(s) => EnvValue::Set(
                    match s.tls {
                        crate::config::SmtpTls::Starttls => "starttls",
                        crate::config::SmtpTls::Tls => "tls",
                        crate::config::SmtpTls::None => "none",
                    }
                    .to_string(),
                ),
                None => EnvValue::Unset,
            },
            default: "(unset)",
            description: "SMTP transport security mode (starttls, tls, or none).",
        },
        EnvSetting {
            var: "PINGWARD_SMTP_USERNAME",
            value: match smtp.as_ref().and_then(|s| s.username.as_deref()) {
                Some(u) => EnvValue::Set(u.to_string()),
                None => EnvValue::Unset,
            },
            default: "(unset)",
            description: "SMTP AUTH username (an identity, not a credential — shown verbatim).",
        },
        EnvSetting {
            var: "PINGWARD_SMTP_PASSWORD",
            value: EnvValue::Secret(smtp.as_ref().is_some_and(|s| s.password.is_some())),
            default: "(unset)",
            description: "SMTP AUTH password — never displayed, only whether it's configured.",
        },
    ];
    vec![
        ("Server", server),
        ("Scheduling", scheduling),
        ("Auth", auth),
        ("Email (SMTP)", email),
    ]
}

/// `scheme://user:pw@host/db` becomes `scheme://***@host/db`; an authority with
/// no `@` (a plain `SQLite` path) is returned unchanged.
fn redact_db_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let authority_start = scheme_end + 3;
    let rest = &url[authority_start..];
    // Only an `@` before `/`, `?` or `#` ends userinfo; a later one is path/query.
    let mut at_pos = None;
    for (i, c) in rest.char_indices() {
        match c {
            '@' => {
                at_pos = Some(i);
                break;
            }
            '/' | '?' | '#' => break,
            _ => {}
        }
    }
    match at_pos {
        Some(i) => format!("{}***@{}", &url[..authority_start], &rest[i + 1..]),
        None => url.to_string(),
    }
}

/// `a*`-prefixed to share `/admin`'s query string. Missing fields mean the unfiltered
/// latest page; bad filter text is ignored, but a non-numeric cursor is a 400.
#[derive(Deserialize, Default)]
struct AdminAuditQuery {
    #[serde(default)]
    ab: Option<i64>,
    #[serde(default)]
    aa: Option<i64>,
    #[serde(default)]
    aactor: Option<String>,
    #[serde(default)]
    aaction: Option<String>,
    #[serde(default)]
    afrom: Option<String>,
    #[serde(default)]
    ato: Option<String>,
}

/// Served by `GET /admin/audit` and inlined into `/admin`, like [`CheckPingsTemplate`].
#[derive(Template)]
#[template(path = "admin_audit.html")]
struct AdminAuditTemplate {
    rows: Vec<AuditRow>,
    empty: bool,
    /// Every actor/action present in the trail, for the two filter selects.
    actors: Vec<String>,
    actions: Vec<String>,
    /// `""` = all, echoed back into the controls.
    f_actor: String,
    f_action: String,
    f_from: String,
    f_to: String,
    /// Switches the empty state's wording and shows the Clear link.
    filtered: bool,
    newer: Option<String>,
    older: Option<String>,
}

/// Every check-page filter token, so each section can carry the other's half.
fn check_page_filter_tokens(
    f_kind: &str,
    p_from: &str,
    p_to: &str,
    f_event: &str,
    f_status: &str,
    n_from: &str,
    n_to: &str,
) -> [(&'static str, String); 7] {
    [
        ("pk", f_kind.to_string()),
        ("pfrom", p_from.to_string()),
        ("pto", p_to.to_string()),
        ("ne", f_event.to_string()),
        ("ns", f_status.to_string()),
        ("nfrom", n_from.to_string()),
        ("nto", n_to.to_string()),
    ]
}

/// The query keys each history section's visible controls own.
const PINGS_FILTER_KEYS: [&str; 3] = ["pk", "pfrom", "pto"];
const NOTIFS_FILTER_KEYS: [&str; 4] = ["ne", "ns", "nfrom", "nto"];

/// Four visible columns (when/actor/action/target); the rest of `models::AuditLog`
/// sits in an expandable row.
struct AuditRow {
    time: String,
    iso: String,
    actor: String,
    action: String,
    target: String,
    method_path: String,
    detail: String,
    target_owner: String,
    /// False when every expanded field is unset (no caret onto an empty box).
    expandable: bool,
}

/// The whole `/admin` page. Colliding names take a section prefix (`settings_*`,
/// `user_*`, `user_count`/`project_count`).
#[derive(Template)]
#[template(path = "admin.html")]
struct AdminTemplate {
    show_nav: bool,
    csrf: String,
    is_admin: bool,
    /// From [`AdminAuditTemplate`], injected with `|safe`.
    audit_partial: String,
    // overview
    user_count: i64,
    project_count: i64,
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
    /// "3m ago", rendered server-side for no-JS; `app.js` re-ticks via `data-ago`.
    last_scan_ago: Option<String>,
    last_prune_ago: Option<String>,
    /// The same stamps as readable UTC; `data-ts`/`data-ago` keep the RFC3339.
    last_scan_utc: Option<String>,
    last_prune_utc: Option<String>,
    // settings
    scan_interval: String,
    nag_interval: String,
    pings_retention_days: String,
    notifications_retention_days: String,
    audit_retention_days: String,
    display_timezone: String,
    settings_error: Option<String>,
    settings_flash: Option<String>,
    // users
    users: Vec<UserRow>,
    user_flash: Option<String>,
    elevation_flash: Option<String>,
    /// False for a passwordless forward-auth admin (nothing to re-assert).
    elevation_applies: bool,
    /// `Some(readable duration)` while the unlock is live, `None` when locked.
    elevation_remaining: Option<String>,
    /// Applies and unconfirmed; only then do gated controls carry `data-reauth`. The
    /// server re-checks, so a lapse before the click just bounces to `/admin/unlock`.
    elevation_locked: bool,
    password_reset_flash: Option<String>,
    user_error: Option<String>,
    projects: Vec<(Project, String)>,
    env_rows: Vec<(&'static str, Vec<EnvSetting>)>,
}

/// Plus `is_self`, so the admin's own row renders its self-mutation controls inert.
struct UserRow {
    id: i64,
    username: String,
    is_admin: bool,
    disabled: bool,
    is_self: bool,
}

impl UserRow {
    fn from_user(u: User, admin_id: i64) -> Self {
        Self {
            id: u.id,
            is_self: u.id == admin_id,
            username: u.username,
            is_admin: u.is_admin,
            disabled: u.disabled,
        }
    }
}

/// `r` holds what varies across the three callers; the rest is loaded fresh.
async fn render_admin(
    state: &AppState,
    jar: &CookieJar,
    admin: &User,
    r: AdminRender,
    audit: &AdminAuditQuery,
) -> Result<Response, AppError> {
    let now = Utc::now();
    let day_ago = now - Duration::days(1);
    // Rendered here so `/admin` and `/admin/audit` emit identical markup.
    let audit_partial = render(&build_audit_partial(state, audit).await?)?.0;
    let last_scan_at = state.store.get_setting("last_scan_at").await?;
    let last_prune_at = state.store.get_setting("last_prune_at").await?;
    let (notif_ok, notif_err) = state.store.notification_counts_since(day_ago).await?;
    let elevation = elevation(state, jar, admin);
    Ok(render(&AdminTemplate {
        show_nav: true,
        csrf: current_csrf(state, jar),
        is_admin: true,
        audit_partial,
        user_count: state.store.count_users().await?,
        project_count: state.store.count_projects().await?,
        checks: state.store.count_checks().await?,
        pings_24h: state.store.count_pings_since(day_ago).await?,
        status: state.store.count_checks_by_status().await?,
        down: state.store.list_down_checks_with_owner().await?,
        notif_ok,
        notif_err,
        channel_fail: state.store.channel_failure_counts_since(day_ago).await?,
        recent_fail: state.store.recent_failed_notifications(10).await?,
        last_scan_at: last_scan_at.clone(),
        last_prune_at: last_prune_at.clone(),
        last_scan_ago: relative_setting(last_scan_at.as_deref(), now),
        last_prune_ago: relative_setting(last_prune_at.as_deref(), now),
        last_scan_utc: absolute_setting(last_scan_at.as_deref()),
        last_prune_utc: absolute_setting(last_prune_at.as_deref()),
        scan_interval: r.settings.scan_interval,
        nag_interval: r.settings.nag_interval,
        pings_retention_days: r.settings.pings_retention_days,
        notifications_retention_days: r.settings.notifications_retention_days,
        audit_retention_days: r.settings.audit_retention_days,
        display_timezone: r.settings.display_timezone,
        settings_error: r.settings_error,
        settings_flash: r.settings_flash,
        users: state
            .store
            .list_users()
            .await?
            .into_iter()
            .map(|u| UserRow::from_user(u, admin.id))
            .collect(),
        user_flash: r.user_flash,
        elevation_flash: r.elevation_flash,
        elevation_applies: !elevation.not_applicable,
        elevation_locked: !elevation.not_applicable && elevation.remaining_secs.is_none(),
        elevation_remaining: elevation.remaining_secs.map(fmt_elevation_secs),
        password_reset_flash: r.password_reset_flash,
        user_error: r.user_error,
        projects: state.store.list_all_projects_with_owner().await?,
        env_rows: env_settings(&state.config),
    })?
    .into_response())
}

/// [`build_pings_partial`]'s twin over the `a*` params.
async fn build_audit_partial(
    state: &AppState,
    q: &AdminAuditQuery,
) -> Result<AdminAuditTemplate, AppError> {
    let filter = AuditFilter {
        actor: parse_filter_text(q.aactor.as_deref()),
        action: parse_filter_text(q.aaction.as_deref()),
        from: parse_date_bound(q.afrom.as_deref()),
        to: parse_date_bound(q.ato.as_deref()),
    };
    let cursor = match (q.ab, q.aa) {
        (Some(b), _) => PageCursor::Before(b),
        (None, Some(a)) => PageCursor::After(a),
        (None, None) => PageCursor::Latest,
    };
    let page = state.store.list_audit_page(cursor, 20, &filter).await?;
    let (actors, actions) = state.store.audit_filter_options().await?;

    let rows: Vec<AuditRow> = page
        .items
        .iter()
        .map(|a| {
            let target = match (a.target_type.as_deref(), a.target_id) {
                (Some(t), Some(id)) => format!("{t} #{id}"),
                (Some(t), None) => t.to_string(),
                _ => "—".into(),
            };
            let method_path = match (a.method.as_deref(), a.path.as_deref()) {
                (Some(m), Some(p)) => format!("{m} {p}"),
                (Some(m), None) => m.to_string(),
                (None, Some(p)) => p.to_string(),
                (None, None) => "—".into(),
            };
            AuditRow {
                time: a.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                iso: a.created_at.to_rfc3339(),
                actor: a.actor_username.clone(),
                action: a.action.clone(),
                target,
                method_path,
                detail: a.detail.clone().unwrap_or_default(),
                target_owner: a
                    .target_owner_id
                    .map_or_else(|| "—".into(), |id| format!("user #{id}")),
                expandable: a.method.is_some()
                    || a.path.is_some()
                    || a.detail.is_some()
                    || a.target_owner_id.is_some(),
            }
        })
        .collect();

    let f_actor = filter.actor.clone().unwrap_or_default();
    let f_action = filter.action.clone().unwrap_or_default();
    let f_from = date_bound_token(filter.from);
    let f_to = date_bound_token(filter.to);
    let carry = [
        ("aactor", f_actor.as_str()),
        ("aaction", f_action.as_str()),
        ("afrom", f_from.as_str()),
        ("ato", f_to.as_str()),
    ];
    let older = page
        .has_older
        .then(|| page.items.last())
        .flatten()
        .map(|a| history_href("/admin/audit", ("ab", a.id), &carry));
    let newer = page
        .has_newer
        .then(|| page.items.first())
        .flatten()
        .map(|a| history_href("/admin/audit", ("aa", a.id), &carry));

    Ok(AdminAuditTemplate {
        empty: rows.is_empty(),
        rows,
        actors,
        actions,
        f_actor,
        f_action,
        f_from,
        f_to,
        filtered: !filter.is_empty(),
        newer,
        older,
    })
}

async fn admin_audit_fragment(
    State(state): State<AppState>,
    AdminUser(_admin): AdminUser,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Query(q): Query<AdminAuditQuery>,
) -> Result<Response, AppError> {
    if !wants_fragment(&headers) {
        return Ok(fragment_page_redirect("/admin", "audit-section", &uri));
    }
    Ok(render(&build_audit_partial(&state, &q).await?)?.into_response())
}

// --- admin cross-user handlers: resolve via `admin_*`, reuse the owner cores ---
// -- projects --
async fn admin_project_show(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = admin_project(&state, id, &admin, method.as_str(), uri.path()).await?;
    let csrf = current_csrf(&state, &jar);
    render_project_page(&state.store, project, None, true, true, csrf).await
}

async fn admin_project_edit(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = admin_project(&state, id, &admin, method.as_str(), uri.path()).await?;
    let csrf = current_csrf(&state, &jar);
    Ok(render(&project_edit_form(project, true, true, csrf))?.into_response())
}

async fn admin_project_update(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Form(form): Form<ProjectForm>,
) -> Result<Response, AppError> {
    admin_project(&state, id, &admin, method.as_str(), uri.path()).await?;
    let (name, description, scan, nag) = match validate_project(&form) {
        Ok(v) => v,
        Err(msg) => {
            let csrf = current_csrf(&state, &jar);
            let t = project_form_with_error(
                "Edit project",
                format!("/admin/projects/{id}"),
                true,
                csrf,
                &form,
                msg,
            );
            return Ok(render(&t)?.into_response());
        }
    };
    state
        .store
        .update_project(id, &name, &description, scan, nag)
        .await?;
    Ok(Redirect::to(&format!("/admin/projects/{id}")).into_response())
}

async fn admin_project_delete(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Query(confirm): Query<ConfirmQuery>,
) -> Result<Response, AppError> {
    admin_project(&state, id, &admin, method.as_str(), uri.path()).await?;
    if !confirm.is_confirmed() {
        return confirmation_page(
            &state,
            &jar,
            true,
            &CONFIRM_DELETE_PROJECT,
            &format!("/admin/projects/{id}/delete"),
            format!("/admin/projects/{id}"),
        );
    }
    state.store.delete_project(id).await?;
    Ok(Redirect::to("/admin").into_response())
}

// -- checks --
async fn admin_check_new(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(pid): Path<i64>,
) -> Result<Response, AppError> {
    admin_project(&state, pid, &admin, method.as_str(), uri.path()).await?;
    let csrf = current_csrf(&state, &jar);
    Ok(render(&empty_check_form(
        "New check",
        format!("/admin/projects/{pid}/checks"),
        true,
        csrf,
    ))?
    .into_response())
}

async fn admin_check_create(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(pid): Path<i64>,
    Form(form): Form<CheckForm>,
) -> Result<Response, AppError> {
    admin_project(&state, pid, &admin, method.as_str(), uri.path()).await?;
    let csrf = current_csrf(&state, &jar);
    check_create_core(&state, pid, form, true, true, csrf).await
}

/// The one audited admin *read*: it discloses a credential. A POST, not `?reveal=1`,
/// so the URL can't be seen without passing through here.
async fn admin_check_reveal_ping_url(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
    Query(page): Query<CheckPageQuery>,
) -> Result<Response, AppError> {
    let check = state
        .store
        .find_check(id)
        .await?
        .ok_or(AppError::NotFound)?;
    let owner = state
        .store
        .find_project(check.project_id)
        .await?
        .map(|p| p.user_id);
    // One's own check discloses nothing (the control isn't rendered for it).
    if owner != Some(admin.id) {
        state
            .store
            .record_audit(
                &crate::store::NewAudit {
                    actor_user_id: admin.id,
                    actor_username: &admin.username,
                    action: "admin.ping_url_reveal",
                    target_type: Some("check"),
                    target_id: Some(check.id),
                    target_owner_id: owner,
                    method: Some("POST"),
                    path: Some(&format!("/admin/checks/{id}/ping-url")),
                    detail: None,
                },
                Utc::now(),
            )
            .await?;
    }
    let csrf = current_csrf(&state, &jar);
    render_check_page(
        &state,
        check,
        true,
        csrf,
        None,
        page,
        CheckPageViewer::Admin {
            viewer_id: admin.id,
            ping_url_revealed: true,
        },
    )
    .await
}

async fn admin_check_show(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Query(page): Query<CheckPageQuery>,
) -> Result<Response, AppError> {
    let check = admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    let csrf = current_csrf(&state, &jar);
    let (jar, flash) = take_flash(&state.config, jar, "channels");
    let resp = render_check_page(
        &state,
        check,
        true,
        csrf,
        flash,
        page,
        CheckPageViewer::Admin {
            viewer_id: admin.id,
            ping_url_revealed: false,
        },
    )
    .await?;
    Ok((jar, resp).into_response())
}

async fn admin_check_edit(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let check = admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    let csrf = current_csrf(&state, &jar);
    Ok(render(&check_edit_form(check, true, true, csrf))?.into_response())
}

async fn admin_check_update(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Form(form): Form<CheckForm>,
) -> Result<Response, AppError> {
    admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    let csrf = current_csrf(&state, &jar);
    check_update_core(&state, id, form, true, true, csrf).await
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
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Query(confirm): Query<ConfirmQuery>,
) -> Result<Response, AppError> {
    admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    if !confirm.is_confirmed() {
        return confirmation_page(
            &state,
            &jar,
            true,
            &CONFIRM_REGENERATE_URL,
            &format!("/admin/checks/{id}/regenerate"),
            format!("/admin/checks/{id}"),
        );
    }
    state
        .store
        .regenerate_uuid(id, &uuid::Uuid::new_v4().to_string())
        .await?;
    Ok(Redirect::to(&format!("/admin/checks/{id}")).into_response())
}

async fn admin_check_delete(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Query(confirm): Query<ConfirmQuery>,
) -> Result<Response, AppError> {
    let check = admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    if !confirm.is_confirmed() {
        return confirmation_page(
            &state,
            &jar,
            true,
            &CONFIRM_DELETE_CHECK,
            &format!("/admin/checks/{id}/delete"),
            format!("/admin/checks/{id}"),
        );
    }
    state.store.delete_check(id).await?;
    Ok(Redirect::to(&format!("/admin/projects/{}", check.project_id)).into_response())
}

async fn admin_check_set_channels(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    jar: CookieJar,
    HtmlForm(form): HtmlForm<BindForm>,
) -> Result<Response, AppError> {
    let check = admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    set_channels_core(&state, &check, form, true, jar).await
}

// -- channels --
async fn admin_channel_new(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(pid): Path<i64>,
) -> Result<Response, AppError> {
    admin_project(&state, pid, &admin, method.as_str(), uri.path()).await?;
    let csrf = current_csrf(&state, &jar);
    Ok(render(&channel_form_template(
        &state, pid, true, true, csrf, None, None,
    ))?
    .into_response())
}

async fn admin_channel_edit(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let channel = admin_channel(&state, id, &admin, method.as_str(), uri.path()).await?;
    let csrf = current_csrf(&state, &jar);
    Ok(render(&channel_form_template(
        &state,
        channel.project_id,
        true,
        true,
        csrf,
        None,
        Some(ChannelEditView::new(&channel)),
    ))?
    .into_response())
}

async fn admin_channel_update(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Form(form): Form<ChannelForm>,
) -> Result<Response, AppError> {
    let channel = admin_channel(&state, id, &admin, method.as_str(), uri.path()).await?;
    let csrf = current_csrf(&state, &jar);
    channel_update_core(&state, &channel, form, true, true, csrf).await
}

async fn admin_channel_create(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(pid): Path<i64>,
    Form(form): Form<ChannelForm>,
) -> Result<Response, AppError> {
    admin_project(&state, pid, &admin, method.as_str(), uri.path()).await?;
    let csrf = current_csrf(&state, &jar);
    channel_create_core(&state, pid, form, true, true, csrf).await
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
    jar: CookieJar,
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
    let csrf = current_csrf(&state, &jar);
    render_project_page(&state.store, project, Some(result), true, true, csrf).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> crate::config::Config {
        crate::config::Config::from_map(|_| None)
    }

    fn base_check() -> Check {
        Check {
            id: 1,
            project_id: 1,
            name: "c".into(),
            description: String::new(),
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
    fn notif_event_pill_class_maps_each_event_to_the_ping_kind_palette() {
        use crate::notify::EventKind;
        assert_eq!(notif_event_pill_class(EventKind::Up), "ok");
        assert_eq!(notif_event_pill_class(EventKind::Down), "fail");
        assert_eq!(notif_event_pill_class(EventKind::Reminder), "start");
        assert_eq!(notif_event_pill_class(EventKind::Test), "log");
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

    #[test]
    fn schedule_label_uses_duration_format_and_shows_cron_grace() {
        let c = base_check();
        assert_eq!(schedule_label(&c), "every 1h · 5m grace");

        let mut c = base_check();
        c.schedule_kind = ScheduleKind::Cron;
        c.period_secs = None;
        c.cron_expr = Some("0 0 * * * *".into());
        c.grace_secs = 600;
        assert_eq!(schedule_label(&c), "0 0 * * * * · 10m grace");
    }

    fn base_check_form() -> CheckForm {
        CheckForm {
            name: "backup".into(),
            description: String::new(),
            schedule_kind: "period".into(),
            period_secs: "3600".into(),
            cron_expr: String::new(),
            grace_secs: "300".into(),
            timezone: "UTC".into(),
            scan_interval_secs: String::new(),
            max_runtime_secs: String::new(),
            nag_interval_secs: String::new(),
        }
    }

    #[test]
    fn validate_check_accepts_a_valid_period_form() {
        assert!(validate_check(&base_check_form()).is_ok());
    }

    /// Otherwise the cron would silently fire on UTC's wall clock.
    #[test]
    fn validate_check_rejects_an_unknown_timezone() {
        let mut form = base_check_form();
        form.timezone = "Asia/Taipeh".into();
        let err = validate_check(&form).unwrap_err();
        assert!(err.contains("Asia/Taipeh"), "got: {err}");
        assert!(err.contains("IANA"), "got: {err}");
    }

    #[test]
    fn validate_check_carries_the_validated_timezone() {
        let mut form = base_check_form();
        form.timezone = "  Asia/Taipei  ".into();
        assert_eq!(validate_check(&form).unwrap().timezone, "Asia/Taipei");
    }

    /// A form posted without the field must not error.
    #[test]
    fn validate_timezone_treats_blank_as_utc_and_canonicalizes() {
        assert_eq!(validate_timezone("").unwrap(), "UTC");
        assert_eq!(validate_timezone("   ").unwrap(), "UTC");
        assert_eq!(validate_timezone("Europe/Berlin").unwrap(), "Europe/Berlin");
        assert!(validate_timezone("Mars/Olympus").is_err());
    }

    /// Blank means unset here, so the check's own zone still applies.
    #[test]
    fn validate_opt_timezone_treats_blank_as_unset() {
        assert_eq!(validate_opt_timezone("").unwrap(), "");
        assert_eq!(
            validate_opt_timezone(" Europe/Berlin ").unwrap(),
            "Europe/Berlin"
        );
        assert!(validate_opt_timezone("nonsense").is_err());
    }

    #[test]
    fn validate_check_rejects_an_empty_name() {
        let mut form = base_check_form();
        form.name = String::new();
        assert_eq!(validate_check(&form).unwrap_err(), "name is required");
    }

    #[test]
    fn validate_check_rejects_a_whitespace_only_name() {
        let mut form = base_check_form();
        form.name = "   ".into();
        assert_eq!(validate_check(&form).unwrap_err(), "name is required");
    }

    #[test]
    fn validate_check_trims_the_name() {
        let mut form = base_check_form();
        form.name = "  backup  ".into();
        let v = validate_check(&form).unwrap();
        assert_eq!(v.name, "backup");
    }

    fn base_project_form() -> ProjectForm {
        ProjectForm {
            name: "proj".into(),
            description: String::new(),
            scan_interval_secs: String::new(),
            nag_interval_secs: String::new(),
        }
    }

    #[test]
    fn parse_opt_positive_blank_is_none() {
        assert_eq!(parse_opt_positive("", "x").unwrap(), None);
        assert_eq!(parse_opt_positive("   ", "x").unwrap(), None);
    }

    #[test]
    fn parse_opt_positive_accepts_positive() {
        assert_eq!(parse_opt_positive("5", "x").unwrap(), Some(5));
    }

    #[test]
    fn parse_opt_positive_rejects_zero_negative_and_non_numeric() {
        assert_eq!(
            parse_opt_positive("0", "Scan interval").unwrap_err(),
            "Scan interval must be a positive integer"
        );
        assert!(parse_opt_positive("-3", "x").is_err());
        assert!(parse_opt_positive("abc", "x").is_err());
    }

    #[test]
    fn validate_check_accepts_positive_overrides() {
        let mut form = base_check_form();
        form.scan_interval_secs = "10".into();
        form.max_runtime_secs = "20".into();
        form.nag_interval_secs = "30".into();
        let v = validate_check(&form).unwrap();
        assert_eq!(v.scan_interval_secs, Some(10));
        assert_eq!(v.max_runtime_secs, Some(20));
        assert_eq!(v.nag_interval_secs, Some(30));
    }

    #[test]
    fn validate_check_rejects_a_non_numeric_scan_interval() {
        let mut form = base_check_form();
        form.scan_interval_secs = "abc".into();
        assert_eq!(
            validate_check(&form).unwrap_err(),
            "scan interval must be a positive duration (e.g. 30, 5m, 1h30m)"
        );
    }

    #[test]
    fn validate_check_rejects_a_zero_max_runtime() {
        let mut form = base_check_form();
        form.max_runtime_secs = "0".into();
        assert_eq!(
            validate_check(&form).unwrap_err(),
            "max runtime must be a positive duration (e.g. 30, 5m, 1h30m)"
        );
    }

    #[test]
    fn validate_check_accepts_human_readable_durations() {
        let mut form = base_check_form();
        form.period_secs = "1h30m".into();
        form.grace_secs = "5m".into();
        form.scan_interval_secs = "30s".into();
        form.max_runtime_secs = "2m".into();
        form.nag_interval_secs = "1h".into();
        let v = validate_check(&form).unwrap();
        assert_eq!(v.period_secs, Some(5400));
        assert_eq!(v.grace, 300);
        assert_eq!(v.scan_interval_secs, Some(30));
        assert_eq!(v.max_runtime_secs, Some(120));
        assert_eq!(v.nag_interval_secs, Some(3600));
    }

    #[test]
    fn parse_opt_positive_duration_blank_is_none() {
        assert_eq!(parse_opt_positive_duration("", "x").unwrap(), None);
        assert_eq!(parse_opt_positive_duration("   ", "x").unwrap(), None);
    }

    #[test]
    fn parse_opt_positive_duration_accepts_human_readable() {
        assert_eq!(parse_opt_positive_duration("5m", "x").unwrap(), Some(300));
    }

    #[test]
    fn parse_opt_positive_duration_rejects_zero_negative_and_invalid() {
        for bad in ["0", "-3", "1x"] {
            assert_eq!(
                parse_opt_positive_duration(bad, "x").unwrap_err(),
                "x must be a positive duration (e.g. 30, 5m, 1h30m)"
            );
        }
    }

    #[test]
    fn validate_project_accepts_blank_and_positive() {
        assert_eq!(
            validate_project(&base_project_form()).unwrap(),
            ("proj".to_string(), String::new(), None, None)
        );
        let mut form = base_project_form();
        form.scan_interval_secs = "15".into();
        form.nag_interval_secs = "25".into();
        assert_eq!(
            validate_project(&form).unwrap(),
            ("proj".to_string(), String::new(), Some(15), Some(25))
        );
    }

    #[test]
    fn validate_project_rejects_non_numeric_and_zero() {
        let mut form = base_project_form();
        form.scan_interval_secs = "abc".into();
        assert!(validate_project(&form).is_err());
        let mut form = base_project_form();
        form.nag_interval_secs = "0".into();
        assert!(validate_project(&form).is_err());
    }

    #[test]
    fn validate_project_accepts_human_readable_durations() {
        let mut form = base_project_form();
        form.scan_interval_secs = "5m".into();
        form.nag_interval_secs = "1h".into();
        assert_eq!(
            validate_project(&form).unwrap(),
            ("proj".to_string(), String::new(), Some(300), Some(3600))
        );
    }

    #[test]
    fn validate_project_rejects_an_empty_name() {
        let mut form = base_project_form();
        form.name = String::new();
        assert_eq!(validate_project(&form).unwrap_err(), "name is required");
    }

    #[test]
    fn validate_project_rejects_a_whitespace_only_name() {
        let mut form = base_project_form();
        form.name = "   ".into();
        assert_eq!(validate_project(&form).unwrap_err(), "name is required");
    }

    #[test]
    fn validate_project_trims_the_name() {
        let mut form = base_project_form();
        form.name = "  Nightly jobs  ".into();
        let (name, _, _, _) = validate_project(&form).unwrap();
        assert_eq!(name, "Nightly jobs");
    }

    #[test]
    fn readable_setting_duration_formats_seconds_and_passes_through_the_rest() {
        assert_eq!(readable_setting_duration("3600".into()), "1h");
        assert_eq!(readable_setting_duration("45".into()), "45s");
        // Blank and non-positive values pass through untouched.
        assert_eq!(readable_setting_duration(String::new()), "");
        assert_eq!(readable_setting_duration("0".into()), "0");
        assert_eq!(readable_setting_duration("abc".into()), "abc");
    }

    /// Built through the production builder, so name and signature are real.
    fn flash_jar(config: &crate::config::Config, value: &str) -> CookieJar {
        CookieJar::new().add(flash_cookie_value(config, value.to_string()))
    }

    #[test]
    fn take_flash_maps_each_surface_to_its_own_message() {
        let config = test_config();
        let jar = flash_jar(&config, "settings");
        let (_, msg) = take_flash(&config, jar, "settings");
        assert_eq!(msg.as_deref(), Some("Settings saved."));

        let jar = flash_jar(&config, "channels");
        let (_, msg) = take_flash(&config, jar, "channels");
        assert_eq!(msg.as_deref(), Some("Notify channels saved."));

        let jar = flash_jar(&config, "users_blocked");
        let (_, msg) = take_flash(&config, jar, "users_blocked");
        assert_eq!(
            msg.as_deref(),
            Some(
                "That action was refused: you cannot remove your own access, and the last enabled admin cannot be removed."
            )
        );

        let jar = flash_jar(&config, "forward_auth_logout");
        let (_, msg) = take_flash(&config, jar, "forward_auth_logout");
        assert_eq!(
            msg.as_deref(),
            Some(
                "Signed out locally, but you're authenticated through your reverse proxy — this app can't end that session. To sign out completely, log out at your proxy or SSO provider."
            )
        );
    }

    #[test]
    fn take_flash_ignores_a_flash_set_for_another_surface() {
        // Path "/" means every page sees it; only its own page may consume it.
        let config = test_config();
        let jar = flash_jar(&config, "channels");
        let (jar, msg) = take_flash(&config, jar, "settings");
        assert_eq!(msg, None);
        let (_, msg) = take_flash(&config, jar, "channels");
        assert_eq!(msg.as_deref(), Some("Notify channels saved."));
    }

    #[test]
    fn take_flash_without_a_cookie_is_none() {
        let (_, msg) = take_flash(&test_config(), CookieJar::new(), "settings");
        assert_eq!(msg, None);
    }

    #[test]
    fn take_flash_never_renders_an_unknown_cookie_value() {
        // Even with a matching surface, an unknown key maps to no message.
        let config = test_config();
        let jar = flash_jar(&config, "<script>");
        let (_, msg) = take_flash(&config, jar, "<script>");
        assert_eq!(msg, None);
    }

    /// An unsigned flash (a sibling subdomain over plain HTTP) is ignored by both readers.
    #[test]
    fn an_unsigned_flash_cookie_is_ignored() {
        let config = test_config();
        let name = flash_cookie_name(&config);

        let jar = CookieJar::new().add(Cookie::new(name, "settings"));
        let (_, msg) = take_flash(&config, jar, "settings");
        assert_eq!(msg, None);

        let jar = CookieJar::new().add(Cookie::new(name, "password_reset_keys:1:99"));
        let (_, msg) = take_password_reset_keys_flash(&config, jar);
        assert_eq!(msg, None);

        // Signed under a different secret: same rejection.
        let other = crate::config::Config::from_map(|k| {
            (k == "PINGWARD_SECRET").then(|| "another-test-secret-32-bytes-x".into())
        });
        let jar = CookieJar::new().add(flash_cookie_value(&other, "settings".into()));
        let (_, msg) = take_flash(&config, jar, "settings");
        assert_eq!(msg, None);
    }

    /// `__Host-` requires `Secure`, `Path=/` and no `Domain`.
    #[test]
    fn a_secure_flash_cookie_is_host_prefixed_and_prefix_legal() {
        let secure = crate::config::Config::from_map(|k| {
            (k == "PINGWARD_COOKIE_SECURE").then(|| "true".into())
        });
        let cookie = flash_cookie_value(&secure, "settings".into());
        assert_eq!(cookie.name(), FLASH_COOKIE_HOST_PREFIXED);
        assert_eq!(cookie.secure(), Some(true));
        assert_eq!(cookie.path(), Some("/"));
        assert_eq!(cookie.domain(), None);

        // Unprefixed when not Secure, or the browser rejects it.
        let plain = test_config();
        assert!(!plain.cookie_secure);
        assert_eq!(
            flash_cookie_value(&plain, "settings".into()).name(),
            FLASH_COOKIE_BASE
        );
    }

    #[test]
    fn redact_db_url_leaves_a_plain_sqlite_url_unchanged() {
        let url = "sqlite://pingward.sqlite3?mode=rwc";
        assert_eq!(redact_db_url(url), url);
    }

    #[test]
    fn redact_db_url_leaves_a_bare_path_unchanged() {
        let url = "pingward.sqlite3";
        assert_eq!(redact_db_url(url), url);
    }

    #[test]
    fn redact_db_url_strips_user_and_password() {
        let url = "postgres://user:pass@host/db";
        let out = redact_db_url(url);
        assert_eq!(out, "postgres://***@host/db");
        assert!(!out.contains("pass"));
        assert!(!out.contains("user"));
    }

    #[test]
    fn redact_db_url_strips_bare_username_with_no_password() {
        let url = "postgres://user@host/db";
        let out = redact_db_url(url);
        assert_eq!(out, "postgres://***@host/db");
        assert!(!out.contains("user@"));
    }

    #[test]
    fn redact_db_url_ignores_an_at_sign_only_in_the_query_string() {
        let url = "postgres://host/db?target_session_attrs=x&opt=a@b";
        assert_eq!(redact_db_url(url), url);
    }

    #[test]
    fn redact_db_url_empty_string_is_unchanged() {
        assert_eq!(redact_db_url(""), "");
    }
}
