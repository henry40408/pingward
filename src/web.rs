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
        .route("/admin/audit", get(admin_audit_fragment))
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
    /// One-shot warning shown after a forward-auth Sign Out with no gateway
    /// logout URL configured: the local session was cleared but the proxy will
    /// re-authenticate, so the visitor must sign out at their proxy/SSO
    /// provider. `None` on an ordinary dashboard load.
    forward_auth_logout: Option<String>,
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

// Deliberately not rate-limited, unlike `login_submit`: `/setup` only ever
// accepts requests while `count_users() == 0`, so there is no credential yet
// to brute-force.
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
    PeerAddr(peer_ip): PeerAddr,
    Form(creds): Form<Credentials>,
) -> Result<Response, AppError> {
    // Rate-limit key is deliberately not `conn` (`ping::ClientIp`, which
    // stamps the session's `ip` column below) — see `ratelimit::rate_limit_key`
    // for why attribution and the security control must diverge. Reserving
    // the attempt before `find_user_by_username` means a throttled request
    // never pays for the argon2 verification that follows.
    let client = crate::ratelimit::rate_limit_key(peer_ip, &headers, &state.config.trusted_proxies);
    if !state.login_limiter.try_acquire(client) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            [(
                header::RETRY_AFTER,
                crate::ratelimit::WINDOW_SECS.to_string(),
            )],
            render(&LoginTemplate {
                show_nav: false,
                csrf: current_csrf(&state, &jar),
                is_admin: false,
                error: Some("too many login attempts — try again in a minute".into()),
            })?,
        )
            .into_response());
    }
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
        // Not released: valid credentials for a disabled account are not a
        // successful login. Releasing here would let an attacker holding one
        // known-disabled credential probe this client's budget indefinitely.
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
    // A successful login hands the reservation back so signing in repeatedly
    // (several devices, a test suite) never exhausts the window.
    state.login_limiter.release(client);
    Ok((jar, Redirect::to("/")).into_response())
}

/// Ends the local session, then sends the browser onward.
///
/// Deleting the row is never enough behind an authentication gateway: the next
/// request still carries the gateway's identity header, so
/// `forward_auth_session` signs the visitor straight back in. Only the gateway
/// can end that identity, which is what `PINGWARD_FORWARD_AUTH_LOGOUT_URL` is
/// for — it points at the gateway's own sign-out endpoint. When configured, we
/// hand the browser there regardless of how this request authenticated.
///
/// Left unset, the destination depends on whether the gateway's identity header
/// is present on *this* request (the same trusted-proxy gate
/// `forward_auth_session` uses):
/// - present → a local logout is a no-op, since the next request re-mints the
///   session. Rather than bounce to `/login` and silently sign the visitor
///   straight back in, land on the dashboard with a one-shot flash telling them
///   only their proxy/SSO provider can end the session.
/// - absent → an ordinary password session; redirect to `/login` as before.
///
/// The redirect target comes from the operator's environment or a fixed
/// in-app path, never from the request, so it is not an open redirect.
///
/// Two of the three exits also send `Clear-Site-Data` (see
/// [`CLEAR_SITE_DATA`]) so the browser drops this origin's cache. The
/// forward-auth flash exit deliberately does not: it is not a credential
/// teardown at all — the gateway re-mints the session on the very next
/// request no matter what this response sends — and its whole job is
/// delivering the `pingward_flash` cookie that renders the "only your
/// proxy/SSO provider can end this session" notice.
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
        tracing::info!(
            target: "pingward::session",
            reason = "logout",
            handle = %crate::auth::session_log_handle(&id),
            "session.destroyed"
        );
    }
    let jar = jar.remove(session_removal_cookie(&state.config));

    // A configured gateway logout URL ends the upstream identity too, so honour
    // it however the request authenticated. We deliberately do not include
    // "cookies" in `Clear-Site-Data` here: that directive clears cookies for
    // the entire registrable domain, not just this origin, so on a typical
    // SSO layout (pingward and the gateway as sibling subdomains of the same
    // parent domain) it would wipe the gateway's own session cookie before
    // the browser even follows the redirect — breaking the logout handoff
    // this URL exists for, and signing the user out of every other app on
    // the domain too. `"cache"` alone is origin-scoped and safe to send.
    if let Some(url) = state.config.forward_auth_logout_url.as_deref() {
        return Ok((
            jar,
            [(HeaderName::from_static("clear-site-data"), CLEAR_SITE_DATA)],
            Redirect::to(url),
        )
            .into_response());
    }

    // No gateway logout URL. If the trusted proxy identity header is present,
    // clearing the local session cannot outlive the redirect — be honest about
    // it instead of pretending logout succeeded.
    if crate::auth::forward_auth_username(&headers, peer_ip, &state.config).is_some() {
        // Deliberately no Clear-Site-Data here: this exit is not a credential
        // teardown at all — the gateway re-mints the session on the very next
        // request no matter what this response sends — so there is nothing
        // to ask the browser to drop. Its whole job is delivering the
        // `pingward_flash` cookie the dashboard needs to render the warning
        // below. Do not "restore consistency" by adding the header back —
        // see `logout`'s doc comment.
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

/// Ask the browser to drop this origin's cache on logout.
///
/// Deliberately **excludes** `"cookies"`: unlike the other directives, it is
/// scoped to the whole *registered domain*, including subdomains — not just
/// this origin. On the SSO layout `PINGWARD_FORWARD_AUTH_LOGOUT_URL` is meant
/// for (pingward and its gateway as sibling subdomains), sending it would
/// clear the gateway's own session cookie before the browser follows the
/// redirect, breaking the logout handoff and signing the user out of every
/// other app on the domain. The session cookie is already ended by the
/// removal `Set-Cookie` (`session_removal_cookie`), which *is* origin- and
/// path-scoped, so nothing is lost by leaving "cookies" out here.
/// Also deliberately **excludes** `"storage"`: the theme preference lives in
/// `localStorage['pw-theme']` (templates/base.html), so clearing it would
/// reset the user's appearance setting on every logout — and pingward keeps
/// nothing secret in localStorage, so it is pure functional regression.
/// `"executionContexts"` is excluded for the same kind of reason: it forces a
/// reload, which fights with the redirect we are already issuing. Browsers
/// only honour `Clear-Site-Data` on a trustworthy origin, so on a plain-HTTP
/// deployment (`PINGWARD_COOKIE_SECURE` off) sending it is a harmless no-op,
/// not a security control.
const CLEAR_SITE_DATA: &str = r#""cache""#;

/// The request's socket peer IP, or `None` when the router is driven without
/// `ConnectInfo` (e.g. some tests) — the same fail-closed source
/// `forward_auth_session` reads, so `logout`'s trusted-proxy check agrees with
/// how the visitor was authenticated in the first place. Always succeeds.
struct PeerAddr(Option<IpAddr>);

impl FromRequestParts<AppState> for PeerAddr {
    type Rejection = Infallible;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(crate::auth::peer_ip(&parts.extensions)))
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
    // One-shot notice set by `logout` when a forward-auth visitor signed out
    // with no gateway logout URL configured; consumed here so the removal
    // Set-Cookie rides back on this response.
    let (jar, forward_auth_logout) = take_flash(&state.config, jar, "forward_auth_logout");
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
        .list_recent_ping_summaries_for_checks(&check_ids, 40)
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

/// The session cookie name this process uses, resolved from `AppState`. A
/// one-line wrapper around `auth::session_cookie_name` to cut the repetition
/// at each of this file's several call sites.
fn session_cookie_name(state: &AppState) -> &'static str {
    crate::auth::session_cookie_name(state.config.cookie_secure)
}

