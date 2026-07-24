use crate::auth::{
    AdminUser, CurrentUser, OptionalUser, SESSION_COOKIE, SESSION_TTL_DAYS, hash_password,
    new_session_token, verify_password,
};
use crate::error::AppError;
use crate::models::{
    Channel, ChannelKind, Check, CheckStatus, Notification, Project, ScheduleKind, User,
};
use crate::notify::{EventKind, NotificationEvent, notifier_for};
use crate::secret;
use crate::state::AppState;
use crate::store::{NotifFilter, PageCursor, PingFilter, Store};
use askama::Template;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
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
        .route("/channels/{id}/delete", post(channel_delete))
        .route("/channels/{id}/test", post(channel_test))
        .route("/checks/{id}/channels", post(check_set_channels))
        .route("/account", get(account_page))
        .route("/account/api-keys", post(api_keys_create))
        .route("/account/api-keys/{id}/delete", post(api_keys_delete))
        .route("/account/sessions/{handle}/revoke", post(sessions_revoke))
        .route(
            "/account/sessions/revoke-others",
            post(sessions_revoke_others),
        )
        // --- admin cross-user route group (every handler guarded by
        // AdminUser, no exceptions) ---
        .route("/admin", get(admin_page))
        .route("/admin/settings", post(settings_save))
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
    /// Checks whose display status is `Up` **or** `Running` — an in-flight run
    /// is still up, so the two share one tile rather than splitting the count.
    up: usize,
    late: usize,
    down: usize,
    groups: Vec<ProjectGroup>,
    /// The active filter term, trimmed, echoed back into the search box. Empty
    /// means unfiltered — the template keys both the empty state and the
    /// "clear" affordance off this rather than a separate flag.
    q: String,
    /// The active status filter as its `?status=` value (`up`/`late`/`down`),
    /// or empty for "all". Drives the `<select>`'s selected option and, with
    /// `q`, whether the "clear" affordance and no-results state show.
    status: String,
}

/// Query params for the dashboard filter. Absent, blank, or whitespace-only
/// means "no filter", so `/?q=` behaves exactly like `/`.
#[derive(Deserialize, Default)]
struct DashboardQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// The dashboard's status filter. Deliberately narrower than [`DisplayStatus`]:
/// it mirrors the summary tiles (`up`/`late`/`down`), and `Up` folds in
/// `Running` the same way the Up tile does. `Paused`/`New` checks have no tile
/// and no filter entry — they show only in the unfiltered list.
#[derive(Clone, Copy)]
enum StatusFilter {
    Up,
    Late,
    Down,
}

impl StatusFilter {
    /// Parse the `?status=` value. Anything outside `up`/`late`/`down` —
    /// including `all`, empty, or garbage — is "no filter" (`None`), so a bad
    /// value degrades to the full list rather than a 400 or an empty page.
    fn parse(s: &str) -> Option<Self> {
        match s {
            "up" => Some(Self::Up),
            "late" => Some(Self::Late),
            "down" => Some(Self::Down),
            _ => None,
        }
    }

    /// The canonical `?status=` value, echoed back so the `<select>` re-selects
    /// the active option.
    fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Late => "late",
            Self::Down => "down",
        }
    }

    /// Does a check's display status fall in this bucket? `Up` matches an
    /// in-flight `Running` check too, matching the merged Up tile.
    fn matches(self, ds: crate::view::DisplayStatus) -> bool {
        use crate::view::DisplayStatus;
        match self {
            Self::Up => matches!(ds, DisplayStatus::Up | DisplayStatus::Running),
            Self::Late => matches!(ds, DisplayStatus::Late),
            Self::Down => matches!(ds, DisplayStatus::Down),
        }
    }
}

/// Case-insensitive substring test backing the dashboard filter.
///
/// `needle` must already be lowercased by the caller — it is the same for every
/// row, so lowercasing it once per request rather than once per field keeps the
/// scan linear in the data actually being searched.
///
/// Matching runs in Rust over the rows the dashboard already loads, not in SQL:
/// `LIKE` is ASCII-case-insensitive on `SQLite` but case-sensitive on Postgres,
/// and `ILIKE` is Postgres-only and untranslated by the `Any` driver, so a
/// portable SQL version would need two dialects for no gain.
fn matches_term(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(needle)
}

/// A check's most recent activity: the later of its last finished ping and its
/// last start. `Option`'s ordering does the work — `Some(_) > None`, and
/// `max` between two `Some`s picks the later instant — which is the same trick
/// `view::display_status` uses to spot an in-flight run. A check that has never
/// been pinged yields `None`.
fn last_activity_at(c: &Check) -> Option<DateTime<Utc>> {
    c.last_ping_at.max(c.last_start_at)
}

/// Order a project's checks for display: most recent activity first, so a job
/// that just ran (or just started) surfaces at the top. Never-pinged checks
/// sort last (`None` is the smallest key, reversed here), and checks sharing a
/// timestamp fall back to creation order so the list is deterministic.
fn sort_checks_by_activity(checks: &mut [Check]) {
    checks.sort_by(|a, b| {
        last_activity_at(b)
            .cmp(&last_activity_at(a))
            .then(a.id.cmp(&b.id))
    });
}

/// Order the dashboard's project groups by name, case-insensitively so `Web`
/// and `api` interleave the way a reader expects rather than splitting on byte
/// value. Equal names fall back to creation order for a deterministic list.
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
    status: &'static str, // view::DisplayStatus::as_str()
    schedule: String,     // e.g. "every 1h · 10m grace" or the cron expr
    last: String,         // fmt_relative or "—"
    bars: Vec<crate::view::Bar>,
    description: String, // markdown::truncate_plain, single-line summary
    /// True when the check has zero bound notification channels — rendered as
    /// a "no channel" chip so a check nobody would be alerted for is visible
    /// at a glance rather than silent.
    no_channel: bool,
}

struct ProjectGroup {
    id: i64,
    name: String,
    count: usize,
    checks: Vec<CheckRow>,
    description: String, // markdown::truncate_plain, single-line summary
}

