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
}