/// The one place a session cookie is built. Its attributes must match
/// `session_removal_cookie` exactly — RFC 6265bis §5.5 ("Leave Secure Cookies
/// Alone") means a removal cookie whose attributes differ can fail to overwrite
/// the original in some browsers.
fn session_cookie(config: &crate::config::Config, value: String) -> Cookie<'static> {
    Cookie::build((
        crate::auth::session_cookie_name(config.cookie_secure),
        value,
    ))
    .http_only(true)
    .same_site(SameSite::Lax)
    .path("/")
    .secure(config.cookie_secure)
    // Deliberately no Max-Age/Expires: a non-persistent cookie is OWASP's
    // explicit preference for an authenticated session. Do not add one —
    // expiry is the server's job (`sessions.expires_at`).
    .build()
}

/// The empty-valued cookie used to clear the session cookie; attributes are
/// aligned with `session_cookie`.
fn session_removal_cookie(config: &crate::config::Config) -> Cookie<'static> {
    session_cookie(config, String::new())
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
    if secret::session_id_from_jar(&jar, &state.config.secret, session_cookie_name(&state))
        .is_some()
    {
        return next.run(req).await;
    }
    // Signature only — an anonymous id has no row to look up, and a stale
    // signed cookie is left alone so its owner keeps one stable token.
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

/// Rewrite the request's `Cookie` header so downstream extractors see `cookie`
/// instead of whatever value it had for that name.
///
/// Dropping the stale entry matters: `CookieJar::get` returns the first match,
/// so appending would leave an expired session id shadowing the fresh one.
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