/// Human-readable schedule summary shown under a check's name (dashboard rows,
/// the project page, and the check detail page). Uses `duration::fmt_duration`
/// so the displayed interval matches what the check form accepts and renders.
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
    if creds.username.is_empty() || creds.password.is_empty() {
        return Ok(render(&SetupTemplate {
            show_nav: false,
            csrf: current_csrf(&state, &jar),
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
    let ua = request_user_agent(&headers);
    let jar = start_session(&state, jar, uid, ua.as_deref(), conn.0.as_deref(), false).await?;
    Ok((jar, Redirect::to("/")).into_response())
}

/// Renders the login form — or bounces an already-signed-in visitor to the
/// dashboard.
///
/// That bounce is load-bearing under forward auth. `logout` clears the session
/// and lands here, but `forward_auth_session` runs first and, seeing the
/// gateway's identity header still present, immediately mints a fresh one. The
/// visitor would otherwise be shown a login form while already signed in as the
/// very account they just tried to leave. Sending them to `/` is the honest
/// outcome: only the gateway can end that identity (see
/// `PINGWARD_FORWARD_AUTH_LOGOUT_URL`).
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

async fn login_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    conn: crate::ping::ClientIp,
    Form(creds): Form<Credentials>,
) -> Result<Response, AppError> {
    let user = state.store.find_user_by_username(&creds.username).await?;
    let ok = user
        .as_ref()
        .and_then(|u| u.password_hash.as_deref())
        .is_some_and(|phc| verify_password(&creds.password, phc));
    if !ok {
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
    Ok((jar, Redirect::to("/")).into_response())
}

/// Ends the local session, then sends the browser to
/// `PINGWARD_FORWARD_AUTH_LOGOUT_URL` when one is configured, else `/login`.
///
/// Deleting the row is never enough behind an authentication gateway: the next
/// request still carries the gateway's identity header, so
/// `forward_auth_session` signs the visitor straight back in. Only the gateway
/// can end that identity, which is what the configured URL is for — it points
/// at the gateway's own sign-out endpoint. Left unset, logout is still
/// well-defined, it just does not outlive the redirect while the gateway
/// session stands.
///
/// The redirect target comes from the operator's environment, never from the
/// request, so it is not an open redirect.
async fn logout(State(state): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    if let Some(id) = secret::session_id_from_jar(&jar, &state.config.secret) {
        state.store.delete_session(&id).await?;
    }
    let jar = jar.remove(Cookie::from(SESSION_COOKIE));
    let target = state
        .config
        .forward_auth_logout_url
        .as_deref()
        .unwrap_or("/login");
    Ok((jar, Redirect::to(target)).into_response())
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
    let now = Utc::now();
    let q = query.q.unwrap_or_default().trim().to_string();
    let needle = q.to_lowercase();
    let status_raw = query.status.unwrap_or_default();
    let status_filter = StatusFilter::parse(&status_raw);
    // Echo back only a recognised value, so a garbage `?status=` neither
    // pre-selects a bogus option nor lights up the "clear" affordance.
    let status = status_filter.map_or("", StatusFilter::as_str).to_string();
    let (mut total, mut up, mut late, mut down) = (0usize, 0, 0, 0);
    let mut groups = Vec::new();
    // Gather every project's checks first, then fetch all their recent pings in
    // one batched query (avoids an N+1 of one `list_recent_pings` per check).
    // Filtering happens here, before the ping fetch, so a narrow filter also
    // narrows the batched query instead of loading pings for hidden rows.
    let mut project_checks = Vec::new();
    let mut check_ids = Vec::new();
    // Display order is decided here rather than in the `Store` queries: those
    // are shared with the project page, the admin views and the API, which all
    // want the stable id order.
    let mut projects = state.store.list_projects_for_user(user.id).await?;
    sort_projects_by_name(&mut projects);
    // Batched like the ping fetch below: one query for every project's checks
    // instead of one per rendered group.
    let project_ids: Vec<i64> = projects.iter().map(|p| p.id).collect();
    let mut checks_by_project = state.store.list_checks_for_projects(&project_ids).await?;
    for project in projects {
        let mut checks = checks_by_project.remove(&project.id).unwrap_or_default();
        sort_checks_by_activity(&mut checks);
        let checks = if needle.is_empty()
            || matches_term(&project.name, &needle)
            || matches_term(&project.description, &needle)
        {
            // A project-level hit shows the project whole, including checks that
            // do not match themselves — otherwise searching a project's own name
            // would render a header above an empty list.
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
        .list_recent_pings_for_checks(&check_ids, 40)
        .await?;
    let with_channels = state.store.checks_with_channels(&check_ids).await?;
    for (project, checks) in project_checks {
        let mut rows = Vec::with_capacity(checks.len());
        for c in &checks {
            let ds = crate::view::display_status(c, now);
            // Tiles count the whole `q`-filtered set, independent of the status
            // selection — otherwise picking "Down" would zero the other tiles
            // and there would be nothing left to switch back to. An in-flight
            // `Running` check counts as up (one merged tile).
            total += 1;
            match ds {
                crate::view::DisplayStatus::Up | crate::view::DisplayStatus::Running => up += 1,
                crate::view::DisplayStatus::Late => late += 1,
                crate::view::DisplayStatus::Down => down += 1,
                _ => {}
            }
            // The status filter narrows the rendered list only, after counting.
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
        // Under a status filter, a project whose checks are all filtered out is
        // dropped entirely rather than rendering a header above an empty list.
        // Its checks still counted toward the tiles above.
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
    Ok(render(&DashboardTemplate {
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
    })?
    .into_response())
}

/// Column-bounding cap for a stored `user_agent` (raw browser headers can be
/// arbitrarily long; the value is display-only, so it is simply truncated).
const MAX_USER_AGENT_CHARS: usize = 300;

/// Extract the `User-Agent` request header as a bounded, valid-UTF-8 string
/// for storage alongside a session row.
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

/// Create a session row and return the signed cookie that addresses it.
async fn open_session(
    state: &AppState,
    user_id: i64,
    user_agent: Option<&str>,
    ip: Option<&str>,
    sso: bool,
) -> Result<Cookie<'static>, AppError> {
    let session_id = new_session_token();
    let expires = Utc::now() + Duration::days(SESSION_TTL_DAYS);
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
    // The cookie carries `<id>.<hmac>`, never the bare id — see `crate::secret`.
    Ok(Cookie::build((
        SESSION_COOKIE,
        secret::sign_session(&state.config.secret, &session_id),
    ))
    .http_only(true)
    .same_site(SameSite::Lax)
    .path("/")
    .build())
}

/// Create a session row and return a jar carrying the signed session cookie.
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

/// Give every visitor a signed session cookie, logged in or not.
///
/// The CSRF token is `HMAC(secret, "csrf:" + session id)` — it needs an id, not
/// a database row. So an anonymous visitor can carry a perfectly good token
/// with no `sessions` insert at all: this mints a random id, signs it, and
/// writes nothing. Only a real login (or forward-auth) creates the row that
/// turns that id into an authenticated session.
///
/// Two things fall out of that. Pages rendered before login — `/login`,
/// `/setup` — can carry a real token, so [`csrf_guard`] needs no path
/// exemptions and login itself is CSRF-protected. And `resolve_user` needs no
/// change: an anonymous id simply matches no row, so the visitor stays
/// anonymous.
///
/// Layered *inside* [`forward_auth_session`] (see `crate::app`) so that when
/// both would mint, the forward-auth one wins — otherwise the outer layer's
/// `Set-Cookie` would be appended last and shadow the real session with an
/// anonymous id.
pub async fn anonymous_session(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let jar = CookieJar::from_headers(req.headers());
    if secret::session_id_from_jar(&jar, &state.config.secret).is_some() {
        return next.run(req).await;
    }
    // Signature only — an anonymous id has no row to look up, and a stale
    // signed cookie is left alone so its owner keeps one stable token.
    let cookie = Cookie::build((
        SESSION_COOKIE,
        secret::sign_session(&state.config.secret, &new_session_token()),
    ))
    .http_only(true)
    .same_site(SameSite::Lax)
    .path("/")
    .build();
    replace_request_cookie(&mut req, &cookie);
    let mut resp = next.run(req).await;
    if let Ok(value) = cookie.to_string().parse() {
        resp.headers_mut().append(header::SET_COOKIE, value);
    }
    resp
}

/// Give a trusted forward-auth identity a real session, so the rest of the
/// browser surface needs no special case for it.
///
/// Without this, a forward-auth user is authenticated (`resolve_user` falls
/// back to the header) but session-less, and everything keyed off the session
/// silently degrades: forms render an empty `_csrf`, [`csrf_guard`] rejects
/// every POST with 403, and the account page lists no sessions to review or
/// revoke. Minting the session here means only this function knows that
/// forward-auth is different.
///
/// Layered *outside* [`anonymous_session`] and [`csrf_guard`] (see
/// `crate::app`) so the guard sees the cookie on the same request that created
/// it, and the newly signed cookie is injected into the request as well as set
/// on the response — a handler rendering a form in this very request must be
/// able to derive the matching token. Running first also means
/// [`anonymous_session`] finds a cookie already in place and stays out of the
/// way, so only one `Set-Cookie` is ever emitted.
///
/// Requests that already carry a live session, and every deployment that has
/// not configured `PINGWARD_FORWARD_AUTH_HEADER`, short-circuit before any
/// database work. Note the liveness check is deliberate rather than a bare
/// signature check: with [`anonymous_session`] in play, a valid signature no
/// longer implies a session row exists.
pub async fn forward_auth_session(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    if state.config.forward_auth_header.is_none() {
        return next.run(req).await;
    }
    let now = Utc::now();
    // A cookie whose signature verifies *and* still addresses a live session
    // needs nothing; a stale or forged one falls through and is replaced.
    let jar = CookieJar::from_headers(req.headers());
    if let Some(id) = secret::session_id_from_jar(&jar, &state.config.secret)
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

/// Rewrite the request's `Cookie` header so downstream extractors see `cookie`
/// instead of whatever value it had for that name.
///
/// Dropping the stale entry matters: `CookieJar::get` returns the first match,
/// so appending would leave an expired session id shadowing the fresh one.
fn replace_request_cookie(req: &mut Request, cookie: &Cookie<'static>) {
    let kept: Vec<String> = req
        .headers()
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .filter(|pair| !pair.is_empty() && !pair.starts_with(&format!("{SESSION_COOKIE}=")))
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

/// Resolve the current session's CSRF synchronizer token from the request
/// cookies, for embedding as a hidden `_csrf` field in rendered POST forms.
/// Returns an empty string when the request carries no valid session (e.g. the
/// pre-session `login`/`setup` pages, which carry exempt forms) — an empty
/// token yields an unsubmittable form rather than a token-less bypass.
fn current_csrf(state: &AppState, jar: &CookieJar) -> String {
    secret::session_id_from_jar(jar, &state.config.secret)
        .map(|id| secret::derive_csrf(&state.config.secret, &id))
        .unwrap_or_default()
}

/// CSRF synchronizer-token guard, applied to `web::routes()` only (the machine
/// `/ping/*` endpoints, assets, and `/healthz` live in sibling routers and are
/// therefore structurally exempt).
///
/// Safe methods (GET/HEAD/OPTIONS) pass through untouched. Every other
/// state-changing request — `POST /login` and `POST /setup` included, since
/// [`anonymous_session`] gives even a logged-out visitor a token to embed —
/// must present the session's token, taken from the `X-CSRF-Token`
/// header or, failing that, the `_csrf` urlencoded form field (in which case
/// the body is buffered and the request rebuilt so the downstream `Form<T>`
/// extractor still works). The token is derived from the session id rather than
/// stored, so this costs no database round trip; comparison is constant-time
/// (`secret::verify_csrf`) because the token is now a MAC over a known input.
///
/// Upper bound on the buffered request body when reading the `_csrf` form field.
/// Browser POSTs to `web::routes()` carry small urlencoded forms; 1 MiB is a
/// generous ceiling that caps memory a malicious client could force us to buffer.
const CSRF_MAX_BODY_BYTES: usize = 1 << 20;

pub async fn csrf_guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // Safe methods never change state.
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    // Resolve the caller's session id from the signed cookie. An unsigned or
    // tampered cookie never gets this far, so no token can match it.
    let jar = CookieJar::from_headers(req.headers());
    let secret = &state.config.secret;
    let Some(session_id) = secret::session_id_from_jar(&jar, secret) else {
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
        return StatusCode::FORBIDDEN.into_response();
    }
    // Otherwise read the `_csrf` form field: buffer the body, extract the token,
    // then rebuild the request with the same bytes for the downstream handler.
    let (parts, body) = req.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, CSRF_MAX_BODY_BYTES).await else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let submitted = form_urlencoded::parse(&bytes)
        .find(|(k, _)| k == "_csrf")
        .map(|(_, v)| v.into_owned());
    if !submitted.is_some_and(|t| secret::verify_csrf(secret, &session_id, &t)) {
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
    channels: Vec<Channel>,
    test_result: Option<TestResult>,
}

/// Project page's per-check row: same idea as the dashboard's `CheckRow`
/// (precompute `display_status` in the handler, since it needs `now` and the
/// template has no clock), trimmed to the fields `project.html` renders.
struct ProjectCheckRow {
    id: i64,
    name: String,
    status: &'static str, // view::DisplayStatus::as_str()
    schedule: String,
    description: String, // markdown::truncate_plain, single-line summary
    /// True when the check has zero bound notification channels — rendered as
    /// a "no channel" chip so a check nobody would be alerted for is visible
    /// at a glance rather than silent.
    no_channel: bool,
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

/// Maximum length, in **characters** (not bytes), of a project/check
/// description. Rendered through `markdown::render`, so this bounds the work
/// that does on every page view, not just storage size. `markdown::render` is
/// worst-case O(n²) (see its module doc); do not raise this without reading
/// that note.
const MAX_DESCRIPTION_CHARS: usize = 2000;

/// Trim a description form field and enforce [`MAX_DESCRIPTION_CHARS`],
/// counting characters rather than bytes so multi-byte input isn't penalized.
fn validate_description(s: &str) -> Result<String, String> {
    let trimmed = s.trim();
    if trimmed.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(format!(
            "description must be at most {MAX_DESCRIPTION_CHARS} characters"
        ));
    }
    Ok(trimmed.to_string())
}

/// Parse an optional positive-integer form field. Blank/whitespace-only input
/// is `Ok(None)` (the field is intentionally unset — inherit the default, or
/// off). A non-blank value MUST parse to an integer strictly greater than zero;
/// anything else is `Err(msg)` naming the field, so the caller can re-render
/// the form instead of discarding what the user typed.
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

/// Parse an optional positive *duration* form field (raw seconds or a
/// human-readable string like `5m` / `1h30m`). Blank/whitespace-only is
/// `Ok(None)` (unset — inherit the default, or off); a non-blank value must
/// parse and be strictly greater than zero, else `Err(msg)` naming the field so
/// the caller can re-render the form instead of discarding what the user typed.
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

/// Validate a project form's name, description, and optional duration
/// override fields, returning the parsed
/// `(name, description, scan_interval_secs, nag_interval_secs)` or an error
/// message. The name and description are returned trimmed — they are what
/// must be stored.
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

/// Rebuild a project form after a validation error, preserving the submitted
/// values so the user can fix the invalid field.
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

/// `/admin` when acting as an admin, otherwise the empty (owner) prefix. Used
/// to point rendered links, form actions, and redirects at the right route.
fn admin_prefix(admin: bool) -> &'static str {
    if admin { "/admin" } else { "" }
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
    let channels = store.list_channels_for_project(project.id).await?;
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

/// Build the project edit form, pointing its action at the owner or `/admin`
/// route depending on `admin`. `is_admin` reflects the current viewer's admin
/// status and controls the nav Admin link.
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
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    owned_project(&state.store, id, user.id).await?;
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
    created_at: String,             // UTC fallback shown when JS is off
    iso: String,                    // RFC3339 UTC; localized to the viewer's zone client-side
    event: &'static str,            // visible event label: "down"|"up"|"reminder"
    event_pill_class: &'static str, // pill css class, mirroring the ping-kind pills
    status: &'static str,
    channel: String,
    error: String,
}

/// Maps a notification `EventKind` to a pill CSS class, reusing the same
/// palette as the ping-kind pills (`ping_pill_class`): a recovery is "ok"
/// (green), a downtime alert is "fail" (red), a reminder is neutral, and a
/// test uses the brand "log" tone. Test deliveries aren't recorded in the
/// history table, but the match stays exhaustive.
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
    schedule: String,
    ping_url: String,
    bars: Vec<crate::view::Bar>,
    channel_boxes: Vec<ChannelBox>,
    /// The "recent pings" card body — filter controls, table, pager — rendered
    /// from [`CheckPingsTemplate`] so the same fragment is emitted on full-page
    /// load and on a JS partial refresh. Injected with `|safe`.
    pings_partial: String,
    /// The "recent notifications" card body, from [`CheckNotifsTemplate`].
    notifs_partial: String,
    flash: Option<String>,
}

/// The "recent pings" fragment: filter controls + table + keyset pager. Served
/// standalone by `GET /checks/{id}/pings` (JS swaps it into `#pings-section`)
/// and inlined into the full check page. `base` is `""` or `/admin`.
#[derive(Template)]
#[template(path = "check_pings.html")]
struct CheckPingsTemplate {
    base: String,
    check_id: i64,
    rows: Vec<PingRow>,
    empty: bool,
    /// Selected kind filter (`""` = all), canonicalized from the query.
    f_kind: String,
    /// Selected date bounds as `Z`-form RFC3339 UTC (`""` = unset); the input is
    /// `datetime-local`, localized client-side from these `data-utc` values.
    f_from: String,
    f_to: String,
    /// Any filter active — controls the "Clear" affordance.
    filtered: bool,
    newer: Option<String>,
    older: Option<String>,
}

/// The "recent notifications" fragment, served by
/// `GET /checks/{id}/notifications`. Filters on event and delivery result.
#[derive(Template)]
#[template(path = "check_notifs.html")]
struct CheckNotifsTemplate {
    base: String,
    check_id: i64,
    rows: Vec<NotificationRow>,
    empty: bool,
    /// Selected event filter (`""` = all): up|down|reminder.
    f_event: String,
    /// Selected delivery-result filter (`""` = all): ok|error.
    f_status: String,
    f_from: String,
    f_to: String,
    filtered: bool,
    newer: Option<String>,
    older: Option<String>,
}

/// Query params for the check-detail ping/notification history fragments. Each
/// table pages and filters independently: `p*` params drive the pings fragment,
/// `n*` the notifications fragment. Cursors are `pb`/`pa` (pings older/newer)
/// and `nb`/`na`; filters are `pk` (ping kind), `ne`/`ns` (notify event/result),
/// and `pfrom`/`pto`/`nfrom`/`nto` (RFC3339 UTC date bounds). Missing/unparsable
/// params fall back to their unset default via `#[serde(default)]` (the
/// "Latest", unfiltered view) rather than a 400. The full check page and both
/// partial endpoints share this struct.
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

/// Parse a single-select enum filter param (`""`/unset/garbage → empty vec, one
/// valid token → a one-element vec), matching the `Vec` shape the store filters
/// accept while the UI only ever offers a single choice.
fn parse_filter_enum<T: FromStr>(v: Option<&str>) -> Vec<T> {
    v.map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<T>().ok())
        .into_iter()
        .collect()
}

/// Parse a date-bound filter param into a UTC instant. Accepts full RFC3339
/// (what the JS sends after localizing the `datetime-local` control) and the
/// bare `YYYY-MM-DDTHH:MM[:SS]` a JS-off submit would produce, treated as UTC.
/// Anything unparsable is dropped to `None` rather than erroring the request.
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

/// Canonical `Z`-form RFC3339 for echoing a parsed date bound back into a
/// fragment's `data-utc` attribute and pager hrefs (`+00:00` would need
/// percent-encoding; `Z` is query-safe).
fn date_bound_token(dt: Option<DateTime<Utc>>) -> String {
    dt.map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// Build a history-fragment href (`{base}/checks/{id}/{seg}?…`) for a keyset
/// pager link. `cursor` is this table's new position; `carry` re-attaches the
/// currently-active filter tokens so paging preserves the filter. Values are
/// ids, enum tokens, or `Z`-form datetimes — all query-safe, so no encoding.
fn history_href(
    base: &str,
    id: i64,
    seg: &str,
    cursor: (&str, i64),
    carry: &[(&str, &str)],
) -> String {
    use std::fmt::Write as _;
    let mut href = format!("{base}/checks/{id}/{seg}?{}={}", cursor.0, cursor.1);
    for (k, v) in carry {
        if !v.is_empty() {
            let _ = write!(href, "&{k}={v}");
        }
    }
    href
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

/// Load a check and enforce ownership through its project.
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
    pub(crate) scan_interval_secs: Option<i64>,
    pub(crate) max_runtime_secs: Option<i64>,
    pub(crate) nag_interval_secs: Option<i64>,
}

/// Validate a check form into a `ValidatedCheck` (schedule + grace + the three
/// optional duration overrides). Returns `Err(message)` on invalid input; a
/// non-blank override that isn't a positive duration is rejected rather than
/// silently discarded.
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
            timezone: &form.timezone,
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

/// Name of the one-shot flash cookie set after a redirect (e.g. saving a
/// check's notify channels) and cleared on the next render.
const FLASH_COOKIE: &str = "pingward_flash";

/// Read and clear the one-shot flash cookie **if** it was set for `surface`,
/// mapping it to that surface's fixed message. The cookie is path-scoped to
/// `/`, so every page sees it — a flash set for another surface is therefore
/// left in the jar for that page to consume rather than rendered here, which
/// keeps a message from surfacing on the wrong page when a redirect is not
/// followed or two tabs race. Only known keys map to a message, so a
/// user-supplied cookie value never renders as arbitrary text.
fn take_flash(jar: CookieJar, surface: &str) -> (CookieJar, Option<String>) {
    let Some(cookie) = jar.get(FLASH_COOKIE) else {
        return (jar, None);
    };
    if cookie.value() != surface {
        return (jar, None);
    }
    let message = match surface {
        "channels" => "Notify channels saved.",
        "settings" => "Settings saved.",
        "users_blocked" => {
            "That action was refused: you cannot remove your own access, and the last enabled admin cannot be removed."
        }
        _ => return (jar, None),
    };
    (
        jar.remove(Cookie::build((FLASH_COOKIE, "")).path("/").build()),
        Some(message.to_string()),
    )
}

/// Set the `users_blocked` flash cookie and redirect to `/admin`. Used by the
/// self-guard and last-enabled-admin-guard branches in `users_delete`,
/// `users_toggle_admin` and `users_set_disabled` — mirrors how
/// `settings_save` sets `FLASH_COOKIE` for its own surface (~line 2413).
fn users_blocked(jar: CookieJar) -> Response {
    let jar = jar.add(
        Cookie::build((FLASH_COOKIE, "users_blocked"))
            .http_only(true)
            .same_site(SameSite::Lax)
            .path("/")
            .build(),
    );
    (jar, Redirect::to("/admin")).into_response()
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
    let (jar, flash) = take_flash(jar, "channels");
    let resp = render_check_page(&state, check, false, user.is_admin, csrf, flash, page).await?;
    Ok((jar, resp).into_response())
}

/// Render the check detail page. `admin` renders `/admin`-prefixed action URLs;
/// `is_admin` reflects the current viewer's admin status and controls the nav
/// Admin link. `page` carries the independent ping/notification keyset
/// cursors read from the request's query string.
async fn render_check_page(
    state: &AppState,
    check: Check,
    admin: bool,
    is_admin: bool,
    csrf: String,
    flash: Option<String>,
    page: CheckPageQuery,
) -> Result<Response, AppError> {
    let id = check.id;
    let base = admin_prefix(admin);
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
    // The heartbeat/bars strip always shows the latest 40 pings, independent
    // of the table's paging below — a paged (older) result must never feed it.
    let recent = state.store.list_recent_pings(id, 40).await?;
    let bars = crate::view::heartbeat(
        &recent,
        check.max_runtime_secs,
        check.status == CheckStatus::Paused,
        30,
    );

    let status = crate::view::display_status(&check, now).as_str();
    let since = status_since_label(&check, now);
    let schedule = schedule_label(&check);
    let description_html = crate::markdown::render(&check.description);

    // Both history tables render from the same fragment templates the JS
    // partial endpoints serve, then get injected here — one source of truth for
    // the markup. The pings fragment reuses the 40-row heartbeat window for
    // duration pairing on the default (unfiltered latest) view; the notif
    // fragment reuses the channel-name map already built above.
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
        schedule,
        ping_url,
        bars,
        channel_boxes,
        pings_partial,
        notifs_partial,
        flash,
    })?
    .into_response())
}

/// Build the "recent pings" fragment for `check_id`, honoring the `p*` filter
/// and cursor params in `page`. `recent`, when supplied by the full-page render,
/// is the 40-row heartbeat window reused for duration pairing on the default
/// (unfiltered latest) view; the standalone partial endpoint passes `None` and
/// the window is fetched only when that view is active.
async fn build_pings_partial(
    state: &AppState,
    check_id: i64,
    base: &str,
    page: &CheckPageQuery,
    recent: Option<&[crate::models::Ping]>,
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

    // Pair durations against the wider 40-row window on the default view so a
    // run whose start sits just past row 20 still shows its duration; a filtered
    // or paged view pairs within its own slice (a start ping may be filtered
    // out, so pairing there is best-effort regardless).
    let durations = if matches!(cursor, PageCursor::Latest) && filter.is_empty() {
        if let Some(r) = recent {
            crate::view::run_durations(r)
        } else {
            let r = state.store.list_recent_pings(check_id, 40).await?;
            crate::view::run_durations(&r)
        }
    } else {
        crate::view::run_durations(&ping_page.items)
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
    let older = ping_page
        .has_older
        .then(|| ping_page.items.last())
        .flatten()
        .map(|p| history_href(base, check_id, "pings", ("pb", p.id), &carry));
    let newer = ping_page
        .has_newer
        .then(|| ping_page.items.first())
        .flatten()
        .map(|p| history_href(base, check_id, "pings", ("pa", p.id), &carry));

    Ok(CheckPingsTemplate {
        base: base.to_string(),
        check_id,
        empty: rows.is_empty(),
        rows,
        f_kind,
        f_from,
        f_to,
        filtered: !filter.is_empty(),
        newer,
        older,
    })
}

/// Build the "recent notifications" fragment for `check_id`, honoring the `n*`
/// filter and cursor params in `page`. `channel_names` labels rows by channel.
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
    let older = notif_page
        .has_older
        .then(|| notif_page.items.last())
        .flatten()
        .map(|n| history_href(base, check_id, "notifications", ("nb", n.id), &carry));
    let newer = notif_page
        .has_newer
        .then(|| notif_page.items.first())
        .flatten()
        .map(|n| history_href(base, check_id, "notifications", ("na", n.id), &carry));

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
        newer,
        older,
    })
}

