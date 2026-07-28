use crate::duration::fmt_duration;
use chrono::{DateTime, Utc};
use std::future::Future;
use std::pin::Pin;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Down,
    Up,
    Reminder,
    Test,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::Down => "down",
            EventKind::Up => "up",
            EventKind::Reminder => "reminder",
            EventKind::Test => "test",
        }
    }
}

impl std::str::FromStr for EventKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "down" => Ok(EventKind::Down),
            "up" => Ok(EventKind::Up),
            "reminder" => Ok(EventKind::Reminder),
            "test" => Ok(EventKind::Test),
            other => Err(format!("invalid EventKind: {other}")),
        }
    }
}

/// Why a check went down. Carried on the event so a `DOWN` message can say
/// what actually happened: "nothing pinged" and "the job reported failure"
/// send the reader to very different places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownCause {
    /// No ping arrived before period/cron + grace elapsed.
    Overdue,
    /// A `start` ping was never followed by a completion within
    /// `max_runtime_secs`.
    Overrun {
        max_runtime_secs: i64,
        started_at: DateTime<Utc>,
    },
    /// An explicit `/fail` ping, or `/{code}` with a non-zero code.
    Failed { exit_code: Option<i64> },
}

impl DownCause {
    /// Stable machine name, used by the webhook payload.
    pub fn as_str(&self) -> &'static str {
        match self {
            DownCause::Overdue => "overdue",
            DownCause::Overrun { .. } => "overrun",
            DownCause::Failed { .. } => "failed",
        }
    }
}

/// Context rendered alongside the state change. Every field is optional: a
/// notification must still go out when a lookup fails, and
/// `EventDetail::default()` reproduces the original bare "<name> is DOWN".
///
/// It is built at the *call site*, from the check snapshot as it was when the
/// event fired, rather than re-read during delivery — for an `Up` event
/// `last_ping_at` is the ping *before* the recovery, which a re-read would
/// have already overwritten.
#[derive(Debug, Clone, Default)]
pub struct EventDetail {
    pub project_name: Option<String>,
    /// Absolute URL of the check page, derived from `PINGWARD_BASE_URL`.
    pub url: Option<String>,
    /// Human schedule summary, e.g. `every 5m (grace 1m)`.
    pub schedule: Option<String>,
    /// The check's last completion before this event.
    pub last_ping_at: Option<DateTime<Utc>>,
    /// The check's timezone; timestamps render in it, falling back to UTC.
    pub timezone: Option<String>,
    /// Set on `Down` only — `Reminder` fires long after the transition and
    /// `Up` has no cause to report.
    pub cause: Option<DownCause>,
}

impl EventDetail {
    /// Build from the check snapshot as it was when the event fired.
    pub fn from_check(check: &Check, project_name: Option<String>, base_url: &str) -> Self {
        Self {
            project_name,
            url: check_url(base_url, check.id),
            schedule: Some(schedule_summary(check)),
            last_ping_at: check.last_ping_at,
            timezone: Some(check.timezone.clone()),
            cause: None,
        }
    }

    pub fn with_cause(mut self, cause: DownCause) -> Self {
        self.cause = Some(cause);
        self
    }

    /// Override the rendering zone with the instance-wide display timezone
    /// when one is configured (`display_timezone`, set on `/admin`).
    ///
    /// A notification is the one surface with no browser to localise it — the
    /// web UI renders every absolute time in the *viewer's* zone — so an
    /// operator who reads alerts in one place can pin every timestamp to it.
    /// Blank keeps the check's own zone, which is what a cron schedule is
    /// written against.
    pub fn with_display_timezone(mut self, tz: Option<&str>) -> Self {
        if let Some(t) = tz.map(str::trim).filter(|t| !t.is_empty()) {
            self.timezone = Some(t.to_string());
        }
        self
    }
}

#[derive(Debug, Clone)]
pub struct NotificationEvent {
    pub check_id: i64,
    pub check_name: String,
    pub event: EventKind,
    pub at: DateTime<Utc>,
    pub project_id: i64,
    pub detail: EventDetail,
}

#[derive(Debug, thiserror::Error)]
#[error("notify failed: {0}")]
pub struct NotifyError(pub String);

/// Convert a reqwest transport error into a `NotifyError` without leaking the
/// request URL. reqwest's `Display` embeds the URL, which for Telegram carries
/// the bot token in its path; surfacing that in the failure banner (or the
/// stored notification error) would leak the secret. Report the error's
/// classification instead — the raw `Display` adds only the URL here anyway.
fn transport_err(e: &reqwest::Error) -> NotifyError {
    let kind = if e.is_timeout() {
        "request timed out"
    } else if e.is_connect() {
        "connection failed"
    } else if e.is_redirect() {
        "too many redirects"
    } else if e.is_body() || e.is_decode() {
        "invalid response"
    } else {
        "request failed"
    };
    NotifyError(kind.into())
}

pub trait Notifier: Send + Sync {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>>;
}

/// Shared reqwest client: a 10s request timeout keeps a hung endpoint from
/// blocking delivery forever. Falls back to a default client if the builder
/// fails (it never does with these options, but we avoid unwrap-panics).
fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Absolute URL of a check's page. `None` when no base URL is configured, so
/// the message simply omits the link instead of rendering `/checks/7`.
fn check_url(base_url: &str, check_id: i64) -> Option<String> {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    Some(format!("{base}/checks/{check_id}"))
}