/// Resolve the current session's CSRF synchronizer token from the request
/// cookies, for embedding as a hidden `_csrf` field in rendered POST forms.
/// Returns an empty string when the request carries no valid session (e.g. the
/// pre-session `login`/`setup` pages, which carry exempt forms) — an empty
/// token yields an unsubmittable form rather than a token-less bypass.
fn current_csrf(state: &AppState, jar: &CookieJar) -> String {
    secret::session_id_from_jar(jar, &state.config.secret, session_cookie_name(state))
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
    let Some(session_id) = secret::session_id_from_jar(&jar, secret, session_cookie_name(&state))
    else {
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

/// Add `Cache-Control: no-store` to every browser response.
///
/// Applied to the whole of `web::routes()`, not just authenticated pages:
/// `/login` and `/setup` render a `_csrf` bound to that visitor's cookie (see
/// `anonymous_session`), so they must not be cached either. The machine
/// `/ping/*` endpoints, static assets and `/healthz` are sibling routers and
/// are structurally unaffected — `src/assets.rs`'s immutable caching is
/// untouched. `/api/*` is mostly exempt the same structural way, but not
/// uniformly: `api::routes()` layers this same function a second time,
/// scoped to just `/api/docs` and `/api/openapi.json`, because those two
/// accept a logged-in web session (`CurrentUser`) alongside `/api/v1`'s
/// bearer auth and are therefore session-authenticated responses too.
/// `/api/v1` stays exempt on purpose: it is bearer-authenticated, was never
/// going to carry a browser-cacheable session, and adding response headers
/// there would affect API consumers for no benefit.
///
/// Only filled in when the response does not already carry a `Cache-Control`,
/// so any handler that wants to override still can.
///
/// The legacy `Pragma: no-cache` / `Expires: 0` pair is deliberately not
/// added: every modern browser honours `no-store`, and those two headers only
/// ever meant anything to HTTP/1.0 caches.
pub async fn no_store(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    if !resp.headers().contains_key(header::CACHE_CONTROL) {
        resp.headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    resp
}

/// Adds `Strict-Transport-Security` when `PINGWARD_HSTS_MAX_AGE` is
/// configured. A zero-cost no-op by default: pingward does not terminate TLS,
/// so sending HSTS unconditionally would tell browsers "HTTPS only" on a
/// deployment that may be plain HTTP behind an internal reverse proxy — the
/// reverse proxy is the right place to set this header, and `README.md`'s
/// "Running behind a reverse proxy" section documents that. This knob exists
/// for operators who cannot edit proxy headers.
///
/// App-wide rather than `web`-only (unlike [`no_store`]): the point of HSTS is
/// telling the browser the *origin* is HTTPS-only, which applies just as much
/// to `/ping/*`, `/healthz` and static assets as to the browser UI. It is
/// layered in `lib.rs` outside every `.merge(...)`, not inside the `web`
/// router.
///
/// Deliberately emits neither `includeSubDomains` nor `preload`: both are
/// close to irreversible once a browser caches them (a wrong
/// `includeSubDomains` takes out unrelated hosts on the same domain, and
/// `preload` list removal can take months). An operator who wants either sets
/// it on the reverse proxy.
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

/// Project page's per-channel row. Same reason [`ChannelEditView`] exists: a
/// stored [`Channel`] carries `config_json`, which holds delivery secrets
/// (webhook/Slack URLs, bot tokens, SMTP credentials). This page renders only a
/// channel's name and kind, so handing the template the whole model would put
/// those secrets in the render context of a page that has no use for them — one
/// stray `{{ ch.config_json }}` away from a leak. Projecting to the three fields
/// the template actually reads makes that impossible by construction rather than
/// by review.
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

/// Whether a cross-user admin request should be audited on its own.
///
/// A `GET` through `/admin/*` is a read: it renders names, schedules and
/// history, none of which is a credential, and auditing every page open
/// buried the entries that matter under browsing noise. Mutations still write
/// an entry, and so does the one read that *does* hand over a credential —
/// revealing a check's ping URL, which has its own explicit action
/// (`admin.ping_url_reveal`) rather than riding on the resolver.
///
/// The resolvers below are the choke point for both reads and writes, so this
/// gate lives here rather than at each call site: without it, dropping the
/// read audit would silently take every admin pause/resume/delete/regenerate
/// with it.
fn audits_as_mutation(method: &str) -> bool {
    !method.eq_ignore_ascii_case("GET")
}

/// Resolve any project by id (no owner filter), auditing the request when it
/// mutates (see [`audits_as_mutation`]). The single choke point for cross-user
/// project reads and writes.
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
    /// When the next ping is expected — see [`crate::view::next_due`].
    next_due: crate::view::NextDue,
    schedule: String,
    ping_url: String,
    /// The ping URL is withheld pending an audited reveal — see
    /// [`CheckPageViewer`]. The template renders the reveal control instead of
    /// the URL and its usage help, both of which spell the credential out.
    ping_url_hidden: bool,
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

/// Parse a free-text filter param: trimmed, with blank treated as "no
/// constraint". The audit filter's actor/action are stored tokens matched
/// exactly, so unlike [`parse_filter_enum`] there is no enum to validate
/// against — a value that matches nothing simply returns an empty page.
fn parse_filter_text(v: Option<&str>) -> Option<String> {
    v.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
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

/// Build a history-fragment href for a keyset pager link. `path` is the
/// fragment endpoint the link re-fetches (`{base}/checks/{id}/pings`,
/// `/admin/audit`, …), `cursor` is this table's new position, and `carry`
/// re-attaches the currently-active filter tokens so paging preserves the
/// filter. Values are ids, enum tokens, or `Z`-form datetimes — all
/// query-safe, so no encoding.
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
    pub(crate) timezone: String,
    pub(crate) scan_interval_secs: Option<i64>,
    pub(crate) max_runtime_secs: Option<i64>,
    pub(crate) nag_interval_secs: Option<i64>,
}

/// Validate the check's IANA timezone. Blank means UTC — both the column
/// default and the API's `default_timezone` already say so, and a form posted
/// without the field should land on the same value rather than an error.
///
/// A typo used to be stored verbatim and then silently ignored: `due_time`
/// falls back to UTC (with a `tracing::warn!` nobody reads) and the check's
/// cron simply fires on the wrong wall clock. Rejecting it here is the only
/// place the operator finds out, so it returns the offending value.
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

/// Name of the one-shot flash cookie set after a redirect (e.g. saving a
/// check's notify channels) and cleared on the next render, when the cookie
/// carries no `Secure` attribute.
const FLASH_COOKIE_BASE: &str = "pingward_flash";

/// `__Host-`-prefixed name of the flash cookie, used when
/// `PINGWARD_COOKIE_SECURE` is on. Same pairing as the session cookie (see
/// [`crate::auth::session_cookie_name`]) and legal for the same reasons: the
/// cookie is `Secure`, path-scoped to `/`, and carries no `Domain`.
const FLASH_COOKIE_HOST_PREFIXED: &str = "__Host-pingward_flash";

/// The flash cookie's name for this deployment.
///
/// The cookie carries no authority — [`take_flash`] maps only known keys to a
/// fixed message and the password-reset variant `u64`-parses its counts, so a
/// forged value can neither elevate nor inject markup. What the `__Host-`
/// prefix and the signature ([`secret::sign_flash`]) close is *provenance*: a
/// response from a sibling subdomain could otherwise plant a flash this origin
/// never set, showing the user — including an admin reading a residual-API-key
/// count — a message the server never sent. The prefix stops a sibling writing
/// the cookie at all under HTTPS; the signature covers the plain-HTTP
/// deployment, where no prefix is available.
fn flash_cookie_name(config: &crate::config::Config) -> &'static str {
    if config.cookie_secure {
        FLASH_COOKIE_HOST_PREFIXED
    } else {
        FLASH_COOKIE_BASE
    }
}

/// Read and clear the one-shot flash cookie **if** it was set for `surface`,
/// mapping it to that surface's fixed message. The cookie is path-scoped to
/// `/`, so every page sees it — a flash set for another surface is therefore
/// left in the jar for that page to consume rather than rendered here, which
/// keeps a message from surfacing on the wrong page when a redirect is not
/// followed or two tabs race. Only known keys map to a message, so a
/// user-supplied cookie value never renders as arbitrary text.
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

/// The verified flash payload carried by `jar`, if any: the cookie's value
/// with its signature stripped, or `None` when the cookie is absent,
/// malformed, or was not signed by this process's secret. Rotating
/// `PINGWARD_SECRET` therefore discards any in-flight flash, exactly as it
/// discards sessions.
fn flash_payload(config: &crate::config::Config, jar: &CookieJar) -> Option<String> {
    let cookie = jar.get(flash_cookie_name(config))?;
    secret::verify_flash(&config.secret, cookie.value())
}

/// Build a one-shot flash cookie carrying `value`, path-scoped to `/` so any
/// page can consume it via [`take_flash`]. Shared by [`flash_cookie`] (a
/// fixed surface key) and [`password_reset_keys_flash`] (a value with counts
/// baked in) so their cookie attributes cannot drift apart.
///
/// The stored value is `<payload>.<hmac>` — see [`flash_cookie_name`] for why
/// a cookie that carries no authority is signed anyway.
fn flash_cookie_value(config: &crate::config::Config, value: String) -> Cookie<'static> {
    flash_cookie_raw(config, secret::sign_flash(&config.secret, &value))
}

/// The flash cookie's attributes in one place, over an already-final cookie
/// value. Both the signed setter ([`flash_cookie_value`]) and the empty-valued
/// remover ([`flash_removal_cookie`]) go through here so the attributes cannot
/// drift apart — see `session_removal_cookie` for why that matters.
fn flash_cookie_raw(config: &crate::config::Config, raw: String) -> Cookie<'static> {
    Cookie::build((flash_cookie_name(config), raw))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .secure(config.cookie_secure)
        .build()
}