/// Channel id → name map for a project, used to label notification rows in the
/// standalone notifications partial (the full page reuses its own map).
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

/// Build the SSE response for `check_id`'s live tail: one "changed" event per
/// broadcast that matches `check_id`, plus a "changed" event whenever this
/// subscriber lags (see the `Err` arm below).
fn sse_for_check(
    events: &broadcast::Sender<i64>,
    check_id: i64,
) -> Sse<impl Stream<Item = Result<Event, Infallible>> + use<>> {
    let stream = BroadcastStream::new(events.subscribe()).filter_map(move |res| match res {
        Ok(id) if id == check_id => Some(Ok(Event::default().data("changed"))),
        Ok(_) => None,
        // Lagged: this subscriber fell behind the buffer. Unlike a log tail,
        // where a dropped entry is just a missing row, a dropped *signal*
        // would leave the page stale forever — so coalesce the gap into one
        // refresh signal rather than discarding it.
        Err(_) => Some(Ok(Event::default().data("changed"))),
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// `GET /checks/{id}/pings` — the pings fragment for a JS partial refresh.
async fn check_pings(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Query(page): Query<CheckPageQuery>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    Ok(render(&build_pings_partial(&state, check.id, "", &page, None).await?)?.into_response())
}

/// `GET /checks/{id}/events` — Server-Sent Events signalling that this check
/// changed (a ping arrived, or the scan loop transitioned it). The event
/// carries no data: the page re-fetches the pings fragment, which keeps
/// rendering, filtering and authorization in one place.
async fn check_events(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    Ok(sse_for_check(&state.events, check.id))
}

/// `GET /checks/{id}/notifications` — the notifications fragment.
async fn check_notifications(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Query(page): Query<CheckPageQuery>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    let names = channel_name_map(&state, check.project_id).await?;
    Ok(render(&build_notifs_partial(&state, check.id, "", &page, &names).await?)?.into_response())
}

/// `GET /admin/checks/{id}/pings` — admin pings fragment (audited access).
async fn admin_check_pings(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Query(page): Query<CheckPageQuery>,
) -> Result<Response, AppError> {
    let check = admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    Ok(
        render(&build_pings_partial(&state, check.id, "/admin", &page, None).await?)?
            .into_response(),
    )
}

/// `GET /admin/checks/{id}/events` — admin twin of `check_events` (audited
/// access, same signal stream).
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

/// `GET /admin/checks/{id}/notifications` — admin notifications fragment.
async fn admin_check_notifications(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Query(page): Query<CheckPageQuery>,
) -> Result<Response, AppError> {
    let check = admin_check(&state, id, &admin, method.as_str(), uri.path()).await?;
    let names = channel_name_map(&state, check.project_id).await?;
    Ok(
        render(&build_notifs_partial(&state, check.id, "/admin", &page, &names).await?)?
            .into_response(),
    )
}

/// Build the check edit form pre-filled from `check`, pointing its action at
/// the owner or `/admin` route depending on `admin`. `is_admin` reflects the
/// current viewer's admin status and controls the nav Admin link.
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
                timezone: &form.timezone,
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
    csrf: String,
    is_admin: bool,
    admin: bool,
    project_id: i64,
    error: Option<String>,
    smtp_available: bool,
}

#[derive(Deserialize)]
pub(crate) struct ChannelForm {
    pub(crate) name: String,
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
    #[serde(default)]
    pub(crate) pushover_token: String, // application token
    #[serde(default)]
    pub(crate) pushover_user: String, // user/group key
    #[serde(default)]
    pub(crate) email_to: String,
}

/// Validate a channel form into `(kind, trimmed name, config JSON)` or an error
/// message. Shared by the web create handler and the programmatic API so both
/// enforce the same per-kind required fields and build the same stored config.
pub(crate) fn validate_channel(
    form: &ChannelForm,
) -> Result<(ChannelKind, String, String), String> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err("a channel name is required".into());
    }
    let kind =
        ChannelKind::from_str(&form.kind).map_err(|_e| "unknown channel kind".to_string())?;
    let config = match kind {
        ChannelKind::Webhook => {
            let url = form.webhook_url.trim();
            if url.is_empty() {
                return Err("a webhook URL is required".into());
            }
            serde_json::json!({ "url": url }).to_string()
        }
        ChannelKind::Slack => {
            let url = form.slack_url.trim();
            if url.is_empty() {
                return Err("a Slack incoming-webhook URL is required".into());
            }
            serde_json::json!({ "url": url }).to_string()
        }
        ChannelKind::Telegram => {
            let token = form.telegram_token.trim();
            let chat_id = form.telegram_chat_id.trim();
            if token.is_empty() || chat_id.is_empty() {
                return Err("Telegram requires both a bot token and a chat id".into());
            }
            serde_json::json!({ "token": token, "chat_id": chat_id }).to_string()
        }
        ChannelKind::Ntfy => {
            let topic = form.ntfy_topic.trim();
            if topic.is_empty() {
                return Err("ntfy requires a topic".into());
            }
            let base_url = {
                let b = form.ntfy_base_url.trim();
                if b.is_empty() { "https://ntfy.sh" } else { b }
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
                return Err("Pushover requires both an application token and a user key".into());
            }
            serde_json::json!({ "token": token, "user": user }).to_string()
        }
        ChannelKind::Email => {
            let to = form.email_to.trim();
            if to.is_empty() {
                return Err("an email recipient address is required".into());
            }
            serde_json::json!({ "to": to }).to_string()
        }
    };
    Ok((kind, name.to_string(), config))
}

