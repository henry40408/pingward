//! Serialization DTOs for the programmatic API. Kept separate from
//! [`crate::models`] so serde/utoipa derives never leak onto the domain types
//! (whose string-backed enums are not `Serialize`). Each DTO owns a
//! `From<Model>` and renders enum fields via their `as_str()` text.
//!
//! Note: [`ChannelDto`] deliberately omits `config_json` — a channel's config
//! holds delivery secrets (webhook URLs, bot tokens, SMTP creds), which must
//! never cross the API boundary.

use crate::models::{ApiKey, Channel, Check, Notification, Ping, Project};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct ProjectDto {
    pub id: i64,
    /// Id of the user who owns this project.
    pub owner_id: i64,
    #[schema(example = "Backups")]
    pub name: String,
    /// Raw markdown (the minimal subset in `src/markdown.rs`) — never
    /// rendered to HTML server-side; the API consumer decides how (or
    /// whether) to render it.
    #[schema(example = "Nightly **offsite** backup jobs.")]
    pub description: String,
    /// Per-project scan-interval override in seconds, if set.
    pub scan_interval_secs: Option<i64>,
    /// Per-project nag-interval override in seconds, if set.
    pub nag_interval_secs: Option<i64>,
    pub created_at: DateTime<Utc>,
}