/// Build a one-shot flash cookie carrying `surface`, path-scoped to `/` so any
/// page can consume it via [`take_flash`]. The value is a fixed surface key,
/// never user input — [`take_flash`] maps only known keys to a message.
///
/// Consistency tidy-up, not a security fix: the flash cookie carries no
/// session material, but there is no reason for it to diverge from the
/// session cookie's `Secure` behaviour.
fn flash_cookie(config: &crate::config::Config, surface: &'static str) -> Cookie<'static> {
    flash_cookie_value(config, surface.to_string())
}

/// The empty-valued cookie used to clear the flash cookie; attributes are
/// aligned with `flash_cookie` (see `session_removal_cookie` for why this
/// matters).
///
/// The value is left empty rather than signed: removal is carried by the
/// attributes, and an unsigned value is rejected by [`flash_payload`] on read
/// anyway, so signing one would only make the cleared cookie harder to read
/// in a trace.
fn flash_removal_cookie(config: &crate::config::Config) -> Cookie<'static> {
    flash_cookie_raw(config, String::new())
}

/// Set the `users_blocked` flash cookie and redirect to `/admin`. Used by the
/// self-guard and last-enabled-admin-guard branches in `users_delete`,
/// `users_toggle_admin` and `users_set_disabled` — mirrors how
/// `settings_save` sets `FLASH_COOKIE` for its own surface (~line 2413).
fn users_blocked(config: &crate::config::Config, jar: CookieJar) -> Response {
    let jar = jar.add(flash_cookie(config, "users_blocked"));
    (jar, Redirect::to("/admin")).into_response()
}

/// Prefix for the dynamic flash value [`password_reset_keys_flash`] sets and
/// [`take_password_reset_keys_flash`] reads back: `"password_reset_keys:
/// <revoked>:<keys>"`. A separate scheme from [`take_flash`]'s, because that
/// helper maps a fixed cookie value to a fixed message — there is no room in
/// it for a message with numbers baked in.
const PASSWORD_RESET_KEYS_PREFIX: &str = "password_reset_keys:";

/// Set the `password_reset_keys` flash cookie and redirect to `/admin`.
/// Called by `users_set_password` only when the target still has at least
/// one still-usable API key after the reset, to surface the gap its doc
/// comment already notes: the reset revokes sessions but never API keys.
/// `revoked` and `keys` are always server-computed (a `DELETE`'s row count
/// and a count of `Store::list_api_keys_for_user`'s rows filtered to those
/// not yet expired), never user input, so baking them into the cookie value
/// carries no injection risk.
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

/// Read and clear the one-shot flash cookie if it carries a
/// [`PASSWORD_RESET_KEYS_PREFIX`] value set by [`password_reset_keys_flash`].
/// Mirrors [`take_flash`]'s one-shot-cookie contract but, unlike that
/// function, decodes two counts out of the value rather than mapping it to a
/// fixed message.
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
    // Covers both a self-targeted and an other-targeted reset, since
    // `users_set_password` allows both. Only the key's owner can revoke it,
    // from their own /account page — true whether that owner is the acting
    // admin or someone else. Disabling is offered as the admin's lever for
    // *another* user's account only: `users_set_disabled` unconditionally
    // refuses a self-targeted disable, so naming that option on a self-reset
    // would point at a control the admin cannot actually use.
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