#[derive(Deserialize)]
struct BindForm {
    #[serde(default)]
    channel_ids: Vec<i64>,
}

async fn channel_new(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    Ok(render(&ChannelFormTemplate {
        show_nav: true,
        csrf: current_csrf(&state, &jar),
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
    csrf: String,
) -> Result<Response, AppError> {
    let base = admin_prefix(admin);

    let err = |msg: &str| -> Result<Response, AppError> {
        Ok(render(&ChannelFormTemplate {
            show_nav: true,
            csrf: csrf.clone(),
            is_admin,
            admin,
            project_id: pid,
            error: Some(msg.to_string()),
            smtp_available: state.config.smtp.is_some(),
        })?
        .into_response())
    };

    let (kind, name, config) = match validate_channel(&form) {
        Ok(v) => v,
        Err(msg) => return err(&msg),
    };

    state
        .store
        .create_channel(pid, kind, &name, &config, Utc::now())
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

/// Replace a check's bound channel set with exactly the submitted ids (only
/// those that belong to the same project are honored). `admin` selects the
/// redirect route surface.
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
    let jar = jar.add(
        Cookie::build((FLASH_COOKIE, "channels"))
            .http_only(true)
            .same_site(SameSite::Lax)
            .path("/")
            .build(),
    );
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

/// Settings persist durations as raw seconds; render them in the readable form
/// the field now accepts. Anything unexpected passes through untouched so the
/// user still sees what is stored.
fn readable_setting_duration(raw: String) -> String {
    match raw.trim().parse::<i64>() {
        Ok(v) if v > 0 => crate::duration::fmt_duration(v),
        _ => raw,
    }
}

/// The four global settings fields as currently persisted, rendered in their
/// readable (duration-string) form. Shared by `render_admin`'s default path
/// and by `users_create`'s error re-render, which needs the same fields but
/// isn't otherwise touching settings.
async fn load_settings_fields(
    state: &AppState,
) -> Result<(String, String, String, String), AppError> {
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
    Ok((
        readable_setting_duration(scan_interval),
        readable_setting_duration(nag_interval),
        pings_retention_days,
        notifications_retention_days,
    ))
}

/// The settings-form fields to render on the merged `/admin` page: either the
/// four persisted values (the default path) or the raw values just submitted
/// to an invalid save, so the user can see and fix what they typed.
struct SettingsFields {
    scan_interval: String,
    nag_interval: String,
    pings_retention_days: String,
    notifications_retention_days: String,
}

/// The re-render inputs `render_admin` needs beyond the data it always
/// gathers itself (overview stats, users, projects): the settings section's
/// fields/error/flash, the users section's flash, and the add-user form's
/// error.
struct AdminRender {
    settings: SettingsFields,
    settings_error: Option<String>,
    settings_flash: Option<String>,
    user_flash: Option<String>,
    user_error: Option<String>,
}

async fn admin_page(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
) -> Result<Response, AppError> {
    let (scan_interval, nag_interval, pings_retention_days, notifications_retention_days) =
        load_settings_fields(&state).await?;
    // Chain both surfaces through the same jar: the cookie is path-scoped to
    // "/", so each `take_flash` call only consumes it if the value matches
    // its own surface, leaving it for the other to check.
    let (jar, settings_flash) = take_flash(jar, "settings");
    let (jar, user_flash) = take_flash(jar, "users_blocked");
    let resp = render_admin(
        &state,
        &jar,
        admin.id,
        AdminRender {
            settings: SettingsFields {
                scan_interval,
                nag_interval,
                pings_retention_days,
                notifications_retention_days,
            },
            settings_error: None,
            settings_flash,
            user_flash,
            user_error: None,
        },
    )
    .await?;
    Ok((jar, resp).into_response())
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
            true,
        ),
        (
            "nag_interval",
            form.nag_interval.as_str(),
            "Global nag interval",
            true,
        ),
        (
            "pings_retention_days",
            form.pings_retention_days.as_str(),
            "Pings retention",
            false,
        ),
        (
            "notifications_retention_days",
            form.notifications_retention_days.as_str(),
            "Notifications retention",
            false,
        ),
    ];
    // Atomic: validate every field before writing any. Blank clears to the
    // default (`Ok(None)`); scan/nag intervals accept a duration (raw seconds
    // or e.g. `5m`), the two retention fields are plain positive integers
    // (days); any non-blank invalid value aborts the whole save and
    // re-renders with the submitted values.
    let mut parsed: Vec<(&str, Option<i64>)> = Vec::with_capacity(fields.len());
    for (key, raw, label, is_duration) in fields {
        let result = if is_duration {
            parse_opt_positive_duration(raw, label)
        } else {
            parse_opt_positive(raw, label)
        };
        match result {
            Ok(v) => parsed.push((key, v)),
            Err(msg) => {
                let resp = render_admin(
                    &state,
                    &jar,
                    admin.id,
                    AdminRender {
                        settings: SettingsFields {
                            scan_interval: form.scan_interval.clone(),
                            nag_interval: form.nag_interval.clone(),
                            pings_retention_days: form.pings_retention_days.clone(),
                            notifications_retention_days: form.notifications_retention_days.clone(),
                        },
                        settings_error: Some(msg),
                        settings_flash: None,
                        user_flash: None,
                        user_error: None,
                    },
                )
                .await?;
                return Ok(resp);
            }
        }
    }
    for (key, v) in parsed {
        let value = v.map(|n| n.to_string()).unwrap_or_default();
        state.store.set_setting(key, &value).await?;
    }
    let jar = jar.add(
        Cookie::build((FLASH_COOKIE, "settings"))
            .http_only(true)
            .same_site(SameSite::Lax)
            .path("/")
            .build(),
    );
    Ok((jar, Redirect::to("/admin")).into_response())
}

async fn users_create(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    Form(form): Form<NewUserForm>,
) -> Result<Response, AppError> {
    if form.username.trim().is_empty() || form.password.is_empty() {
        let (scan_interval, nag_interval, pings_retention_days, notifications_retention_days) =
            load_settings_fields(&state).await?;
        let resp = render_admin(
            &state,
            &jar,
            admin.id,
            AdminRender {
                settings: SettingsFields {
                    scan_interval,
                    nag_interval,
                    pings_retention_days,
                    notifications_retention_days,
                },
                settings_error: None,
                settings_flash: None,
                user_flash: None,
                user_error: Some("username and password are required".into()),
            },
        )
        .await?;
        return Ok(resp);
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
    Ok(Redirect::to("/admin").into_response())
}

async fn users_delete(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    // Never allow deleting yourself — you'd lose your own account mid-session.
    if id == admin.id {
        return Ok(users_blocked(jar));
    }
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/admin").into_response());
    };
    // Refuse to delete the last enabled admin. Provably unreachable today:
    // the actor is always an enabled admin (AdminUser/resolve_user rejects
    // disabled users), and the self-guard above already rules out
    // target == actor, so a target that is a *different* enabled admin
    // implies count_enabled_admins() is already >= 2. Kept as
    // defence-in-depth behind the self-guard.
    if target.is_admin && !target.disabled && state.store.count_enabled_admins().await? <= 1 {
        return Ok(users_blocked(jar));
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
    Ok(Redirect::to("/admin").into_response())
}

async fn users_set_password(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
    Form(form): Form<PasswordForm>,
) -> Result<Response, AppError> {
    if form.password.is_empty() {
        return Ok(Redirect::to("/admin").into_response());
    }
    if state.store.find_user_by_id(id).await?.is_none() {
        return Ok(Redirect::to("/admin").into_response());
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
    Ok(Redirect::to("/admin").into_response())
}

async fn users_toggle_admin(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    // Never allow revoking your own admin rights — it would lock you out of
    // `/admin` immediately (the very next request re-resolves AdminUser and
    // 403s), mirroring the self-guards in `users_delete`/`users_set_disabled`.
    if id == admin.id {
        return Ok(users_blocked(jar));
    }
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/admin").into_response());
    };
    let new_admin = !target.is_admin;
    // Refuse to remove the last enabled admin. Provably unreachable today:
    // see `users_delete`'s comment — the self-guard above already rules out
    // target == actor, so a target that is a *different* enabled admin
    // implies count_enabled_admins() is already >= 2. Kept as
    // defence-in-depth behind the self-guard.
    if !new_admin
        && target.is_admin
        && !target.disabled
        && state.store.count_enabled_admins().await? <= 1
    {
        return Ok(users_blocked(jar));
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
) -> Result<Response, AppError> {
    // Never disable yourself.
    if id == admin.id {
        return Ok(users_blocked(jar));
    }
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/admin").into_response());
    };
    let new_disabled = !target.disabled;
    // Refuse to disable the last enabled admin. Provably unreachable today:
    // see `users_delete`'s comment — the self-guard above already rules out
    // target == actor, so a target that is a *different* enabled admin
    // implies count_enabled_admins() is already >= 2. Kept as
    // defence-in-depth behind the self-guard.
    if new_disabled
        && target.is_admin
        && !target.disabled
        && state.store.count_enabled_admins().await? <= 1
    {
        return Ok(users_blocked(jar));
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
    Ok(Redirect::to("/admin").into_response())
}

// --- account (session-authenticated self-service page for every logged-in
// user: sessions, then API keys, merged onto a single `/account` page) ---
//
// `sessions.id` is half of the session cookie — the signed half is derived
// from it (see `crate::secret`), so it is still a bearer secret and must never
// be rendered or appear in a URL. Rows are identified in the UI (and in the
// revoke route) by `handle`, the SHA-256 hex of the id, computed with the same
// helper the API-key hashing uses. Session lists are tiny, so resolving a
// handle back to a row is a linear scan rather than an indexed lookup.
#[derive(Template)]
#[template(path = "account.html")]
struct AccountTemplate {
    show_nav: bool,
    csrf: String,
    is_admin: bool,
    // sessions section
    sessions: Vec<SessionRow>,
    /// Count of non-current sessions, so the template can hide the "revoke
    /// others" control when there is nothing else to revoke.
    other_count: usize,
    // api-keys section
    keys: Vec<ApiKeyRow>,
    /// The plaintext token, rendered exactly once right after creation and
    /// never recoverable afterwards.
    new_token: Option<String>,
    key_error: Option<String>,
}

/// One row of the sessions table. Mirrors [`crate::models::Session`], minus
/// the raw `id` (never exposed) and plus the derived `handle` + `current`.
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

/// One row of the API-keys table. Mirrors [`crate::models::ApiKey`] plus a
/// precomputed `expired` flag (an expired key still lists so it can be revoked,
/// but is flagged so the user knows it no longer authenticates).
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
}

async fn account_page(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
) -> Result<Response, AppError> {
    render_account(&state, &jar, &user, None, None).await
}

/// Gather both the sessions and API-keys datasets and render the merged
/// `/account` page.
async fn render_account(
    state: &AppState,
    jar: &CookieJar,
    user: &User,
    new_token: Option<String>,
    key_error: Option<&str>,
) -> Result<Response, AppError> {
    let now = Utc::now();

    // The handle hashes the session *id*, so the cookie must be unwrapped first
    // — hashing the raw cookie value would never match any row.
    let current_handle = secret::session_id_from_jar(jar, &state.config.secret)
        .map(|id| crate::apikey::hash_api_key(&id));
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
    // `list_sessions_for_user` already returns newest-created-first; a stable
    // sort on "is this the current session" preserves that ordering within
    // each group while pulling the current row to the top.
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
        keys,
        new_token,
        key_error: key_error.map(str::to_string),
    })?
    .into_response())
}

