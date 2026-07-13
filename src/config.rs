#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTls {
    Starttls,
    Tls,
    None,
}

#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
    pub tls: SmtpTls,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind: String,
    pub base_url: String,
    pub scan_interval_secs: u64,
    pub prune_interval_secs: u64,
    pub forward_auth_header: Option<String>,
    pub trusted_proxies: Vec<String>,
    pub smtp: Option<SmtpConfig>,
}

impl Config {
    pub fn from_env() -> Self {
        Self::from_map(|k| std::env::var(k).ok())
    }

    /// Testable core: `get` resolves an env key to an optional value.
    pub fn from_map(get: impl Fn(&str) -> Option<String>) -> Self {
        let scan_interval_secs = get("PINGWARD_SCAN_INTERVAL")
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        let prune_interval_secs = get("PINGWARD_PRUNE_INTERVAL_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600);
        let trusted_proxies = get("PINGWARD_TRUSTED_PROXIES")
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        // Instance SMTP: present only when both host and from are set. Any
        // partial config (host without from, etc.) means email is unavailable.
        let nonblank = |k: &str| {
            get(k)
                .map(|v| v.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let smtp = match (
            nonblank("PINGWARD_SMTP_HOST"),
            nonblank("PINGWARD_SMTP_FROM"),
        ) {
            (Some(host), Some(from)) => {
                let port = nonblank("PINGWARD_SMTP_PORT")
                    .and_then(|v| v.parse::<u16>().ok())
                    .unwrap_or(587);
                let tls = match nonblank("PINGWARD_SMTP_TLS")
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "tls" => SmtpTls::Tls,
                    "none" => SmtpTls::None,
                    _ => SmtpTls::Starttls,
                };
                Some(SmtpConfig {
                    host,
                    port,
                    username: nonblank("PINGWARD_SMTP_USERNAME"),
                    password: nonblank("PINGWARD_SMTP_PASSWORD"),
                    from,
                    tls,
                })
            }
            _ => None,
        };
        Config {
            database_url: get("DATABASE_URL")
                .unwrap_or_else(|| "sqlite://pingward.sqlite3?mode=rwc".into()),
            bind: get("PINGWARD_BIND").unwrap_or_else(|| "127.0.0.1:8080".into()),
            base_url: get("PINGWARD_BASE_URL").unwrap_or_else(|| "http://localhost:8080".into()),
            scan_interval_secs,
            prune_interval_secs,
            forward_auth_header: get("PINGWARD_FORWARD_AUTH_HEADER"),
            trusted_proxies,
            smtp,
        }
    }
}

/// Resolve the effective scan interval for a check using the spec §8 cascade:
/// check → project → global (DB settings) → env default. A `Some(v)` override
/// with `v <= 0` is treated as unset and falls through. The result is clamped
/// to at least 1 second so the scan loop's timer is always valid.
pub fn effective_scan_interval(
    check_secs: Option<i64>,
    project_secs: Option<i64>,
    global_secs: Option<i64>,
    env_default: u64,
) -> u64 {
    for v in [check_secs, project_secs, global_secs]
        .into_iter()
        .flatten()
    {
        if v > 0 {
            return v as u64;
        }
    }
    env_default.max(1)
}

/// Resolve the effective nag (repeat-notification) interval for a check from
/// the cascade: check → project → global. Returns `None` when nag is off at
/// every level (unset or non-positive). Unlike `effective_scan_interval`,
/// there is no env-default fallback — nag is opt-in.
pub fn effective_nag_interval(
    check_secs: Option<i64>,
    project_secs: Option<i64>,
    global_secs: Option<i64>,
) -> Option<i64> {
    [check_secs, project_secs, global_secs]
        .into_iter()
        .flatten()
        .find(|&v| v > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_apply_when_unset() {
        let c = Config::from_map(|_| None);
        assert_eq!(c.scan_interval_secs, 30);
        assert_eq!(c.bind, "127.0.0.1:8080");
        assert_eq!(c.database_url, "sqlite://pingward.sqlite3?mode=rwc");
    }

    #[test]
    fn env_overrides_defaults() {
        let c = Config::from_map(|k| match k {
            "PINGWARD_SCAN_INTERVAL" => Some("10".into()),
            "PINGWARD_TRUSTED_PROXIES" => Some("10.0.0.1,10.0.0.2".into()),
            _ => None,
        });
        assert_eq!(c.scan_interval_secs, 10);
        assert_eq!(c.trusted_proxies, vec!["10.0.0.1", "10.0.0.2"]);
    }

    #[test]
    fn prune_interval_defaults_and_overrides() {
        assert_eq!(Config::from_map(|_| None).prune_interval_secs, 3600);
        let c = Config::from_map(|k| (k == "PINGWARD_PRUNE_INTERVAL_SECS").then(|| "60".into()));
        assert_eq!(c.prune_interval_secs, 60);
    }

    #[test]
    fn cascade_prefers_most_specific() {
        // check wins
        assert_eq!(effective_scan_interval(Some(5), Some(10), Some(20), 30), 5);
        // project when no check
        assert_eq!(effective_scan_interval(None, Some(10), Some(20), 30), 10);
        // global when no check/project
        assert_eq!(effective_scan_interval(None, None, Some(20), 30), 20);
        // env default when nothing set
        assert_eq!(effective_scan_interval(None, None, None, 30), 30);
        // non-positive overrides are ignored
        assert_eq!(effective_scan_interval(Some(0), Some(-1), None, 30), 30);
        // result is clamped to >= 1 even if env default is 0
        assert_eq!(effective_scan_interval(None, None, None, 0), 1);
    }

    #[test]
    fn nag_cascade_prefers_most_specific_and_is_opt_in() {
        assert_eq!(effective_nag_interval(Some(5), Some(10), Some(20)), Some(5));
        assert_eq!(effective_nag_interval(None, Some(10), Some(20)), Some(10));
        assert_eq!(effective_nag_interval(None, None, Some(20)), Some(20));
        // opt-in: all unset → off (no env default)
        assert_eq!(effective_nag_interval(None, None, None), None);
        // non-positive levels are skipped
        assert_eq!(
            effective_nag_interval(Some(0), Some(-1), Some(30)),
            Some(30)
        );
        assert_eq!(effective_nag_interval(Some(0), None, None), None);
    }

    #[test]
    fn smtp_none_when_host_or_from_missing() {
        assert!(Config::from_map(|_| None).smtp.is_none());
        let c = Config::from_map(|k| (k == "PINGWARD_SMTP_HOST").then(|| "mail.x".into()));
        assert!(c.smtp.is_none());
        let c = Config::from_map(|k| (k == "PINGWARD_SMTP_FROM").then(|| "a@x".into()));
        assert!(c.smtp.is_none());
    }

    #[test]
    fn smtp_parsed_when_host_and_from_present() {
        let c = Config::from_map(|k| match k {
            "PINGWARD_SMTP_HOST" => Some("mail.example.com".into()),
            "PINGWARD_SMTP_FROM" => Some("alerts@example.com".into()),
            "PINGWARD_SMTP_PORT" => Some("2525".into()),
            "PINGWARD_SMTP_USERNAME" => Some("u".into()),
            "PINGWARD_SMTP_PASSWORD" => Some("p".into()),
            "PINGWARD_SMTP_TLS" => Some("TLS".into()),
            _ => None,
        });
        let s = c.smtp.expect("smtp should be Some");
        assert_eq!(s.host, "mail.example.com");
        assert_eq!(s.from, "alerts@example.com");
        assert_eq!(s.port, 2525);
        assert_eq!(s.username.as_deref(), Some("u"));
        assert_eq!(s.password.as_deref(), Some("p"));
        assert_eq!(s.tls, SmtpTls::Tls);
    }

    #[test]
    fn smtp_defaults_port_and_tls_and_optional_auth() {
        let c = Config::from_map(|k| match k {
            "PINGWARD_SMTP_HOST" => Some("mail.x".into()),
            "PINGWARD_SMTP_FROM" => Some("a@x".into()),
            "PINGWARD_SMTP_PORT" => Some("not-a-number".into()),
            "PINGWARD_SMTP_TLS" => Some("weird".into()),
            _ => None,
        });
        let s = c.smtp.unwrap();
        assert_eq!(s.port, 587, "blank/invalid port falls back to 587");
        assert_eq!(
            s.tls,
            SmtpTls::Starttls,
            "unknown tls falls back to starttls"
        );
        assert!(s.username.is_none() && s.password.is_none());
    }

    #[test]
    fn smtp_tls_modes_parse() {
        let mk = |tls: &str| {
            Config::from_map(|k| match k {
                "PINGWARD_SMTP_HOST" => Some("h".into()),
                "PINGWARD_SMTP_FROM" => Some("a@x".into()),
                "PINGWARD_SMTP_TLS" => Some(tls.into()),
                _ => None,
            })
            .smtp
            .unwrap()
            .tls
        };
        assert_eq!(mk("starttls"), SmtpTls::Starttls);
        assert_eq!(mk("none"), SmtpTls::None);
        assert_eq!(mk("tls"), SmtpTls::Tls);
    }
}