/// One-line schedule summary — what the reader needs to judge how alarming a
/// missed check-in is. Reuses `duration::fmt_duration`, the same rendering the
/// edit forms use, so `300` reads as `5m` in both places.
fn schedule_summary(check: &Check) -> String {
    let grace = if check.grace_secs > 0 {
        format!(" (grace {})", fmt_duration(check.grace_secs))
    } else {
        String::new()
    };
    match check.schedule_kind {
        ScheduleKind::Period => match check.period_secs {
            Some(p) => format!("every {}{grace}", fmt_duration(p)),
            None => format!("no period set{grace}"),
        },
        ScheduleKind::Cron => match &check.cron_expr {
            Some(e) => format!("cron \"{e}\" {}{grace}", check.timezone),
            None => format!("no cron expression set{grace}"),
        },
    }
}

/// Render an instant in the check's timezone (UTC when unset or unparseable),
/// e.g. `2026-07-29 17:03 CST`. A notification is read away from the web UI,
/// where nothing localises the timestamp for the reader.
fn fmt_at(at: DateTime<Utc>, tz: Option<&str>) -> String {
    let zone: chrono_tz::Tz = tz.and_then(|t| t.parse().ok()).unwrap_or(chrono_tz::UTC);
    at.with_timezone(&zone)
        .format("%Y-%m-%d %H:%M %Z")
        .to_string()
}

/// `fmt_at` plus how long ago it was, when that is in the past.
fn fmt_at_rel(at: DateTime<Utc>, now: DateTime<Utc>, tz: Option<&str>) -> String {
    let secs = (now - at).num_seconds();
    if secs > 0 {
        format!("{} ({} ago)", fmt_at(at, tz), fmt_duration(secs))
    } else {
        fmt_at(at, tz)
    }
}

/// Replace control characters with spaces. Both the ntfy `Title` header and
/// the email `Subject` are single-line fields: a check or project name holding
/// a newline would make `HeaderValue` construction fail and abort the send.
fn single_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// `Project: infra · every 5m (grace 1m)` — omitted entirely when neither
/// half is known.
fn context_line(d: &EventDetail) -> Option<String> {
    match (&d.project_name, &d.schedule) {
        (Some(p), Some(s)) => Some(format!("Project: {p} · {s}")),
        (Some(p), None) => Some(format!("Project: {p}")),
        (None, Some(s)) => Some(format!("Schedule: {s}")),
        (None, None) => None,
    }
}

/// The one line that says what actually happened. `Reminder` deliberately
/// reports "Last ping" rather than "No ping since": it carries no `cause`, and
/// a check downed by a `fail` ping *did* ping.
fn reason_line(ev: &NotificationEvent) -> String {
    let d = &ev.detail;
    let tz = d.timezone.as_deref();
    match ev.event {
        EventKind::Up => match d.last_ping_at {
            Some(prev) => format!("Recovered; previous ping {}", fmt_at_rel(prev, ev.at, tz)),
            None => "Recovered".to_string(),
        },
        EventKind::Reminder => match d.last_ping_at {
            Some(prev) => format!("Last ping {}", fmt_at_rel(prev, ev.at, tz)),
            None => "Never pinged".to_string(),
        },
        _ => match d.cause {
            Some(DownCause::Overrun {
                max_runtime_secs,
                started_at,
            }) => format!(
                "Run started {} and exceeded its {} max runtime",
                fmt_at_rel(started_at, ev.at, tz),
                fmt_duration(max_runtime_secs)
            ),
            Some(DownCause::Failed { exit_code }) => match exit_code {
                Some(c) => format!("Fail ping received (exit code {c})"),
                None => "Fail ping received".to_string(),
            },
            _ => match d.last_ping_at {
                Some(prev) => format!("No ping since {}", fmt_at_rel(prev, ev.at, tz)),
                None => "No ping received yet".to_string(),
            },
        },
    }
}

/// Human summary of a state transition, reused by text-oriented channels
/// (Telegram, Slack, the ntfy body and the email body).
///
/// Deliberately capped at four short lines — headline, context, reason, link.
/// Everything past that is on the linked page, which is the point of the link.
fn event_text(ev: &NotificationEvent) -> String {
    let d = &ev.detail;
    let tz = d.timezone.as_deref();
    let mut lines: Vec<String> = Vec::new();
    match ev.event {
        EventKind::Test => {
            lines.push("\u{1F514} pingward test notification".to_string());
            lines.push(match &d.project_name {
                Some(p) => format!("Channel \"{}\" · project {p}", ev.check_name),
                None => format!("Channel \"{}\"", ev.check_name),
            });
            lines.push(fmt_at(ev.at, tz));
            return lines.join("\n");
        }
        EventKind::Down => lines.push(format!("\u{1F534} DOWN — {}", ev.check_name)),
        EventKind::Reminder => lines.push(format!("\u{1F534} STILL DOWN — {}", ev.check_name)),
        EventKind::Up => lines.push(format!("\u{1F7E2} UP — {}", ev.check_name)),
    }
    if let Some(ctx) = context_line(d) {
        lines.push(ctx);
    }
    lines.push(reason_line(ev));
    if let Some(url) = &d.url {
        lines.push(url.clone());
    }
    lines.join("\n")
}

/// Short title for channels with a separate title field (ntfy, Pushover, and
/// the email subject).
fn event_title(ev: &NotificationEvent) -> String {
    let name = single_line(&ev.check_name);
    let subject = match &ev.detail.project_name {
        Some(p) => format!("{}/{name}", single_line(p)),
        None => name,
    };
    match ev.event {
        EventKind::Test => format!("pingward: test notification for \"{subject}\""),
        EventKind::Down => format!("pingward: {subject} is DOWN"),
        EventKind::Up => format!("pingward: {subject} is UP"),
        EventKind::Reminder => format!("pingward: {subject} is STILL DOWN"),
    }
}

