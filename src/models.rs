use chrono::{DateTime, Utc};
use std::str::FromStr;

macro_rules! str_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }
        }
        impl FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, String> {
                match s { $($text => Ok(Self::$variant),)+ other => Err(format!("invalid {}: {other}", stringify!($name))) }
            }
        }
    };
}

str_enum!(ScheduleKind { Period => "period", Cron => "cron" });
str_enum!(CheckStatus { New => "new", Up => "up", Down => "down", Paused => "paused" });
str_enum!(PingKind { Success => "success", Fail => "fail", Start => "start", Log => "log", Exitcode => "exitcode" });
str_enum!(ChannelKind { Webhook => "webhook", Telegram => "telegram", Slack => "slack", Ntfy => "ntfy", Pushover => "pushover", Email => "email" });
str_enum!(NotifyStatus { Ok => "ok", Error => "error" });

#[derive(Debug, Clone)]
pub struct Check {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub ping_uuid: String,
    pub schedule_kind: ScheduleKind,
    pub period_secs: Option<i64>,
    pub grace_secs: i64,
    pub cron_expr: Option<String>,
    pub timezone: String,
    pub status: CheckStatus,
    pub last_ping_at: Option<DateTime<Utc>>,
    pub last_start_at: Option<DateTime<Utc>>,
    pub next_due_at: Option<DateTime<Utc>>,
    pub scan_interval_secs: Option<i64>,
    pub max_runtime_secs: Option<i64>,
    pub nag_interval_secs: Option<i64>,
    pub last_alert_at: Option<DateTime<Utc>>,
    pub acknowledged: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: Option<String>,
    pub is_admin: bool,
    pub disabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub scan_interval_secs: Option<i64>,
    pub nag_interval_secs: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub id: i64,
    pub project_id: i64,
    pub kind: ChannelKind,
    pub name: String,
    pub config_json: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Ping {
    pub id: i64,
    pub check_id: i64,
    pub kind: PingKind,
    pub exit_code: Option<i64>,
    pub body: String,
    pub source_ip: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: i64,
    pub check_id: i64,
    pub channel_id: i64,
    pub event: crate::notify::EventKind,
    pub status: NotifyStatus,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A stored API key's metadata. The secret token itself is never held here —
/// only its SHA-256 hash lives in the database, and the plaintext is shown once
/// at creation. `prefix` is a non-secret display fragment.
#[derive(Debug, Clone)]
pub struct ApiKey {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct AuditLog {
    pub id: i64,
    pub actor_user_id: i64,
    pub actor_username: String,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<i64>,
    pub target_owner_id: Option<i64>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub detail: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn status_roundtrips_through_text() {
        for s in [
            CheckStatus::New,
            CheckStatus::Up,
            CheckStatus::Down,
            CheckStatus::Paused,
        ] {
            assert_eq!(CheckStatus::from_str(s.as_str()).unwrap(), s);
        }
    }

    #[test]
    fn unknown_status_is_error() {
        assert!(CheckStatus::from_str("bogus").is_err());
    }

    #[test]
    fn channel_kind_roundtrips() {
        for k in [
            ChannelKind::Webhook,
            ChannelKind::Telegram,
            ChannelKind::Slack,
            ChannelKind::Ntfy,
            ChannelKind::Pushover,
            ChannelKind::Email,
        ] {
            assert_eq!(ChannelKind::from_str(k.as_str()).unwrap(), k);
        }
        assert_eq!(ChannelKind::from_str("email").unwrap(), ChannelKind::Email);
        assert!(ChannelKind::from_str("carrier-pigeon").is_err());
    }

    #[test]
    fn notify_status_roundtrips() {
        assert_eq!(NotifyStatus::from_str("ok").unwrap(), NotifyStatus::Ok);
        assert_eq!(
            NotifyStatus::from_str("error").unwrap(),
            NotifyStatus::Error
        );
    }
}