/// Who is looking at a check page. Decides both the action-URL prefix and
/// whether the ping URL may be printed, which is why it replaced the separate
/// `admin: bool` — the two could otherwise be passed contradicting each other.
///
/// The ping URL is a bearer credential — holding it is enough to mark the
/// check up or down — so it is shown freely to the owner and withheld from an
/// admin looking at someone else's check until they ask, which is audited
/// (`admin.ping_url_reveal`). An admin viewing a check they own themselves is
/// not gated: it is their own credential, and `viewer_id == owner_id` is what
/// distinguishes that from a cross-user view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckPageViewer {
    /// An owner-route render: the URL is the page's whole point.
    Owner,
    /// An `/admin`-route render.
    Admin {
        viewer_id: i64,
        ping_url_revealed: bool,
    },
}

impl CheckPageViewer {
    /// True for an `/admin`-route render, which prefixes every action URL.
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

/// Render the check detail page. `admin` renders `/admin`-prefixed action URLs;
/// `is_admin` reflects the current viewer's admin status and controls the nav
/// Admin link. `page` carries the independent ping/notification keyset
/// cursors read from the request's query string.
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
    // The ping URL is a bearer credential: anyone holding it can mark the
    // check up or down. On the owner's own page that is the whole point of
    // the page, but an admin opening someone else's check gets it behind an
    // explicit, audited reveal — otherwise "just looking" silently hands over
    // a way to falsify that check's status with nothing recorded anywhere.
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
    // The heartbeat/bars strip always shows the latest 40 pings, independent
    // of the table's paging below — a paged (older) result must never feed it.
    // Narrowed to the four columns the strip and the duration pairing read, so
    // this window never materialises the captured bodies (see #116); the
    // table's own rows come from `list_pings_page` and are still full pings.
    let recent = state.store.list_recent_ping_summaries(id, 40).await?;
    let bars = crate::view::heartbeat(
        &recent,
        check.max_runtime_secs,
        check.status == CheckStatus::Paused,
        30,
    );

    let status = crate::view::display_status(&check, now).as_str();
    let since = status_since_label(&check, now);
    let next_due = crate::view::next_due(&check, now);
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