pub struct WebhookNotifier {
    url: String,
    client: reqwest::Client,
}

impl WebhookNotifier {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: http_client(),
        }
    }
}

impl Notifier for WebhookNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            // The original four keys are kept verbatim; everything else is
            // additive, so an existing consumer keeps parsing what it parsed.
            let d = &ev.detail;
            let exit_code = match d.cause {
                Some(DownCause::Failed { exit_code }) => exit_code,
                _ => None,
            };
            let body = serde_json::json!({
                "check": ev.check_name,
                "event": ev.event.as_str(),
                "at": ev.at.to_rfc3339(),
                "project_id": ev.project_id,
                "check_id": ev.check_id,
                "project": d.project_name,
                "url": d.url,
                "schedule": d.schedule,
                "timezone": d.timezone,
                "last_ping_at": d.last_ping_at.map(|t| t.to_rfc3339()),
                "cause": d.cause.map(|c| c.as_str()),
                "exit_code": exit_code,
                "text": event_text(ev),
            });
            let resp = self
                .client
                .post(&self.url)
                .json(&body)
                .send()
                .await
                .map_err(|e| transport_err(&e))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(NotifyError(format!("status {}", resp.status())))
            }
        })
    }
}

/// Telegram Bot API. `POST {base_url}/bot{token}/sendMessage` with a JSON
/// `{chat_id, text}` body. `base_url` is injectable so tests can point at a
/// mock server; production uses `https://api.telegram.org`.
pub struct TelegramNotifier {
    token: String,
    chat_id: String,
    base_url: String,
    client: reqwest::Client,
}

impl TelegramNotifier {
    pub fn new(token: String, chat_id: String) -> Self {
        Self::with_base_url(token, chat_id, "https://api.telegram.org".to_string())
    }

    pub fn with_base_url(token: String, chat_id: String, base_url: String) -> Self {
        Self {
            token,
            chat_id,
            base_url: base_url.trim_end_matches('/').to_string(),
            client: http_client(),
        }
    }
}

impl Notifier for TelegramNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}/bot{}/sendMessage", self.base_url, self.token);
            let body = serde_json::json!({
                "chat_id": self.chat_id,
                "text": event_text(ev),
            });
            let resp = self
                .client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| transport_err(&e))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(NotifyError(format!("status {}", resp.status())))
            }
        })
    }
}

/// Slack incoming webhook: `POST {url}` with a JSON `{text}` body.
pub struct SlackNotifier {
    url: String,
    client: reqwest::Client,
}

impl SlackNotifier {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: http_client(),
        }
    }
}

impl Notifier for SlackNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let body = serde_json::json!({ "text": event_text(ev) });
            let resp = self
                .client
                .post(&self.url)
                .json(&body)
                .send()
                .await
                .map_err(|e| transport_err(&e))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(NotifyError(format!("status {}", resp.status())))
            }
        })
    }
}

/// ntfy publish: `POST {base_url}/{topic}` with the message as the body and
/// `Title`/`Priority`/`Tags` headers. An optional bearer token authenticates
/// against protected topics / self-hosted servers.
pub struct NtfyNotifier {
    base_url: String,
    topic: String,
    token: Option<String>,
    client: reqwest::Client,
}

impl NtfyNotifier {
    pub fn new(base_url: String, topic: String, token: Option<String>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            topic,
            token: token.filter(|t| !t.is_empty()),
            client: http_client(),
        }
    }
}

impl Notifier for NtfyNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}/{}", self.base_url, self.topic);
            let (priority, tags) = match ev.event {
                EventKind::Down | EventKind::Reminder => ("high", "red_circle"),
                EventKind::Up => ("default", "green_circle"),
                EventKind::Test => ("default", "bell"),
            };
            let mut req = self
                .client
                .post(&url)
                .header("Title", event_title(ev))
                .header("Priority", priority)
                .header("Tags", tags)
                .body(event_text(ev));
            // `Click` makes tapping the notification open the check page.
            // Guarded on the URL being header-safe: a base URL carrying a
            // control character would otherwise fail `HeaderValue`
            // construction and abort the whole send.
            if let Some(u) = ev.detail.url.as_ref().filter(|u| {
                !u.is_empty() && !u.chars().any(|c| c.is_control() || c.is_whitespace())
            }) {
                req = req.header("Click", u);
            }
            if let Some(t) = &self.token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().await.map_err(|e| transport_err(&e))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(NotifyError(format!("status {}", resp.status())))
            }
        })
    }
}

/// Pushover: `POST {base_url}/1/messages.json` with a form body carrying the
/// app `token`, the recipient `user` key, and the `message`. `base_url` is
/// injectable so tests can point at a mock; production uses
/// `https://api.pushover.net`.
pub struct PushoverNotifier {
    token: String,
    user: String,
    base_url: String,
    client: reqwest::Client,
}

impl PushoverNotifier {
    pub fn new(token: String, user: String) -> Self {
        Self::with_base_url(token, user, "https://api.pushover.net".to_string())
    }

    pub fn with_base_url(token: String, user: String, base_url: String) -> Self {
        Self {
            token,
            user,
            base_url: base_url.trim_end_matches('/').to_string(),
            client: http_client(),
        }
    }
}

