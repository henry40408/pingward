#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind: String,
    pub base_url: String,
    pub scan_interval_secs: u64,
    pub forward_auth_header: Option<String>,
    pub trusted_proxies: Vec<String>,
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
        let trusted_proxies = get("PINGWARD_TRUSTED_PROXIES")
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Config {
            database_url: get("DATABASE_URL")
                .unwrap_or_else(|| "sqlite://pingward.db?mode=rwc".into()),
            bind: get("PINGWARD_BIND").unwrap_or_else(|| "127.0.0.1:8080".into()),
            base_url: get("PINGWARD_BASE_URL").unwrap_or_else(|| "http://localhost:8080".into()),
            scan_interval_secs,
            forward_auth_header: get("PINGWARD_FORWARD_AUTH_HEADER"),
            trusted_proxies,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_apply_when_unset() {
        let c = Config::from_map(|_| None);
        assert_eq!(c.scan_interval_secs, 30);
        assert_eq!(c.bind, "127.0.0.1:8080");
        assert_eq!(c.database_url, "sqlite://pingward.db?mode=rwc");
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
}
