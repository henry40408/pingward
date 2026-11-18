// Package config reads the process configuration from environment variables.
package config

import (
	"fmt"
	"log/slog"
	"strings"

	"github.com/henry40408/pingward/internal/duration"
	"github.com/henry40408/pingward/internal/secret"
)

// LogFormat selects how log lines are written. Full, Compact and Pretty are
// all text; slog has one text layout.
type LogFormat int

const (
	LogFormatFull LogFormat = iota
	LogFormatCompact
	LogFormatPretty
	LogFormatJSON
)

// Config is the whole process configuration.
type Config struct {
	DatabaseURL       string
	Bind              string
	BaseURL           string
	ScanIntervalSecs  int64
	PruneIntervalSecs int64
	ForwardAuthHeader *string
	ForwardAuthLogout *string
	TrustedProxies    []string
	SMTP              *SMTPConfig
	LogFormat         LogFormat
	Secret            []byte
	SecretSource      secret.Source
	CookieSecure      bool
	HSTSMaxAgeSecs    int64
	// Warnings are problems found while reading the environment. The caller
	// logs them once a logger exists.
	Warnings []string
}

// Load reads the configuration through lookupEnv (os.LookupEnv in production).
func Load(lookupEnv func(string) (string, bool)) Config {
	env := environment{lookup: lookupEnv}
	var warnings []string

	logFormat, logWarning := parseLogFormat(env.raw("PINGWARD_LOG_FORMAT"))
	warnings = appendIf(warnings, logWarning)

	smtp, smtpWarning := loadSMTP(env)
	warnings = appendIf(warnings, smtpWarning)

	baseURL := env.rawOr("PINGWARD_BASE_URL", "http://localhost:8080")
	cookieSecure, cookieWarning := parseCookieSecure(env.trimmed("PINGWARD_COOKIE_SECURE"), baseURL)
	warnings = appendIf(warnings, cookieWarning)

	secretBytes, secretSource := secret.Resolve(env.trimmed("PINGWARD_SECRET"))

	return Config{
		DatabaseURL:       env.rawOr("DATABASE_URL", "sqlite://pingward.sqlite3?mode=rwc"),
		Bind:              env.rawOr("PINGWARD_BIND", "127.0.0.1:8080"),
		BaseURL:           baseURL,
		ScanIntervalSecs:  env.durationSecs("PINGWARD_SCAN_INTERVAL", 30),
		PruneIntervalSecs: env.durationSecs("PINGWARD_PRUNE_INTERVAL_SECS", 3600),
		ForwardAuthHeader: env.optionalRaw("PINGWARD_FORWARD_AUTH_HEADER"),
		ForwardAuthLogout: env.optionalNonBlank("PINGWARD_FORWARD_AUTH_LOGOUT_URL"),
		TrustedProxies:    splitList(env.raw("PINGWARD_TRUSTED_PROXIES")),
		SMTP:              smtp,
		LogFormat:         logFormat,
		Secret:            secretBytes,
		SecretSource:      secretSource,
		CookieSecure:      cookieSecure,
		HSTSMaxAgeSecs:    env.durationSecs("PINGWARD_HSTS_MAX_AGE", 0),
		Warnings:          warnings,
	}
}

// String and GoString keep the secret and the SMTP password out of logs and
// of the admin environment card.
func (c Config) String() string {
	return fmt.Sprintf("config{database_url=%q bind=%q base_url=%q smtp=%v}",
		c.DatabaseURL, c.Bind, c.BaseURL, c.SMTP)
}

func (c Config) GoString() string { return c.String() }

// LogValue implements slog.LogValuer without the secret.
func (c Config) LogValue() slog.Value { return slog.StringValue(c.String()) }

// environment wraps lookups so each variable is read in one way.
type environment struct {
	lookup func(string) (string, bool)
}

func (e environment) raw(key string) string {
	value, _ := e.lookup(key)
	return value
}

func (e environment) rawOr(key, fallback string) string {
	if value, ok := e.lookup(key); ok {
		return value
	}
	return fallback
}

func (e environment) optionalRaw(key string) *string {
	value, ok := e.lookup(key)
	if !ok {
		return nil
	}
	return &value
}

// trimmed returns the trimmed value; blank means unset.
func (e environment) trimmed(key string) string {
	raw, _ := e.lookup(key)
	return strings.TrimSpace(raw)
}

func (e environment) optionalNonBlank(key string) *string {
	value := e.trimmed(key)
	if value == "" {
		return nil
	}
	return &value
}

// durationSecs reads seconds from a bare integer or a human duration; an
// unparseable or negative value falls back, so a typo cannot stop the server
// booting. Zero is kept.
func (e environment) durationSecs(key string, fallback int64) int64 {
	raw, ok := e.lookup(key)
	if !ok {
		return fallback
	}
	seconds, ok := duration.Parse(raw)
	if !ok || seconds < 0 {
		return fallback
	}
	return seconds
}

// parseLogFormat treats anything unknown, including the legacy "text", as full.
func parseLogFormat(raw string) (LogFormat, string) {
	switch strings.ToLower(strings.TrimSpace(raw)) {
	case "json":
		return LogFormatJSON, ""
	case "compact":
		return LogFormatCompact, ""
	case "pretty":
		return LogFormatPretty, ""
	case "", "full", "text":
		return LogFormatFull, ""
	}
	return LogFormatFull, fmt.Sprintf("unrecognized PINGWARD_LOG_FORMAT %q; using full", raw)
}

func splitList(raw string) []string {
	var items []string
	for _, part := range strings.Split(raw, ",") {
		if trimmed := strings.TrimSpace(part); trimmed != "" {
			items = append(items, trimmed)
		}
	}
	return items
}

func appendIf(list []string, item string) []string {
	if item == "" {
		return list
	}
	return append(list, item)
}

// parseCookieSecure honours an explicit true/false/1/0, else follows the
// scheme of the base URL. It is never forced on: browsers drop a Secure cookie
// over HTTP, so a plaintext deploy would silently fail to log in.
func parseCookieSecure(raw, baseURL string) (bool, string) {
	if raw == "" {
		return derivedFromBaseURL(baseURL), ""
	}
	switch strings.ToLower(raw) {
	case "true", "1":
		return true, ""
	case "false", "0":
		return false, ""
	}
	return derivedFromBaseURL(baseURL), fmt.Sprintf(
		"invalid PINGWARD_COOKIE_SECURE '%s': expected true/false/1/0; "+
			"falling back to the scheme of PINGWARD_BASE_URL", raw)
}

func derivedFromBaseURL(baseURL string) bool {
	return strings.HasPrefix(strings.ToLower(strings.TrimLeft(baseURL, " \t\n\f\r")), "https://")
}