async fn api_keys_create(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Form(form): Form<NewApiKeyForm>,
) -> Result<Response, AppError> {
    let name = form.name.trim();
    if name.is_empty() {
        return render_account(&state, &jar, &user, None, Some("a name is required")).await;
    }
    // Optional expiry: blank means never; otherwise a duration from now
    // (`30d`, `12h`, …) reusing the same parser as the check/duration fields.
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
                        None,
                        Some("expiry must be a duration like 30d, or blank for never"),
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
    render_account(&state, &jar, &user, Some(full), None).await
}

async fn api_keys_delete(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    // Owner-scoped delete; a key the caller doesn't own is silently a no-op.
    state.store.delete_api_key(id, user.id).await?;
    Ok(Redirect::to("/account").into_response())
}

async fn sessions_revoke(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
    Path(handle): Path<String>,
) -> Result<Response, AppError> {
    // Resolve the handle among the caller's own sessions; an unknown or
    // foreign handle is a silent no-op (never a 500), mirroring the
    // API-key/project/check owner-scoped delete pattern.
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
    let is_current =
        secret::session_id_from_jar(&jar, &state.config.secret).is_some_and(|id| id == target.id);
    state
        .store
        .delete_session_owned(&target.id, user.id)
        .await?;
    if is_current {
        // Must carry `path("/")` to match how the cookie was set — a
        // pathless removal cookie gets this route's own path
        // (`/account/sessions/{handle}/revoke`) and would not clear a
        // `path=/` cookie.
        let jar = jar.remove(Cookie::build((SESSION_COOKIE, "")).path("/").build());
        return Ok((jar, Redirect::to("/login")).into_response());
    }
    Ok((jar, Redirect::to("/account")).into_response())
}