impl From<Project> for ProjectDto {
    fn from(p: Project) -> Self {
        Self {
            id: p.id,
            owner_id: p.user_id,
            name: p.name,
            description: p.description,
            scan_interval_secs: p.scan_interval_secs,
            nag_interval_secs: p.nag_interval_secs,
            created_at: p.created_at,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct CheckDto {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    /// Raw markdown (the minimal subset in `src/markdown.rs`) — never
    /// rendered to HTML server-side; the API consumer decides how (or
    /// whether) to render it.
    #[schema(example = "Runs nightly at 02:00 **UTC**.")]
    pub description: String,
    /// The per-check UUID embedded in this check's ping URL.
    #[schema(example = "b1946ac9-2f8a-4e6d-9c3b-6f0e2d1a7c55")]
    pub ping_uuid: String,
    /// One of `new`, `up`, `down`, `paused`.
    #[schema(example = "up")]
    pub status: String,
    /// Schedule type: `period` or `cron`.
    #[schema(example = "period")]
    pub schedule_kind: String,
    /// Expected interval between pings, in seconds (period schedules).
    pub period_secs: Option<i64>,
    /// Grace period past the deadline before the check is marked down, seconds.
    pub grace_secs: i64,
    /// The 6-field cron expression (cron schedules).
    pub cron_expr: Option<String>,
    /// IANA timezone the schedule is evaluated in.
    pub timezone: String,
    pub last_ping_at: Option<DateTime<Utc>>,
    pub last_start_at: Option<DateTime<Utc>>,
    pub next_due_at: Option<DateTime<Utc>>,
    /// Whether the current down incident has been acknowledged.
    pub acknowledged: bool,
    pub created_at: DateTime<Utc>,
}

impl From<Check> for CheckDto {
    fn from(c: Check) -> Self {
        Self {
            id: c.id,
            project_id: c.project_id,
            name: c.name,
            description: c.description,
            ping_uuid: c.ping_uuid,
            status: c.status.as_str().to_string(),
            schedule_kind: c.schedule_kind.as_str().to_string(),
            period_secs: c.period_secs,
            grace_secs: c.grace_secs,
            cron_expr: c.cron_expr,
            timezone: c.timezone,
            last_ping_at: c.last_ping_at,
            last_start_at: c.last_start_at,
            next_due_at: c.next_due_at,
            acknowledged: c.acknowledged,
            created_at: c.created_at,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct ChannelDto {
    pub id: i64,
    pub project_id: i64,
    /// Channel type: `webhook`, `telegram`, `slack`, `ntfy`, `pushover`, `email`.
    #[schema(example = "webhook")]
    pub kind: String,
    #[schema(example = "On-call webhook")]
    pub name: String,
    pub created_at: DateTime<Utc>,
}

impl From<Channel> for ChannelDto {
    fn from(c: Channel) -> Self {
        Self {
            id: c.id,
            project_id: c.project_id,
            kind: c.kind.as_str().to_string(),
            name: c.name,
            created_at: c.created_at,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct PingDto {
    pub id: i64,
    pub check_id: i64,
    /// One of `success`, `fail`, `start`, `log`, `exitcode`.
    pub kind: String,
    /// The reported exit code (for `exitcode` pings).
    pub exit_code: Option<i64>,
    /// The request body captured with the ping (truncated at ingest).
    pub body: String,
    pub source_ip: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<Ping> for PingDto {
    fn from(p: Ping) -> Self {
        Self {
            id: p.id,
            check_id: p.check_id,
            kind: p.kind.as_str().to_string(),
            exit_code: p.exit_code,
            body: p.body,
            source_ip: p.source_ip,
            created_at: p.created_at,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct NotificationDto {
    pub id: i64,
    pub check_id: i64,
    pub channel_id: i64,
    /// The notified event: `up`, `down`, `reminder`, or `test`.
    pub event: String,
    /// Delivery result: `ok` or `error`.
    pub status: String,
    /// The failure detail when `status` is `error`.
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<Notification> for NotificationDto {
    fn from(n: Notification) -> Self {
        Self {
            id: n.id,
            check_id: n.check_id,
            channel_id: n.channel_id,
            event: n.event.as_str().to_string(),
            status: n.status.as_str().to_string(),
            error: n.error,
            created_at: n.created_at,
        }
    }
}

/// Metadata for one of the caller's API keys. The secret token is never
/// included — only the non-secret display `prefix`.
#[derive(Serialize, ToSchema)]
pub struct ApiKeyDto {
    pub id: i64,
    pub name: String,
    pub prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl From<ApiKey> for ApiKeyDto {
    fn from(k: ApiKey) -> Self {
        Self {
            id: k.id,
            name: k.name,
            prefix: k.prefix,
            created_at: k.created_at,
            last_used_at: k.last_used_at,
            expires_at: k.expires_at,
        }
    }
}

/// A keyset-paginated slice of pings, newest-first. `has_older`/`has_newer`
/// report whether adjacent pages exist; `next_before`/`next_after` carry the
/// boundary ids to fetch them — pass `next_before` as `?before=` for the next
/// (older) page and `next_after` as `?after=` for the previous (newer) page.
/// Each is `null` when there is no adjacent page in that direction.
#[derive(Serialize, ToSchema)]
pub struct PingPage {
    pub items: Vec<PingDto>,
    pub has_newer: bool,
    pub has_older: bool,
    /// Id to pass as `?before=` for the next (older) page; `null` if none.
    pub next_before: Option<i64>,
    /// Id to pass as `?after=` for the previous (newer) page; `null` if none.
    pub next_after: Option<i64>,
}

impl PingPage {
    /// Build the response envelope from a store [`crate::store::Page`], deriving
    /// the next/previous cursor ids from the (newest-first) item boundaries.
    pub fn from_page(page: crate::store::Page<Ping>) -> Self {
        let next_before = page
            .has_older
            .then(|| page.items.last().map(|p| p.id))
            .flatten();
        let next_after = page
            .has_newer
            .then(|| page.items.first().map(|p| p.id))
            .flatten();
        Self {
            has_newer: page.has_newer,
            has_older: page.has_older,
            next_before,
            next_after,
            items: page.items.into_iter().map(PingDto::from).collect(),
        }
    }
}

/// A keyset-paginated slice of notifications, newest-first. See [`PingPage`].
#[derive(Serialize, ToSchema)]
pub struct NotificationPage {
    pub items: Vec<NotificationDto>,
    pub has_newer: bool,
    pub has_older: bool,
    /// Id to pass as `?before=` for the next (older) page; `null` if none.
    pub next_before: Option<i64>,
    /// Id to pass as `?after=` for the previous (newer) page; `null` if none.
    pub next_after: Option<i64>,
}

impl NotificationPage {
    /// Build the response envelope from a store [`crate::store::Page`]. See
    /// [`PingPage::from_page`].
    pub fn from_page(page: crate::store::Page<Notification>) -> Self {
        let next_before = page
            .has_older
            .then(|| page.items.last().map(|n| n.id))
            .flatten();
        let next_after = page
            .has_newer
            .then(|| page.items.first().map(|n| n.id))
            .flatten();
        Self {
            has_newer: page.has_newer,
            has_older: page.has_older,
            next_before,
            next_after,
            items: page.items.into_iter().map(NotificationDto::from).collect(),
        }
    }
}

/// The channel ids bound to a check after a bind update.
#[derive(Serialize, ToSchema)]
pub struct BoundChannels {
    pub channel_ids: Vec<i64>,
}
