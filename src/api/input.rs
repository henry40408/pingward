//! Request bodies for the write API. Each input is normalized into the
//! all-string web form structs ([`crate::web::ProjectForm`] etc.) so the write
//! handlers reuse the exact same validators as the browser UI
//! ([`crate::web::validate_project`] / [`crate::web::validate_check`] /
//! [`crate::web::validate_channel`]) — one source of truth for what a valid
//! project/check/channel is.

use crate::web::{ChannelForm, CheckForm, ProjectForm};
use serde::Deserialize;
use utoipa::ToSchema;

/// A duration field that accepts either a JSON string (`"5m"`, `"1h30m"`, or a
/// bare-seconds string `"90"`) or a JSON integer (raw seconds). Both normalize
/// to the raw string the web validators parse via `duration::parse_duration`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum DurationInput {
    Str(String),
    Int(i64),
}

impl DurationInput {
    fn into_form_string(self) -> String {
        match self {
            DurationInput::Str(s) => s,
            DurationInput::Int(n) => n.to_string(),
        }
    }
}

/// A blank string means "unset / inherit the default", exactly as an empty form
/// field does — `validate_*` maps it to `None`.
fn opt_form(v: Option<DurationInput>) -> String {
    v.map(DurationInput::into_form_string).unwrap_or_default()
}

/// Create/replace body for a project. On `PATCH` this is a full replacement of
/// the editable fields (mirrors the web edit form): send the complete
/// representation, not a partial patch.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ProjectInput {
    pub name: String,
    /// Per-project scan-interval override: seconds (int) or a duration string
    /// (`"5m"`). Omit or send `null` to inherit the global default.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "5m")]
    pub scan_interval_secs: Option<DurationInput>,
    /// Per-project nag-interval override: seconds (int) or a duration string.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "1h")]
    pub nag_interval_secs: Option<DurationInput>,
}

impl From<ProjectInput> for ProjectForm {
    fn from(i: ProjectInput) -> Self {
        ProjectForm {
            name: i.name,
            scan_interval_secs: opt_form(i.scan_interval_secs),
            nag_interval_secs: opt_form(i.nag_interval_secs),
        }
    }
}

fn default_schedule_kind() -> String {
    "period".to_string()
}

fn default_timezone() -> String {
    "UTC".to_string()
}

/// Create/replace body for a check. On `PATCH` this fully replaces the check's
/// editable fields (mirrors the web edit form) — send the complete
/// representation. The `ping_uuid`, status, and history are never set here.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckInput {
    pub name: String,
    /// Schedule type: `period` (default) or `cron`.
    #[serde(default = "default_schedule_kind")]
    pub schedule_kind: String,
    /// Expected interval between pings (period schedules): seconds or duration
    /// string. Required when `schedule_kind` is `period`.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "1h")]
    pub period_secs: Option<DurationInput>,
    /// The 6-field cron expression (`sec min hour dom mon dow`). Required when
    /// `schedule_kind` is `cron`.
    #[serde(default)]
    pub cron_expr: Option<String>,
    /// Grace past the deadline before the check is marked down: seconds or a
    /// duration string. Omitted defaults to `0`.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "5m")]
    pub grace_secs: Option<DurationInput>,
    /// IANA timezone the schedule is evaluated in (default `UTC`).
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// Per-check scan-interval override: seconds or duration string.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "30s")]
    pub scan_interval_secs: Option<DurationInput>,
    /// Max runtime after a `start` ping before the run is overdue: seconds or
    /// duration string.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "10m")]
    pub max_runtime_secs: Option<DurationInput>,
    /// Per-check nag/reminder-interval override: seconds or duration string.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "1h")]
    pub nag_interval_secs: Option<DurationInput>,
}

impl From<CheckInput> for CheckForm {
    fn from(i: CheckInput) -> Self {
        CheckForm {
            name: i.name,
            schedule_kind: i.schedule_kind,
            period_secs: opt_form(i.period_secs),
            cron_expr: i.cron_expr.unwrap_or_default(),
            // An omitted grace parses as 0 (`parse_duration("0")`), a valid
            // "no grace" — `validate_check` rejects a blank grace outright.
            grace_secs: i
                .grace_secs
                .map_or_else(|| "0".to_string(), DurationInput::into_form_string),
            timezone: i.timezone,
            scan_interval_secs: opt_form(i.scan_interval_secs),
            max_runtime_secs: opt_form(i.max_runtime_secs),
            nag_interval_secs: opt_form(i.nag_interval_secs),
        }
    }
}

/// Create body for a notification channel. The kind-specific credential fields
/// are flat and optional; `validate_channel` enforces exactly which are
/// required for the chosen `kind`.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ChannelInput {
    pub name: String,
    /// One of `webhook`, `slack`, `telegram`, `ntfy`, `pushover`, `email`.
    pub kind: String,
    #[serde(default)]
    pub webhook_url: String,
    #[serde(default)]
    pub slack_url: String,
    #[serde(default)]
    pub telegram_token: String,
    #[serde(default)]
    pub telegram_chat_id: String,
    /// ntfy server base URL (default `https://ntfy.sh`).
    #[serde(default)]
    pub ntfy_base_url: String,
    #[serde(default)]
    pub ntfy_topic: String,
    #[serde(default)]
    pub ntfy_token: String,
    #[serde(default)]
    pub pushover_token: String,
    #[serde(default)]
    pub pushover_user: String,
    #[serde(default)]
    pub email_to: String,
}

impl From<ChannelInput> for ChannelForm {
    fn from(i: ChannelInput) -> Self {
        ChannelForm {
            name: i.name,
            kind: i.kind,
            webhook_url: i.webhook_url,
            slack_url: i.slack_url,
            telegram_token: i.telegram_token,
            telegram_chat_id: i.telegram_chat_id,
            ntfy_base_url: i.ntfy_base_url,
            ntfy_topic: i.ntfy_topic,
            ntfy_token: i.ntfy_token,
            pushover_token: i.pushover_token,
            pushover_user: i.pushover_user,
            email_to: i.email_to,
        }
    }
}

/// Replace the set of channels bound to a check with exactly these ids. Ids
/// that do not belong to the check's own project are ignored.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ChannelBindInput {
    #[serde(default)]
    pub channel_ids: Vec<i64>,
}