async fn sessions_revoke_others(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
) -> Result<Response, AppError> {
    if let Some(id) = secret::session_id_from_jar(&jar, &state.config.secret) {
        state
            .store
            .delete_other_sessions_for_user(user.id, &id)
            .await?;
    }
    Ok(Redirect::to("/account").into_response())
}

// --- admin route group (cross-user management, every access audited) ---
//
// Each handler resolves its target through the `admin_*` helpers (which fetch
// unfiltered and write one `admin.access` audit row), then reuses the exact
// same core logic/render helper/mutator as the owner handler, differing only in
// pointing links and redirects at the `/admin`-prefixed route surface.
/// One environment-configured setting, as displayed on `/admin`. The value is
/// redacted or summarised in Rust *before* it lands here, so a secret never
/// crosses into the template and no template change can print one.
struct EnvSetting {
    var: &'static str,
    value: EnvValue,
    default: &'static str,
    description: &'static str,
}

/// How a setting's current value is presented. `Secret` carries only whether
/// something is configured — never the value itself.
enum EnvValue {
    Set(String),
    Unset,
    Secret(bool),
}

/// Group every env-configured setting (nothing in this DB) into the sections
/// shown on the read-only `/admin` "Environment" card. Values are the
/// process's *current effective* config, so this reflects what's actually
/// running, not just what's documented as a default.
fn env_settings(config: &crate::config::Config) -> Vec<(&'static str, Vec<EnvSetting>)> {
    let log_format = match config.log_format {
        crate::config::LogFormat::Text => "text",
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
            default: "text",
            description: "Log line format (text or json); applied at process startup — changing it requires a restart.",
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

/// Strip credentials from a database URL for display: `scheme://user:pw@host/db`
/// becomes `scheme://***@host/db`. Anything without an `@` in its authority
/// (e.g. a plain `SQLite` path) is returned unchanged. Never returns the password.
fn redact_db_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let authority_start = scheme_end + 3;
    let rest = &url[authority_start..];
    // Only an `@` found before any of `/`, `?`, `#` counts as authority
    // credentials — an `@` in a later path/query/fragment must not be treated
    // as one (e.g. `...?callback=user@host`).
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

/// `/admin` landing: one merged page — site-wide overview (scale, check
/// health, notification health, scheduler heartbeat), global settings, user
/// management, and every project across all users — stacked as ordinary
/// cards (no tabs/sub-nav), mirroring how `/account` merges its sections.
/// Field names are shared with the four templates this replaced where they
/// don't collide; collisions get a section prefix (`settings_*`, `user_*`,
/// and the overview's scale counters `user_count`/`project_count` to leave
/// `users`/`projects` for the user-management and all-projects lists below).
#[derive(Template)]
#[template(path = "admin.html")]
struct AdminTemplate {
    show_nav: bool,
    csrf: String,
    is_admin: bool,
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
    // settings
    scan_interval: String,
    nag_interval: String,
    pings_retention_days: String,
    notifications_retention_days: String,
    settings_error: Option<String>,
    settings_flash: Option<String>,
    // users
    users: Vec<UserRow>,
    user_flash: Option<String>,
    user_error: Option<String>,
    // all projects
    projects: Vec<(Project, String)>,
    // environment
    env_rows: Vec<(&'static str, Vec<EnvSetting>)>,
}

/// One row of the `/admin` "All users" table. Mirrors [`crate::models::User`]
/// plus a precomputed `is_self` (`u.id == admin.id`), so the template can
/// render the signed-in admin's own row with inert self-mutation controls
/// (delete/toggle-admin/toggle-disabled) without comparing ids itself.
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

/// Gather every dataset the merged `/admin` page needs and render it. `r`
/// carries the settings-section fields/error/flash and the add-user error —
/// the only parts that vary across the page's three entry points (the plain
/// GET, a rejected settings save, and a rejected add-user submission); every
/// other section (overview stats, users list, projects list) is always
/// freshly loaded from the store.
async fn render_admin(
    state: &AppState,
    jar: &CookieJar,
    admin_id: i64,
    r: AdminRender,
) -> Result<Response, AppError> {
    let day_ago = Utc::now() - Duration::days(1);
    let (notif_ok, notif_err) = state.store.notification_counts_since(day_ago).await?;
    Ok(render(&AdminTemplate {
        show_nav: true,
        csrf: current_csrf(state, jar),
        is_admin: true,
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
        last_scan_at: state.store.get_setting("last_scan_at").await?,
        last_prune_at: state.store.get_setting("last_prune_at").await?,
        scan_interval: r.settings.scan_interval,
        nag_interval: r.settings.nag_interval,
        pings_retention_days: r.settings.pings_retention_days,
        notifications_retention_days: r.settings.notifications_retention_days,
        settings_error: r.settings_error,
        settings_flash: r.settings_flash,
        users: state
            .store
            .list_users()
            .await?
            .into_iter()
            .map(|u| UserRow::from_user(u, admin_id))
            .collect(),
        user_flash: r.user_flash,
        user_error: r.user_error,
        projects: state.store.list_all_projects_with_owner().await?,
        env_rows: env_settings(&state.config),
    })?
    .into_response())
}

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
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    admin_project(&state, id, &admin, method.as_str(), uri.path()).await?;
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
    let (jar, flash) = take_flash(jar, "channels");
    let resp = render_check_page(&state, check, true, true, csrf, flash, page).await?;
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
    Ok(render(&ChannelFormTemplate {
        show_nav: true,
        csrf: current_csrf(&state, &jar),
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
        // Blank (unset) and anything that is not a positive integer must survive
        // untouched so the user still sees exactly what is stored.
        assert_eq!(readable_setting_duration(String::new()), "");
        assert_eq!(readable_setting_duration("0".into()), "0");
        assert_eq!(readable_setting_duration("abc".into()), "abc");
    }

    #[test]
    fn take_flash_maps_each_surface_to_its_own_message() {
        let jar = CookieJar::new().add(Cookie::new(FLASH_COOKIE, "settings"));
        let (_, msg) = take_flash(jar, "settings");
        assert_eq!(msg.as_deref(), Some("Settings saved."));

        let jar = CookieJar::new().add(Cookie::new(FLASH_COOKIE, "channels"));
        let (_, msg) = take_flash(jar, "channels");
        assert_eq!(msg.as_deref(), Some("Notify channels saved."));

        let jar = CookieJar::new().add(Cookie::new(FLASH_COOKIE, "users_blocked"));
        let (_, msg) = take_flash(jar, "users_blocked");
        assert_eq!(
            msg.as_deref(),
            Some(
                "That action was refused: you cannot remove your own access, and the last enabled admin cannot be removed."
            )
        );
    }

    #[test]
    fn take_flash_ignores_a_flash_set_for_another_surface() {
        // The cookie is path-scoped to "/", so the settings page also sees a
        // check-page flash. It must neither render nor consume it — the page it
        // was set for still gets it.
        let jar = CookieJar::new().add(Cookie::new(FLASH_COOKIE, "channels"));
        let (jar, msg) = take_flash(jar, "settings");
        assert_eq!(msg, None);
        let (_, msg) = take_flash(jar, "channels");
        assert_eq!(msg.as_deref(), Some("Notify channels saved."));
    }

    #[test]
    fn take_flash_without_a_cookie_is_none() {
        let (_, msg) = take_flash(CookieJar::new(), "settings");
        assert_eq!(msg, None);
    }

    #[test]
    fn take_flash_never_renders_an_unknown_cookie_value() {
        // Even when the surface matches, an unknown key maps to no message, so a
        // user-supplied cookie value can never render as arbitrary text.
        let jar = CookieJar::new().add(Cookie::new(FLASH_COOKIE, "<script>"));
        let (_, msg) = take_flash(jar, "<script>");
        assert_eq!(msg, None);
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