impl Notifier for PushoverNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}/1/messages.json", self.base_url);
            let priority = match ev.event {
                EventKind::Down | EventKind::Reminder => "1",
                EventKind::Up | EventKind::Test => "0",
            };
            let title = event_title(ev);
            let message = event_text(ev);
            let mut form = vec![
                ("token", self.token.as_str()),
                ("user", self.user.as_str()),
                ("title", title.as_str()),
                ("message", message.as_str()),
                ("priority", priority),
            ];
            // Pushover renders `url`/`url_title` as a tappable action, so the
            // link does not have to be re-read out of the message body.
            if let Some(u) = ev.detail.url.as_deref().filter(|u| !u.is_empty()) {
                form.push(("url", u));
                form.push(("url_title", "Open in pingward"));
            }
            let resp = self
                .client
                .post(&url)
                .form(&form)
                .send()
                .await
                .map_err(|e| transport_err(&e))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(NotifyError(format!("status {}", resp.status())))
            }
        })
    }
}

use crate::config::SmtpConfig;
use crate::config::SmtpTls;
use crate::models::{Channel, ChannelKind, Check, NotifyStatus, ScheduleKind};
use crate::store::Store;
use lettre::message::Message;
use lettre::transport::smtp::AsyncSmtpTransport;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncTransport, Tokio1Executor};

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_backoff: std::time::Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_backoff: std::time::Duration::from_millis(500),
        }
    }
}

/// Read a required non-empty string field from parsed channel config.
fn cfg_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Build the plain-text email for an event. Pure and panic-free: a malformed
/// address yields a `NotifyError` rather than panicking. Subject reuses
/// `event_title` (control-char sanitized); body is the one-line `event_text`.
fn build_email(from: &str, to: &str, ev: &NotificationEvent) -> Result<Message, NotifyError> {
    Message::builder()
        .from(
            from.parse()
                .map_err(|e| NotifyError(format!("invalid from address: {e}")))?,
        )
        .to(to
            .parse()
            .map_err(|e| NotifyError(format!("invalid recipient address: {e}")))?)
        .subject(event_title(ev))
        .body(event_text(ev))
        .map_err(|e| NotifyError(format!("failed to build email: {e}")))
}

/// Email via the instance SMTP relay. `smtp` is `None` when the relay is not
/// configured — `send` then reports a recorded delivery error rather than
/// silently dropping the alert.
pub struct EmailNotifier {
    smtp: Option<SmtpConfig>,
    to: String,
}

impl Notifier for EmailNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let smtp = self
                .smtp
                .as_ref()
                .ok_or_else(|| NotifyError("instance SMTP not configured".into()))?;
            let msg = build_email(&smtp.from, &self.to, ev)?;
            let builder = match smtp.tls {
                SmtpTls::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.host)
                    .map_err(|e| NotifyError(format!("smtp setup failed: {e}")))?,
                SmtpTls::Starttls => {
                    AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host)
                        .map_err(|e| NotifyError(format!("smtp setup failed: {e}")))?
                }
                SmtpTls::None => {
                    AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp.host)
                }
            };
            let mut builder = builder.port(smtp.port);
            if let (Some(u), Some(p)) = (&smtp.username, &smtp.password) {
                builder = builder.credentials(Credentials::new(u.clone(), p.clone()));
            }
            let transport = builder.build();
            transport
                .send(msg)
                .await
                .map_err(|e| NotifyError(format!("smtp send failed: {e}")))?;
            Ok(())
        })
    }
}

/// Build a notifier for a channel from its `(kind, config_json)`. Returns
/// `None` (with a warning) when a required config field is missing or blank —
/// `deliver_event` skips such channels rather than failing the event.
pub fn notifier_for(channel: &Channel, smtp: Option<&SmtpConfig>) -> Option<Box<dyn Notifier>> {
    let cfg: serde_json::Value = serde_json::from_str(&channel.config_json)
        .map_err(|e| {
            tracing::warn!(channel_id = channel.id, "invalid config_json: {e}");
        })
        .ok()?;
    let missing = |field: &str| {
        tracing::warn!(
            channel_id = channel.id,
            kind = channel.kind.as_str(),
            "channel missing required config field: {field}"
        );
        None::<Box<dyn Notifier>>
    };
    match channel.kind {
        ChannelKind::Webhook => match cfg_str(&cfg, "url") {
            Some(url) => Some(Box::new(WebhookNotifier::new(url))),
            None => missing("url"),
        },
        ChannelKind::Slack => match cfg_str(&cfg, "url") {
            Some(url) => Some(Box::new(SlackNotifier::new(url))),
            None => missing("url"),
        },
        ChannelKind::Telegram => match (cfg_str(&cfg, "token"), cfg_str(&cfg, "chat_id")) {
            (Some(token), Some(chat_id)) => Some(Box::new(TelegramNotifier::new(token, chat_id))),
            _ => missing("token/chat_id"),
        },
        ChannelKind::Ntfy => match cfg_str(&cfg, "topic") {
            Some(topic) => {
                let base_url =
                    cfg_str(&cfg, "base_url").unwrap_or_else(|| "https://ntfy.sh".to_string());
                let token = cfg_str(&cfg, "token");
                Some(Box::new(NtfyNotifier::new(base_url, topic, token)))
            }
            None => missing("topic"),
        },
        ChannelKind::Pushover => match (cfg_str(&cfg, "token"), cfg_str(&cfg, "user")) {
            (Some(token), Some(user)) => Some(Box::new(PushoverNotifier::new(token, user))),
            _ => missing("token/user"),
        },
        ChannelKind::Email => match cfg_str(&cfg, "to") {
            Some(to) => Some(Box::new(EmailNotifier {
                smtp: smtp.cloned(),
                to,
            })),
            None => missing("to"),
        },
    }
}

