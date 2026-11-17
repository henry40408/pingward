package web

import (
	"strconv"
	"strings"

	"github.com/henry40408/pingward/internal/config"
	"github.com/henry40408/pingward/internal/views"
)

// envGroups is the /admin Environment card, from the effective config (what is
// running). Secrets reach the views only as configured/not set.
func envGroups(cfg *config.Config) []views.EnvGroup {
	return []views.EnvGroup{
		{Title: "Server", Rows: serverEnv(cfg)},
		{Title: "Scheduling", Rows: schedulingEnv(cfg)},
		{Title: "Auth", Rows: authEnv(cfg)},
		{Title: "Email (SMTP)", Rows: emailEnv(cfg)},
	}
}

func envText(name, value, fallback, description string) views.EnvSetting {
	return views.EnvSetting{Var: name, Kind: views.EnvText, Value: value, Default: fallback, Description: description}
}

// envOptional is unset when value is nil.
func envOptional(name string, value *string, description string) views.EnvSetting {
	if value == nil {
		return views.EnvSetting{Var: name, Kind: views.EnvUnset, Default: "(unset)", Description: description}
	}
	return envText(name, *value, "(unset)", description)
}

func serverEnv(cfg *config.Config) []views.EnvSetting {
	logFormat := map[config.LogFormat]string{
		config.LogFormatFull: "full", config.LogFormatCompact: "compact",
		config.LogFormatPretty: "pretty", config.LogFormatJSON: "json",
	}[cfg.LogFormat]
	return []views.EnvSetting{
		envText("DATABASE_URL", redactDBURL(cfg.DatabaseURL), "sqlite://pingward.sqlite3?mode=rwc", "The database connection string (SQLite or Postgres)."),
		envText("PINGWARD_BIND", cfg.Bind, "127.0.0.1:8080", "The socket address the server listens on."),
		envText("PINGWARD_BASE_URL", cfg.BaseURL, "http://localhost:8080", "Used to render absolute ping URLs."),
		envText("PINGWARD_LOG_FORMAT", logFormat, "full", "Log line format (full, compact, pretty or json); applied at process startup — changing it requires a restart."),
		envText("PINGWARD_HSTS_MAX_AGE", strconv.FormatInt(cfg.HSTSMaxAgeSecs, 10), "0 (off)", "Strict-Transport-Security max-age in seconds; 0 sends no header. Prefer setting this on the reverse proxy — see README."),
	}
}

func schedulingEnv(cfg *config.Config) []views.EnvSetting {
	return []views.EnvSetting{
		envText("PINGWARD_SCAN_INTERVAL", strconv.FormatInt(cfg.ScanIntervalSecs, 10), "30", "Fallback scan interval — only used when no check, project, or global setting overrides it (check → project → global setting → env cascade)."),
		envText("PINGWARD_PRUNE_INTERVAL_SECS", strconv.FormatInt(cfg.PruneIntervalSecs, 10), "3600", "How often old pings/notifications are deleted."),
	}
}

func authEnv(cfg *config.Config) []views.EnvSetting {
	proxies := views.EnvSetting{Var: "PINGWARD_TRUSTED_PROXIES", Kind: views.EnvUnset, Default: "(unset)", Description: "Proxy addresses trusted to set the forward-auth header."}
	if len(cfg.TrustedProxies) > 0 {
		proxies = envText(proxies.Var, strings.Join(cfg.TrustedProxies, ", "), proxies.Default, proxies.Description)
	}
	return []views.EnvSetting{
		envOptional("PINGWARD_FORWARD_AUTH_HEADER", cfg.ForwardAuthHeader, "Header name for the trusted-proxy forward-auth mechanism."),
		proxies,
		envOptional("PINGWARD_FORWARD_AUTH_LOGOUT_URL", cfg.ForwardAuthLogout, "Where logging out sends the browser — point it at the gateway's sign-out endpoint to end the SSO session too. Unset means /login."),
		envText("PINGWARD_COOKIE_SECURE", strconv.FormatBool(cfg.CookieSecure), "derived from PINGWARD_BASE_URL's scheme", "Whether the session cookie carries `Secure`. Leave unset unless TLS terminates upstream and PINGWARD_BASE_URL cannot say so."),
	}
}

func emailEnv(cfg *config.Config) []views.EnvSetting {
	smtp := cfg.SMTP
	if smtp == nil {
		return emailEnvUnset()
	}
	tls := map[config.TLSMode]string{config.TLSStartTLS: "starttls", config.TLSImplicit: "tls", config.TLSNone: "none"}[smtp.TLS]
	password := views.EnvSecretNotSet
	if smtp.Password != nil {
		password = views.EnvSecretSet
	}
	return []views.EnvSetting{
		envText("PINGWARD_SMTP_HOST", smtp.Host, "(unset)", "Instance SMTP server host."),
		envText("PINGWARD_SMTP_PORT", strconv.FormatUint(uint64(smtp.Port), 10), "(unset)", "Instance SMTP server port."),
		envText("PINGWARD_SMTP_FROM", smtp.From, "(unset)", "The From address used for outgoing email notifications."),
		envText("PINGWARD_SMTP_TLS", tls, "(unset)", "SMTP transport security mode (starttls, tls, or none)."),
		envOptional("PINGWARD_SMTP_USERNAME", smtp.Username, "SMTP AUTH username (an identity, not a credential — shown verbatim)."),
		{Var: "PINGWARD_SMTP_PASSWORD", Kind: password, Default: "(unset)", Description: "SMTP AUTH password — never displayed, only whether it's configured."},
	}
}

func emailEnvUnset() []views.EnvSetting {
	return []views.EnvSetting{
		envOptional("PINGWARD_SMTP_HOST", nil, "Instance SMTP server host."),
		envOptional("PINGWARD_SMTP_PORT", nil, "Instance SMTP server port."),
		envOptional("PINGWARD_SMTP_FROM", nil, "The From address used for outgoing email notifications."),
		envOptional("PINGWARD_SMTP_TLS", nil, "SMTP transport security mode (starttls, tls, or none)."),
		envOptional("PINGWARD_SMTP_USERNAME", nil, "SMTP AUTH username (an identity, not a credential — shown verbatim)."),
		{Var: "PINGWARD_SMTP_PASSWORD", Kind: views.EnvSecretNotSet, Default: "(unset)", Description: "SMTP AUTH password — never displayed, only whether it's configured."},
	}
}

// redactDBURL turns scheme://user:pw@host/db into scheme://***@host/db. An
// authority with no @ (a plain SQLite path) is returned unchanged. Only an @
// before the first /, ? or # is userinfo; a later one is path or query.
func redactDBURL(url string) string {
	schemeEnd := strings.Index(url, "://")
	if schemeEnd < 0 {
		return url
	}
	authorityStart := schemeEnd + 3
	rest := url[authorityStart:]
	for i, c := range rest {
		switch c {
		case '@':
			return url[:authorityStart] + "***@" + rest[i+1:]
		case '/', '?', '#':
			return url
		}
	}
	return url
}