    // Pair durations against the wider 40-row window on the default view so a
    // run whose start sits just past row 20 still shows its duration; a filtered
    // or paged view pairs within its own slice (a start ping may be filtered
    // out, so pairing there is best-effort regardless).
    let durations = if matches!(cursor, PageCursor::Latest) && filter.is_empty() {
        if let Some(r) = recent {
            crate::view::run_durations(r)
        } else {
            let r = state.store.list_recent_ping_summaries(check_id, 40).await?;
            crate::view::run_durations(&r)
        }
    } else {
        // A filtered or paged view pairs within its own slice, which is made
        // of full rows — project them down to what the pairing reads (four
        // `Copy` fields; no body clone).
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
    /// `Some` when editing an existing channel rather than creating one —
    /// drives the heading, the form action, the static (immutable) kind, and
    /// which config block is rendered.
    edit: Option<ChannelEditView>,
}

/// The non-secret half of a stored channel config, plus a `configured` flag per
/// secret field. Constructing this is the **only** way the edit template sees a
/// stored config, so a delivery secret cannot reach the rendered page even by
/// accident — non-leakage is a property of the type rather than of template
/// discipline (`ChannelDto` keeps the same invariant for the API; see
/// `src/api/dto.rs`).
///
/// Which fields count as secret is a judgement call, not a mechanical one: a
/// webhook or Slack URL *is* the capability to post to that room, so it is
/// treated as a secret even though it reads like an address. A telegram chat
/// id, an ntfy server/topic, and an email recipient are identifiers and are
/// safe to pre-fill.
struct ChannelEditView {
    id: i64,
    kind: &'static str,
    name: String,
    // -- pre-filled, non-secret --
    telegram_chat_id: String,
    ntfy_base_url: String,
    ntfy_topic: String,
    email_to: String,
    // -- rendered as a configured/not-set pill, never as a value. Webhook and
    //    Slack both store their URL under `url`, and every bot/app token under
    //    `token`; only the block for `kind` is ever rendered, so the flags do
    //    not collide in the page.
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
    /// Ignored when editing — a channel's kind is immutable, see
    /// [`crate::store::Store::update_channel`].
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
    /// Explicit clear for the one *optional* secret. Blank-means-unchanged (see
    /// [`validate_channel_update`]) would otherwise make a stored ntfy token
    /// impossible to remove through the edit form.
    #[serde(default)]
    pub(crate) ntfy_token_clear: bool,
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
    validate_channel_update(form, None)
}

/// [`validate_channel`] generalized over an optional `existing` channel the
/// form is editing.
///
/// One rule governs every field: **a blank submission keeps the stored value.**
/// That is what lets the edit form render each secret as an empty
/// `placeholder="unchanged"` input instead of printing the stored one back into
/// the page (see [`ChannelEditView`]), while still reusing the exact same
/// per-kind required-field checks as create — a required secret that is blank
/// *and* unset is still an error. The one escape hatch is
/// `ChannelForm::ntfy_token_clear`, for the single optional secret.
///
/// `existing.kind` always wins over the submitted `kind`: the kind is immutable
/// once created (the edit form renders it as static text) because a stored
/// config only has meaning for the kind that wrote it.
pub(crate) fn validate_channel_update(
    form: &ChannelForm,
    existing: Option<&Channel>,
) -> Result<(ChannelKind, String, String), String> {
    // A config that fails to parse is treated as "nothing stored", so a
    // corrupt row degrades to the create rules (every required field must be
    // submitted) rather than failing the edit outright.
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

/// Build the create/edit channel form. `edit` is `None` for a create, `Some`
/// for an edit — the only difference between the two surfaces, so both go
/// through here and a new template field is wired up once.
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

/// Shared edit-channel core: merge the submitted form over the stored config
/// (a blank field keeps its stored value, see [`validate_channel_update`]),
/// re-render the form on a validation error, else update and redirect to the
/// project page. The channel's kind is not touched.
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
            // Re-render from the *stored* channel, not from the rejected
            // submission: the secrets the user typed are deliberately not
            // echoed back into the page.
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

/// Resolve a channel the signed-in user owns (ownership derived from its
/// project), 404 for anyone else's — same existence-hiding rule as
/// [`owned_project`] / [`owned_check`].
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

/// Send a one-off test notification to a single channel. Sends once (no retry)
/// and does not record the attempt in the notification history.
async fn run_channel_test(state: &AppState, channel: &Channel) -> TestResult {
    // A test names the channel, not a check, so the only context worth
    // carrying is the project it belongs to.
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

/// How a settings field is validated. The settings table stores strings, so
/// each variant only decides what a *valid* value looks like before it is
/// written back as one.
#[derive(Clone, Copy)]
enum SettingKind {
    /// Raw seconds or a human duration (`5m`, `1h30m`).
    Duration,
    /// A plain positive integer count of days.
    Days,
    /// An IANA timezone name.
    Timezone,
}

/// Render a validated optional number as its stored form — blank clears the
/// setting, which is how every numeric setting spells "unset".
fn fmt_opt_setting(v: Option<i64>) -> String {
    v.map(|n| n.to_string()).unwrap_or_default()
}

/// Instance-wide display timezone: blank means unset (fall back to the check's
/// own timezone), unlike a check's own field where blank means UTC.
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

/// The settings-form fields to render on the merged `/admin` page: either the
/// four persisted values (the default path) or the raw values just submitted
/// to an invalid save, so the user can see and fix what they typed.
struct SettingsFields {
    scan_interval: String,
    nag_interval: String,
    pings_retention_days: String,
    notifications_retention_days: String,
    audit_retention_days: String,
    display_timezone: String,
}

/// The re-render inputs `render_admin` needs beyond the data it always
/// gathers itself (overview stats, users, projects): the settings section's
/// fields/error/flash, the users section's flash(es), and the add-user
/// form's error.
struct AdminRender {
    settings: SettingsFields,
    settings_error: Option<String>,
    settings_flash: Option<String>,
    user_flash: Option<String>,
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
    // Chain every surface through the same jar: the cookie is path-scoped to
    // "/", so each `take_flash`/`take_password_reset_keys_flash` call only
    // consumes it if the value matches its own surface, leaving it for the
    // next to check.
    let (jar, settings_flash) = take_flash(&state.config, jar, "settings");
    let (jar, user_flash) = take_flash(&state.config, jar, "users_blocked");
    let (jar, password_reset_flash) = take_password_reset_keys_flash(&state.config, jar);
    let resp = render_admin(
        &state,
        &jar,
        admin.id,
        AdminRender {
            settings,
            settings_error: None,
            settings_flash,
            user_flash,
            password_reset_flash,
            user_error: None,
        },
        &audit,
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
    // Atomic: validate every field before writing any. Blank clears the
    // setting; scan/nag intervals accept a duration (raw seconds or e.g.
    // `5m`), the retention fields are plain positive integers (days), and the
    // display timezone is an IANA name. Any non-blank invalid value aborts the
    // whole save and re-renders with the submitted values. Each field is
    // reduced to the string that will be stored, so the change-detection and
    // write passes below stay type-agnostic.
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
                    admin.id,
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
                        settings_flash: None,
                        user_flash: None,
                        password_reset_flash: None,
                        user_error: None,
                    },
                    // A rejected settings save re-renders the page; the audit
                    // card just comes back on its default latest page.
                    &AdminAuditQuery::default(),
                )
                .await?;
                return Ok(resp);
            }
        }
    }
    // Record what the save is about to change *before* writing, so `detail`
    // names the fields the operator actually touched rather than re-reading
    // them back afterwards.
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
    // A settings save is an admin action on the whole instance and had been
    // going unrecorded. It matters most for `audit_retention_days`: shortening
    // the window is how an admin would erase their own trail, and this is what
    // leaves a mark when they do. A no-op save writes nothing.
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