/// Send with bounded exponential-backoff retry. Returns the last error if all
/// attempts fail.
pub async fn send_with_retry(
    n: &dyn Notifier,
    ev: &NotificationEvent,
    policy: RetryPolicy,
) -> Result<(), NotifyError> {
    let mut last = NotifyError("no attempts".into());
    for attempt in 0..policy.max_attempts.max(1) {
        match n.send(ev).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = e;
                if attempt + 1 < policy.max_attempts {
                    let backoff = policy.base_backoff * 2u32.saturating_pow(attempt);
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
    Err(last)
}

/// Resolve the check's bound channels, deliver to each with retry, and record
/// every outcome in `notifications`. Delivery failures are recorded, never
/// propagated (spec §6: a failing channel must not affect state).
pub async fn deliver_event(
    store: &Store,
    ev: &NotificationEvent,
    policy: RetryPolicy,
    now: DateTime<Utc>,
    smtp: Option<&SmtpConfig>,
) {
    let channels = match store.channels_for_check(ev.check_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(check_id = ev.check_id, "failed to load channels: {e}");
            return;
        }
    };
    if channels.is_empty() {
        tracing::debug!(
            check = %ev.check_name,
            event = ev.event.as_str(),
            "no channels bound; nothing to deliver"
        );
        return;
    }
    for channel in &channels {
        let Some(notifier) = notifier_for(channel, smtp) else {
            continue;
        };
        let (status, error) = match send_with_retry(notifier.as_ref(), ev, policy).await {
            Ok(()) => (NotifyStatus::Ok, None),
            Err(e) => (NotifyStatus::Error, Some(e.to_string())),
        };
        if let Err(e) = store
            .record_notification(
                ev.check_id,
                channel.id,
                ev.event,
                status,
                error.as_deref(),
                now,
            )
            .await
        {
            tracing::error!(
                check_id = ev.check_id,
                channel_id = channel.id,
                "failed to record notification: {e}"
            );
        }
    }
}

pub async fn dispatch(
    notifiers: &[Box<dyn Notifier>],
    ev: &NotificationEvent,
) -> Vec<Result<(), NotifyError>> {
    let mut out = Vec::with_capacity(notifiers.len());
    for n in notifiers {
        out.push(n.send(ev).await);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn telegram_posts_sendmessage_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendMessage"))
            .and(body_string_contains("\"chat_id\":\"999\""))
            .and(body_string_contains("DOWN"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"ok\":true}"))
            .expect(1)
            .mount(&server)
            .await;

        let n = TelegramNotifier::with_base_url("123:ABC".into(), "999".into(), server.uri());
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        n.send(&ev).await.unwrap();
        // wiremock verifies expect(1) on drop
    }

    #[tokio::test]
    async fn telegram_returns_err_on_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("{\"ok\":false}"))
            .mount(&server)
            .await;
        let n = TelegramNotifier::with_base_url("bad".into(), "1".into(), server.uri());
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        assert!(n.send(&ev).await.is_err());
    }

    #[tokio::test]
    async fn webhook_posts_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let n = WebhookNotifier::new(format!("{}/hook", server.uri()));
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        n.send(&ev).await.unwrap();
        // wiremock verifies expect(1) on drop
    }

    /// A hung endpoint must not block delivery forever: the client's 10s
    /// timeout should fire and `send` should return `Err` well before the
    /// mock's 30s delay elapses. This test adds ~10s of real wall-clock time
    /// (reqwest's timer is real; tokio's paused clock does not apply to it).
    #[tokio::test]
    async fn webhook_send_times_out_on_hung_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(30)))
            .mount(&server)
            .await;

        let n = WebhookNotifier::new(format!("{}/hook", server.uri()));
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };

        let start = std::time::Instant::now();
        let result = n.send(&ev).await;
        let elapsed = start.elapsed();

        assert!(result.is_err(), "expected timeout to produce an error");
        assert!(
            elapsed < std::time::Duration::from_secs(20),
            "send took {elapsed:?}, expected the 10s client timeout to fire well before the 30s mock delay"
        );
    }

    #[tokio::test]
    async fn slack_posts_text_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/services/hook"))
            .and(body_string_contains("\"text\""))
            .and(body_string_contains("UP"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&server)
            .await;

        let n = SlackNotifier::new(format!("{}/services/hook", server.uri()));
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Up,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        n.send(&ev).await.unwrap();
    }

    #[tokio::test]
    async fn slack_returns_err_on_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let n = SlackNotifier::new(format!("{}/x", server.uri()));
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Up,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        assert!(n.send(&ev).await.is_err());
    }

    #[tokio::test]
    async fn ntfy_posts_body_with_headers_and_token() {
        use wiremock::matchers::header;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/mytopic"))
            .and(header("authorization", "Bearer tok"))
            .and(header("priority", "high"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"id\":\"x\"}"))
            .expect(1)
            .mount(&server)
            .await;

        let n = NtfyNotifier::new(server.uri(), "mytopic".into(), Some("tok".into()));
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        n.send(&ev).await.unwrap();
    }

    #[tokio::test]
    async fn ntfy_works_without_token_and_errors_on_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let n = NtfyNotifier::new(server.uri(), "t".into(), None);
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Up,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        assert!(n.send(&ev).await.is_err());
    }

    #[tokio::test]
    async fn telegram_send_error_does_not_leak_bot_token() {
        // A connection-level failure must not surface the request URL (which for
        // Telegram carries the bot token in its path) in the NotifyError shown
        // to the user. Point at a closed local port to force a connect error.
        let token = "123456:SECRETTOKENVALUE";
        let n = TelegramNotifier::with_base_url(
            token.into(),
            "1".into(),
            "http://127.0.0.1:1".to_string(),
        );
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        let err = n.send(&ev).await.unwrap_err();
        assert!(
            !err.to_string().contains("SECRETTOKENVALUE"),
            "bot token leaked into NotifyError: {err}"
        );
    }

    #[test]
    fn event_title_strips_control_characters() {
        // A check name with a newline/tab must not survive into the ntfy
        // `Title` header, or HeaderValue construction fails and aborts the send.
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "back\nup\tjob".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        let title = event_title(&ev);
        assert!(!title.chars().any(char::is_control));
        assert!(title.contains("back up job"));
    }

    #[tokio::test]
    async fn ntfy_send_succeeds_with_control_char_check_name() {
        // Regression guard: before sanitizing `event_title`, a check name
        // containing a control char made the `Title` header invalid and the
        // send returned Err. It must now succeed against a normal 200 server.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"id\":\"x\"}"))
            .expect(1)
            .mount(&server)
            .await;
        let n = NtfyNotifier::new(server.uri(), "topic".into(), None);
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "nightly\nbackup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        n.send(&ev).await.unwrap();
    }

    #[tokio::test]
    async fn pushover_posts_form_with_token_and_user() {
        use wiremock::matchers::body_string_contains;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/1/messages.json"))
            .and(body_string_contains("token=apptok"))
            .and(body_string_contains("user=userkey"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"status\":1}"))
            .expect(1)
            .mount(&server)
            .await;

        let n = PushoverNotifier::with_base_url("apptok".into(), "userkey".into(), server.uri());
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        n.send(&ev).await.unwrap();
    }

    #[tokio::test]
    async fn pushover_returns_err_on_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("{\"status\":0}"))
            .mount(&server)
            .await;
        let n = PushoverNotifier::with_base_url("bad".into(), "bad".into(), server.uri());
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Up,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        assert!(n.send(&ev).await.is_err());
    }

    fn channel_with(kind: ChannelKind, config_json: &str) -> Channel {
        Channel {
            id: 1,
            project_id: 1,
            kind,
            name: "c".into(),
            config_json: config_json.into(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn notifier_for_builds_each_kind_with_valid_config() {
        assert!(
            notifier_for(
                &channel_with(ChannelKind::Webhook, "{\"url\":\"http://x\"}"),
                None
            )
            .is_some()
        );
        assert!(
            notifier_for(
                &channel_with(ChannelKind::Slack, "{\"url\":\"http://x\"}"),
                None
            )
            .is_some()
        );
        assert!(
            notifier_for(
                &channel_with(ChannelKind::Telegram, "{\"token\":\"t\",\"chat_id\":\"1\"}"),
                None
            )
            .is_some()
        );
        assert!(
            notifier_for(
                &channel_with(
                    ChannelKind::Ntfy,
                    "{\"base_url\":\"https://ntfy.sh\",\"topic\":\"t\"}"
                ),
                None
            )
            .is_some()
        );
        assert!(
            notifier_for(
                &channel_with(ChannelKind::Pushover, "{\"token\":\"t\",\"user\":\"u\"}"),
                None
            )
            .is_some()
        );
    }

    #[test]
    fn notifier_for_returns_none_on_missing_config() {
        assert!(notifier_for(&channel_with(ChannelKind::Slack, "{}"), None).is_none());
        assert!(
            notifier_for(
                &channel_with(ChannelKind::Telegram, "{\"token\":\"t\"}"),
                None
            )
            .is_none()
        );
        assert!(
            notifier_for(
                &channel_with(ChannelKind::Ntfy, "{\"base_url\":\"x\"}"),
                None
            )
            .is_none()
        );
        assert!(
            notifier_for(
                &channel_with(ChannelKind::Pushover, "{\"token\":\"t\"}"),
                None
            )
            .is_none()
        );
    }

    use crate::db;
    use crate::models::ChannelKind;
    use crate::store::Store;

    async fn store_with_check_and_channel(url: &str) -> (Store, i64) {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool, "sqlite::memory:").await.unwrap();
        sqlx::query(
            "INSERT INTO users (username,is_admin,created_at) VALUES ('u',0,datetime('now'))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO projects (user_id,name,created_at) VALUES (1,'p',datetime('now'))",
        )
        .execute(&pool)
        .await
        .unwrap();
        let store = Store::new(pool);
        let now = Utc::now();
        let cid = store
            .create_channel(
                1,
                ChannelKind::Webhook,
                "hook",
                &format!("{{\"url\":\"{url}\"}}"),
                now,
            )
            .await
            .unwrap();
        let chk = store
            .create_check(&crate::store::NewCheck {
                project_id: 1,
                name: "job",
                ping_uuid: "u1",
                kind: crate::models::ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        store.bind_channel(chk, cid).await.unwrap();
        (store, chk)
    }

    #[tokio::test]
    async fn deliver_event_posts_and_records_ok() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let (store, chk) = store_with_check_and_channel(&server.uri()).await;
        let ev = NotificationEvent {
            check_id: chk,
            check_name: "job".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        deliver_event(&store, &ev, RetryPolicy::default(), Utc::now(), None).await;

        let recs = store.list_recent_notifications(chk, 10).await.unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].status, crate::models::NotifyStatus::Ok);
    }

    #[tokio::test]
    async fn deliver_event_records_error_when_channel_fails() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let (store, chk) = store_with_check_and_channel(&server.uri()).await;
        let ev = NotificationEvent {
            check_id: chk,
            check_name: "job".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        // fast policy so the test does not sleep for seconds
        let policy = RetryPolicy {
            max_attempts: 2,
            base_backoff: std::time::Duration::from_millis(1),
        };
        deliver_event(&store, &ev, policy, Utc::now(), None).await;

        let recs = store.list_recent_notifications(chk, 10).await.unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].status, crate::models::NotifyStatus::Error);
        assert!(recs[0].error.is_some());
    }

    #[test]
    fn build_email_sets_headers_and_builds() {
        let ev = NotificationEvent {
            check_id: 0,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        let msg = build_email("alerts@example.com", "ops@example.com", &ev).unwrap();
        let raw = String::from_utf8(msg.formatted()).unwrap();
        assert!(raw.contains("From: alerts@example.com"), "got: {raw}");
        assert!(raw.contains("To: ops@example.com"), "got: {raw}");
        assert!(
            raw.contains("Subject:") && raw.contains("pingward"),
            "got: {raw}"
        );
    }

    #[test]
    fn build_email_rejects_bad_address() {
        let ev = NotificationEvent {
            check_id: 0,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        assert!(build_email("not-an-address", "ops@example.com", &ev).is_err());
    }

    #[tokio::test]
    async fn email_notifier_errors_when_smtp_unconfigured() {
        let n = EmailNotifier {
            smtp: None,
            to: "ops@example.com".into(),
        };
        let ev = NotificationEvent {
            check_id: 0,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        let err = n.send(&ev).await.unwrap_err();
        assert!(
            err.to_string().contains("instance SMTP not configured"),
            "got: {err}"
        );
    }

    #[test]
    fn notifier_for_email_needs_recipient() {
        assert!(
            notifier_for(
                &channel_with(ChannelKind::Email, "{\"to\":\"ops@example.com\"}"),
                None
            )
            .is_some()
        );
        assert!(notifier_for(&channel_with(ChannelKind::Email, "{}"), None).is_none());
    }

    #[test]
    fn reminder_event_roundtrips_and_renders_still_down() {
        assert_eq!(EventKind::Reminder.as_str(), "reminder");
        assert_eq!(
            std::str::FromStr::from_str("reminder"),
            Ok(EventKind::Reminder)
        );
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Reminder,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        let text = event_text(&ev);
        assert!(text.contains("STILL DOWN"), "got: {text}");
        assert_eq!(event_title(&ev), "pingward: backup is STILL DOWN");
    }

    #[test]
    fn test_event_roundtrips_and_renders() {
        assert_eq!(EventKind::Test.as_str(), "test");
        assert_eq!(std::str::FromStr::from_str("test"), Ok(EventKind::Test));
        let ev = NotificationEvent {
            check_id: 0,
            check_name: "my-slack".into(),
            event: EventKind::Test,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        let text = event_text(&ev);
        assert!(text.contains("test notification"), "got: {text}");
        assert!(text.contains("my-slack"), "got: {text}");
        assert_eq!(
            event_title(&ev),
            "pingward: test notification for \"my-slack\""
        );
    }

    // --- message content -------------------------------------------------

    fn detail_check() -> Check {
        Check {
            id: 42,
            project_id: 1,
            name: "nightly-backup".into(),
            description: String::new(),
            ping_uuid: "u".into(),
            schedule_kind: ScheduleKind::Period,
            period_secs: Some(300),
            grace_secs: 60,
            cron_expr: None,
            timezone: "Asia/Taipei".into(),
            status: crate::models::CheckStatus::Up,
            last_ping_at: Some(Utc.with_ymd_and_hms(2026, 7, 29, 9, 3, 0).unwrap()),
            last_start_at: None,
            next_due_at: None,
            scan_interval_secs: None,
            max_runtime_secs: None,
            nag_interval_secs: None,
            last_alert_at: None,
            acknowledged: false,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    fn detailed_event(event: EventKind, cause: Option<DownCause>) -> NotificationEvent {
        let check = detail_check();
        let mut detail = EventDetail::from_check(
            &check,
            Some("infra".into()),
            "https://pingward.example.com/",
        );
        detail.cause = cause;
        NotificationEvent {
            check_id: check.id,
            check_name: check.name.clone(),
            event,
            at: Utc.with_ymd_and_hms(2026, 7, 29, 10, 8, 0).unwrap(),
            project_id: check.project_id,
            detail,
        }
    }

    #[test]
    fn down_text_names_project_schedule_reason_and_link() {
        let text = event_text(&detailed_event(EventKind::Down, Some(DownCause::Overdue)));
        assert_eq!(
            text,
            "\u{1F534} DOWN — nightly-backup\n\
             Project: infra · every 5m (grace 1m)\n\
             No ping since 2026-07-29 17:03 CST (1h5m ago)\n\
             https://pingward.example.com/checks/42"
        );
    }

    #[test]
    fn down_text_distinguishes_fail_ping_from_a_missed_check_in() {
        let text = event_text(&detailed_event(
            EventKind::Down,
            Some(DownCause::Failed { exit_code: Some(1) }),
        ));
        assert!(
            text.contains("Fail ping received (exit code 1)"),
            "got: {text}"
        );
        assert!(!text.contains("No ping since"), "got: {text}");
    }

    #[test]
    fn down_text_reports_an_overrun_run_with_its_budget() {
        let text = event_text(&detailed_event(
            EventKind::Down,
            Some(DownCause::Overrun {
                max_runtime_secs: 600,
                started_at: Utc.with_ymd_and_hms(2026, 7, 29, 9, 50, 0).unwrap(),
            }),
        ));
        assert!(
            text.contains(
                "Run started 2026-07-29 17:50 CST (18m ago) and exceeded its 10m max runtime"
            ),
            "got: {text}"
        );
    }

    #[test]
    fn up_text_reports_the_previous_ping_not_a_cause() {
        let text = event_text(&detailed_event(EventKind::Up, None));
        assert!(
            text.starts_with("\u{1F7E2} UP — nightly-backup"),
            "got: {text}"
        );
        assert!(
            text.contains("Recovered; previous ping 2026-07-29 17:03 CST (1h5m ago)"),
            "got: {text}"
        );
    }

    /// A reminder carries no cause, and a check downed by a `fail` ping *did*
    /// ping — so it says "Last ping", never "No ping since".
    #[test]
    fn reminder_text_reports_the_last_ping_neutrally() {
        let text = event_text(&detailed_event(EventKind::Reminder, None));
        assert!(
            text.starts_with("\u{1F534} STILL DOWN — nightly-backup"),
            "got: {text}"
        );
        assert!(
            text.contains("Last ping 2026-07-29 17:03 CST"),
            "got: {text}"
        );
        assert!(!text.contains("No ping since"), "got: {text}");
    }

    /// The whole point of the extra context is that it degrades: with nothing
    /// resolved the message is still the original one-liner plus a reason.
    #[test]
    fn text_degrades_to_headline_and_reason_without_detail() {
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
            detail: EventDetail::default(),
        };
        assert_eq!(
            event_text(&ev),
            "\u{1F534} DOWN — backup\nNo ping received yet"
        );
        assert_eq!(event_title(&ev), "pingward: backup is DOWN");
    }

    #[test]
    fn title_includes_the_project_and_stays_single_line() {
        let mut ev = detailed_event(EventKind::Down, Some(DownCause::Overdue));
        ev.check_name = "nightly\nbackup".into();
        ev.detail.project_name = Some("in\tfra".into());
        let title = event_title(&ev);
        assert_eq!(title, "pingward: in fra/nightly backup is DOWN");
        assert!(!title.chars().any(char::is_control));
    }

    /// The instance display timezone is what an operator who reads alerts in
    /// one place sets; it beats the check's own zone, which is written for the
    /// cron schedule rather than for the reader.
    #[test]
    fn display_timezone_overrides_the_checks_own_zone() {
        let mut ev = detailed_event(EventKind::Down, Some(DownCause::Overdue));
        assert!(event_text(&ev).contains("17:03 CST"));

        ev.detail = ev
            .detail
            .clone()
            .with_display_timezone(Some("Europe/Berlin"));
        let text = event_text(&ev);
        assert!(text.contains("11:03 CEST"), "got: {text}");
        assert!(!text.contains("CST"), "got: {text}");
    }

    #[test]
    fn a_blank_display_timezone_keeps_the_checks_zone() {
        let ev = detailed_event(EventKind::Down, Some(DownCause::Overdue));
        for blank in [None, Some(""), Some("   ")] {
            let detail = ev.detail.clone().with_display_timezone(blank);
            assert_eq!(detail.timezone.as_deref(), Some("Asia/Taipei"));
        }
    }

    #[test]
    fn no_base_url_means_no_link() {
        let check = detail_check();
        let detail = EventDetail::from_check(&check, None, "");
        assert_eq!(detail.url, None);
        assert_eq!(
            EventDetail::from_check(&check, None, "https://x.test")
                .url
                .as_deref(),
            Some("https://x.test/checks/42")
        );
    }

    #[test]
    fn schedule_summary_renders_both_kinds() {
        let mut check = detail_check();
        assert_eq!(schedule_summary(&check), "every 5m (grace 1m)");
        check.grace_secs = 0;
        assert_eq!(schedule_summary(&check), "every 5m");
        check.schedule_kind = ScheduleKind::Cron;
        check.cron_expr = Some("0 0 * * * *".into());
        assert_eq!(schedule_summary(&check), "cron \"0 0 * * * *\" Asia/Taipei");
    }

    /// The webhook payload's original four keys are load-bearing for existing
    /// consumers; everything richer is additive.
    #[tokio::test]
    async fn webhook_payload_keeps_old_keys_and_adds_context() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("\"check\":\"nightly-backup\""))
            .and(body_string_contains("\"event\":\"down\""))
            .and(body_string_contains("\"project_id\":1"))
            .and(body_string_contains("\"project\":\"infra\""))
            .and(body_string_contains(
                "\"url\":\"https://pingward.example.com/checks/42\"",
            ))
            .and(body_string_contains("\"cause\":\"failed\""))
            .and(body_string_contains("\"exit_code\":2"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let n = WebhookNotifier::new(server.uri());
        n.send(&detailed_event(
            EventKind::Down,
            Some(DownCause::Failed { exit_code: Some(2) }),
        ))
        .await
        .unwrap();
    }

    /// Tapping an ntfy notification should land on the check page.
    #[tokio::test]
    async fn ntfy_sets_click_header_to_the_check_url() {
        use wiremock::matchers::header;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(header("click", "https://pingward.example.com/checks/42"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"id\":\"x\"}"))
            .expect(1)
            .mount(&server)
            .await;
        let n = NtfyNotifier::new(server.uri(), "topic".into(), None);
        n.send(&detailed_event(EventKind::Down, Some(DownCause::Overdue)))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn pushover_sends_the_check_url_as_a_supplementary_action() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("url_title=Open+in+pingward"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"status\":1}"))
            .expect(1)
            .mount(&server)
            .await;
        let n = PushoverNotifier::with_base_url("t".into(), "u".into(), server.uri());
        n.send(&detailed_event(EventKind::Down, Some(DownCause::Overdue)))
            .await
            .unwrap();
    }
}