async fn users_create(
    State(state): State<AppState>,
    jar: CookieJar,
    AdminUser(admin): AdminUser,
    Form(form): Form<NewUserForm>,
) -> Result<Response, AppError> {
    if form.username.trim().is_empty() || form.password.is_empty() {
        let settings = load_settings_fields(&state).await?;
        let resp = render_admin(
            &state,
            &jar,
            admin.id,
            AdminRender {
                settings,
                settings_error: None,
                settings_flash: None,
                user_flash: None,
                password_reset_flash: None,
                user_error: Some("username and password are required".into()),
            },
            &AdminAuditQuery::default(),
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
        return Ok(users_blocked(&state.config, jar));
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
        return Ok(users_blocked(&state.config, jar));
    }
    state.store.delete_user(id).await?;
    // No `count` field here: the user's session rows go via the FK's ON
    // DELETE CASCADE, not a direct DELETE this handler issues, so no row
    // count is available to log.
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
    if form.password.is_empty() {
        return Ok(Redirect::to("/admin").into_response());
    }
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/admin").into_response());
    };
    let phc = hash_password(&form.password).map_err(|e| AppError::Other(e.to_string().into()))?;
    state.store.set_user_password(id, &phc).await?;
    // OWASP: a password change is a privilege level change, so existing
    // sessions must be invalidated — otherwise resetting a password to evict
    // an intruder leaves the intruder's cookie working. When the admin resets
    // their *own* password (the reset form is not hidden behind `is_self` in
    // `templates/admin.html`), the session they are currently operating from
    // must survive — see `Store::delete_sessions_for_user`'s doc comment.
    // Consequence: a self-targeted reset no longer evicts an attacker sharing
    // that same session row (e.g. a shoulder-surfed or exported cookie) — it
    // keeps the row exactly as `/account`'s "revoke others" does, and `logout`
    // only ever deletes the row for the browser issuing it. Evicting that
    // attacker therefore takes two steps: reset your password, then log out.
    // This handler also does not touch the target's API keys, unlike
    // `users_set_disabled` (covered because `api::extract::ApiUser` re-checks
    // `disabled` on every request): a password reset revokes sessions only, so
    // a `pw_…` key minted before the reset keeps working indefinitely, and
    // evicting it requires revoking it from `/account` or disabling the
    // account instead. That same re-check is why the residual-access flash
    // below is suppressed for a target who is already disabled: their keys
    // are already inert, so the warning would name access that does not
    // exist.
    let revoked = if id == admin.id {
        match secret::session_id_from_jar(&jar, &state.config.secret, session_cookie_name(&state)) {
            Some(current) => {
                state
                    .store
                    .delete_other_sessions_for_user(id, &current)
                    .await?
            }
            // Should not happen for an authenticated AdminUser, but fail safe
            // rather than leave stale sessions behind.
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
    // Sessions are revoked above, but API keys are not (see the doc comment
    // above) — a `pw_…` key minted before the reset keeps working
    // indefinitely. Surface that gap instead of leaving it silent: if the
    // target still has at least one key, flash a warning naming the residual
    // access and where to close it.
    // Count only keys that still resolve: `validate_api_key` already refuses
    // an expired key, so including one here would claim residual access that
    // does not exist.
    let now = Utc::now();
    let key_count = state
        .store
        .list_api_keys_for_user(id)
        .await?
        .iter()
        .filter(|k| k.expires_at.is_none_or(|e| e > now))
        .count() as u64;
    // A disabled account's keys are already inert: `api::extract::ApiUser`
    // re-checks `disabled` on every request, so warning about residual access
    // here would name access that does not exist — and point at a remedy
    // (disable the account) that is already in effect.
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
) -> Result<Response, AppError> {
    // Never allow revoking your own admin rights — it would lock you out of
    // `/admin` immediately (the very next request re-resolves AdminUser and
    // 403s), mirroring the self-guards in `users_delete`/`users_set_disabled`.
    if id == admin.id {
        return Ok(users_blocked(&state.config, jar));
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
        return Ok(users_blocked(&state.config, jar));
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
        return Ok(users_blocked(&state.config, jar));
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
        return Ok(users_blocked(&state.config, jar));
    }
    state.store.set_user_disabled(id, new_disabled).await?;
    // Only delete in the "disable" direction. Enabling has no sessions to
    // delete; more importantly, not deleting would let "disable then enable"
    // resurrect every old session (including one on a stolen device), because
    // `resolve_user`'s disabled check only blocks *while* disabled.
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
    let current_handle =
        secret::session_id_from_jar(jar, &state.config.secret, session_cookie_name(state))
            .map(|id| crate::apikey::hash_api_key(&id));
    // Reap this user's past-the-absolute-cap rows before listing. They are
    // already inert, but leaving them in the table until the next prune pass
    // means the owner can neither see nor revoke them; deleting them here is
    // what makes "not listed" mean "gone".
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
        // Must carry `path("/")` to match how the cookie was set — a
        // pathless removal cookie gets this route's own path
        // (`/account/sessions/{handle}/revoke`) and would not clear a
        // `path=/` cookie.
        let jar = jar.remove(session_removal_cookie(&state.config));
        return Ok((jar, Redirect::to("/login")).into_response());
    }
    Ok((jar, Redirect::to("/account")).into_response())
}

async fn sessions_revoke_others(
    State(state): State<AppState>,
    jar: CookieJar,
    CurrentUser(user): CurrentUser,
) -> Result<Response, AppError> {
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
/// Cursor + filter params for the `/admin` audit table. Prefixed `a*` so they
/// share the page's query string without colliding with anything else, and
/// every field is optional — an unknown or malformed value falls back to the
/// unfiltered latest page rather than 400ing the whole admin page. Accepted
/// both on `GET /admin` (full page) and `GET /admin/audit` (fragment).
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

/// The audit-trail fragment: filter controls + table + keyset pager. Served
/// standalone by `GET /admin/audit` (JS swaps it into `#audit-section`) and
/// inlined into the merged `/admin` page — the same two-surface arrangement
/// the check page's pings/notifications fragments use.
#[derive(Template)]
#[template(path = "admin_audit.html")]
struct AdminAuditTemplate {
    rows: Vec<AuditRow>,
    empty: bool,
    /// Every actor/action present in the trail, for the two filter selects.
    actors: Vec<String>,
    actions: Vec<String>,
    /// Selected filter values (`""` = all), echoed back into the controls.
    f_actor: String,
    f_action: String,
    f_from: String,
    f_to: String,
    /// Any filter is active — switches the empty state's wording and shows
    /// the Clear link.
    filtered: bool,
    newer: Option<String>,
    older: Option<String>,
}

/// One rendered audit row. The four columns are the "who did what to what,
/// when" summary; `method_path`, `detail` and `target_owner` are the rest of
/// `models::AuditLog`, carried in an expandable row (the ping table's
/// captured-output pattern) so nothing written to the table is unreachable.
struct AuditRow {
    time: String,
    iso: String,
    actor: String,
    action: String,
    target: String,
    method_path: String,
    detail: String,
    target_owner: String,
    /// This row has something behind the caret. False only when `method`,
    /// `path`, `detail` and `target_owner_id` are all unset, which would
    /// otherwise render a caret opening onto an empty box.
    expandable: bool,
}

#[derive(Template)]
#[template(path = "admin.html")]
struct AdminTemplate {
    show_nav: bool,
    csrf: String,
    is_admin: bool,
    /// The audit card body, from [`AdminAuditTemplate`]. Injected with `|safe`
    /// exactly like the check page's history fragments.
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
    password_reset_flash: Option<String>,
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
    audit: &AdminAuditQuery,
) -> Result<Response, AppError> {
    let day_ago = Utc::now() - Duration::days(1);
    // Rendered here rather than in the template so the inline card body and
    // the `/admin/audit` fragment endpoint emit byte-identical markup.
    let audit_partial = render(&build_audit_partial(state, audit).await?)?.0;
    let (notif_ok, notif_err) = state.store.notification_counts_since(day_ago).await?;
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
        last_scan_at: state.store.get_setting("last_scan_at").await?,
        last_prune_at: state.store.get_setting("last_prune_at").await?,
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
            .map(|u| UserRow::from_user(u, admin_id))
            .collect(),
        user_flash: r.user_flash,
        password_reset_flash: r.password_reset_flash,
        user_error: r.user_error,
        projects: state.store.list_all_projects_with_owner().await?,
        env_rows: env_settings(&state.config),
    })?
    .into_response())
}

/// Build the audit-trail fragment, honoring the `a*` filter and cursor params.
/// Mirrors [`build_pings_partial`]: parse filters, take one keyset page, render
/// rows, then build the two pager hrefs with the active filter carried along so
/// paging does not silently drop it.
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

/// `GET /admin/audit` — the audit fragment on its own, for the in-place
/// filter/pager swap. `AdminUser` guards it like every other `/admin` route.
async fn admin_audit_fragment(
    State(state): State<AppState>,
    AdminUser(_admin): AdminUser,
    Query(q): Query<AdminAuditQuery>,
) -> Result<Response, AppError> {
    Ok(render(&build_audit_partial(&state, &q).await?)?.into_response())
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

/// `POST /admin/checks/{id}/ping-url` — disclose a check's ping URL to an
/// admin who does not own it, and record that disclosure.
///
/// This is the one *read* under `/admin` that still audits, because it is the
/// one that hands over a credential rather than a description. It is a POST
/// rather than a query parameter on the page precisely so the disclosure
/// cannot happen without passing through here: a `?reveal=1` would be a way
/// to see the URL with nothing written down.
///
/// Re-submitting records the disclosure again, which is correct — it happened
/// again.
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
    // An admin revealing their own check's URL discloses nothing to anyone —
    // same condition that leaves the control unrendered in the first place.
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

    /// A typo used to be stored verbatim and then silently ignored — the cron
    /// simply fired on UTC's wall clock. The rejection is the only place the
    /// operator ever finds out.
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

    /// Blank means UTC, matching both the column default and the API's
    /// `default_timezone` — a form posted without the field must not error.
    #[test]
    fn validate_timezone_treats_blank_as_utc_and_canonicalizes() {
        assert_eq!(validate_timezone("").unwrap(), "UTC");
        assert_eq!(validate_timezone("   ").unwrap(), "UTC");
        assert_eq!(validate_timezone("Europe/Berlin").unwrap(), "Europe/Berlin");
        assert!(validate_timezone("Mars/Olympus").is_err());
    }

    /// The instance setting means something different by blank: unset, so the
    /// check's own zone still applies.
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
        // Blank (unset) and anything that is not a positive integer must survive
        // untouched so the user still sees exactly what is stored.
        assert_eq!(readable_setting_duration(String::new()), "");
        assert_eq!(readable_setting_duration("0".into()), "0");
        assert_eq!(readable_setting_duration("abc".into()), "abc");
    }

    /// A jar carrying the flash cookie exactly as a handler would set it —
    /// through the production builder, so the name and the signature are the
    /// real ones rather than a copy that can drift.
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
        // The cookie is path-scoped to "/", so the settings page also sees a
        // check-page flash. It must neither render nor consume it — the page it
        // was set for still gets it.
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
        // Even when the surface matches, an unknown key maps to no message, so a
        // user-supplied cookie value can never render as arbitrary text.
        let config = test_config();
        let jar = flash_jar(&config, "<script>");
        let (_, msg) = take_flash(&config, jar, "<script>");
        assert_eq!(msg, None);
    }

    /// A flash this origin never signed — what a sibling subdomain can write
    /// under plain HTTP, where no `__Host-` prefix is available — is ignored,
    /// for both the fixed-surface and the counts-carrying reader.
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

    /// The `__Host-` prefix is legal only on a `Secure`, `Path=/`,
    /// `Domain`-less cookie — assert the builder actually meets that contract
    /// wherever it uses the prefixed name.
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

        // …and the unprefixed name when it is not Secure, or the browser
        // would reject the cookie outright.
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
