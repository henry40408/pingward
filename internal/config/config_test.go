package config

import (
	"bytes"
	"fmt"
	"log/slog"
	"strings"
	"testing"

	"github.com/henry40408/pingward/internal/secret"
)

func load(env map[string]string) Config {
	return Load(func(key string) (string, bool) {
		value, ok := env[key]
		return value, ok
	})
}

func TestDefaultsApplyWhenUnset(t *testing.T) {
	c := load(nil)
	if c.ScanIntervalSecs != 30 || c.Bind != "127.0.0.1:8080" ||
		c.DatabaseURL != "sqlite://pingward.sqlite3?mode=rwc" ||
		c.BaseURL != "http://localhost:8080" || c.PruneIntervalSecs != 3600 {
		t.Fatalf("unexpected defaults: %s", c)
	}
}

func TestLogFormat(t *testing.T) {
	cases := []struct {
		name, value string
		want        LogFormat
	}{
		{"unset", "", LogFormatFull},
		{"compact", "compact", LogFormatCompact},
		{"pretty trimmed and uppercase", "  PRETTY  ", LogFormatPretty},
		{"legacy text is full", "text", LogFormatFull},
		{"json uppercase", "JSON", LogFormatJSON},
		{"json trimmed", "  json  ", LogFormatJSON},
		{"unrecognised falls back to full", "yaml", LogFormatFull},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := load(map[string]string{"PINGWARD_LOG_FORMAT": tc.value}).LogFormat; got != tc.want {
				t.Fatalf("got %d, want %d", got, tc.want)
			}
		})
	}
}

func TestUnrecognisedLogFormatWarns(t *testing.T) {
	c := load(map[string]string{"PINGWARD_LOG_FORMAT": "yaml"})
	if len(c.Warnings) != 1 || !strings.Contains(c.Warnings[0], "yaml") {
		t.Fatalf("warnings: %v", c.Warnings)
	}
	if got := load(map[string]string{"PINGWARD_LOG_FORMAT": "text"}).Warnings; len(got) != 0 {
		t.Fatalf("legacy text must not warn: %v", got)
	}
}

func TestEnvOverridesDefaults(t *testing.T) {
	c := load(map[string]string{
		"PINGWARD_SCAN_INTERVAL":   "10",
		"PINGWARD_TRUSTED_PROXIES": "10.0.0.1,10.0.0.2",
	})
	if c.ScanIntervalSecs != 10 {
		t.Fatalf("scan interval %d", c.ScanIntervalSecs)
	}
	if fmt.Sprint(c.TrustedProxies) != "[10.0.0.1 10.0.0.2]" {
		t.Fatalf("proxies %v", c.TrustedProxies)
	}
}

func TestTrustedProxiesAreTrimmedAndBlanksDropped(t *testing.T) {
	c := load(map[string]string{"PINGWARD_TRUSTED_PROXIES": " 10.0.0.1 ,, 10.0.0.0/8 ,"})
	if fmt.Sprint(c.TrustedProxies) != "[10.0.0.1 10.0.0.0/8]" {
		t.Fatalf("proxies %v", c.TrustedProxies)
	}
}

func TestPruneIntervalDefaultsAndOverrides(t *testing.T) {
	if got := load(nil).PruneIntervalSecs; got != 3600 {
		t.Fatalf("default %d", got)
	}
	if got := load(map[string]string{"PINGWARD_PRUNE_INTERVAL_SECS": "60"}).PruneIntervalSecs; got != 60 {
		t.Fatalf("override %d", got)
	}
}

func TestIntervalsAcceptHumanReadableDurations(t *testing.T) {
	if got := load(map[string]string{"PINGWARD_SCAN_INTERVAL": "5m"}).ScanIntervalSecs; got != 300 {
		t.Fatalf("scan %d", got)
	}
	if got := load(map[string]string{"PINGWARD_PRUNE_INTERVAL_SECS": "1h"}).PruneIntervalSecs; got != 3600 {
		t.Fatalf("prune %d", got)
	}
}

func TestInvalidOrNegativeDurationFallsBackToDefault(t *testing.T) {
	for _, value := range []string{"abc", "-5"} {
		if got := load(map[string]string{"PINGWARD_SCAN_INTERVAL": value}).ScanIntervalSecs; got != 30 {
			t.Fatalf("%q gave %d", value, got)
		}
	}
}

func TestZeroDurationIsPreserved(t *testing.T) {
	if got := load(map[string]string{"PINGWARD_SCAN_INTERVAL": "0"}).ScanIntervalSecs; got != 0 {
		t.Fatalf("got %d", got)
	}
}

func TestForwardAuthLogoutURLIsOptionalAndTrimmed(t *testing.T) {
	if load(nil).ForwardAuthLogout != nil {
		t.Fatal("unset must be nil")
	}
	c := load(map[string]string{"PINGWARD_FORWARD_AUTH_LOGOUT_URL": "  https://auth.example.com/logout "})
	if c.ForwardAuthLogout == nil || *c.ForwardAuthLogout != "https://auth.example.com/logout" {
		t.Fatalf("got %v", c.ForwardAuthLogout)
	}
	// Blank is unset: an empty Location would strand the browser.
	if load(map[string]string{"PINGWARD_FORWARD_AUTH_LOGOUT_URL": "  "}).ForwardAuthLogout != nil {
		t.Fatal("blank must be nil")
	}
}

func TestForwardAuthHeaderIsKeptVerbatim(t *testing.T) {
	c := load(map[string]string{"PINGWARD_FORWARD_AUTH_HEADER": " X-User "})
	if c.ForwardAuthHeader == nil || *c.ForwardAuthHeader != " X-User " {
		t.Fatalf("got %v", c.ForwardAuthHeader)
	}
	if load(nil).ForwardAuthHeader != nil {
		t.Fatal("unset must be nil")
	}
}

func smtpEnv(extra map[string]string) map[string]string {
	env := map[string]string{"PINGWARD_SMTP_HOST": "mail.x", "PINGWARD_SMTP_FROM": "a@x"}
	for key, value := range extra {
		env[key] = value
	}
	return env
}

func TestSMTPIsNilWhenHostOrFromMissing(t *testing.T) {
	for _, env := range []map[string]string{
		nil,
		{"PINGWARD_SMTP_HOST": "mail.x"},
		{"PINGWARD_SMTP_FROM": "a@x"},
		{"PINGWARD_SMTP_HOST": "  ", "PINGWARD_SMTP_FROM": "a@x"},
	} {
		if load(env).SMTP != nil {
			t.Fatalf("env %v must not enable SMTP", env)
		}
	}
}

func TestSMTPParsedWhenHostAndFromPresent(t *testing.T) {
	c := load(map[string]string{
		"PINGWARD_SMTP_HOST":     "mail.example.com",
		"PINGWARD_SMTP_FROM":     "alerts@example.com",
		"PINGWARD_SMTP_PORT":     "2525",
		"PINGWARD_SMTP_USERNAME": "u",
		"PINGWARD_SMTP_PASSWORD": "p",
		"PINGWARD_SMTP_TLS":      "TLS",
	})
	s := c.SMTP
	if s == nil || s.Host != "mail.example.com" || s.From != "alerts@example.com" || s.Port != 2525 ||
		s.Username == nil || *s.Username != "u" || s.Password == nil || *s.Password != "p" ||
		s.TLS != TLSImplicit {
		t.Fatalf("got %+v", s)
	}
}

func TestSMTPDefaultsPortTLSAndOptionalAuth(t *testing.T) {
	s := load(smtpEnv(map[string]string{"PINGWARD_SMTP_PORT": "not-a-number", "PINGWARD_SMTP_TLS": "weird"})).SMTP
	if s.Port != 587 {
		t.Fatalf("invalid port must fall back to 587, got %d", s.Port)
	}
	if s.TLS != TLSStartTLS {
		t.Fatalf("unknown tls must fall back to starttls, got %d", s.TLS)
	}
	if s.Username != nil || s.Password != nil {
		t.Fatal("auth must be optional")
	}
}

func TestSMTPPortOutOfRangeFallsBack(t *testing.T) {
	if got := load(smtpEnv(map[string]string{"PINGWARD_SMTP_PORT": "70000"})).SMTP.Port; got != 587 {
		t.Fatalf("got %d", got)
	}
}

func TestSMTPTLSModesParse(t *testing.T) {
	cases := map[string]TLSMode{"starttls": TLSStartTLS, "none": TLSNone, "tls": TLSImplicit}
	for value, want := range cases {
		if got := load(smtpEnv(map[string]string{"PINGWARD_SMTP_TLS": value})).SMTP.TLS; got != want {
			t.Errorf("%q gave %d, want %d", value, got, want)
		}
	}
}

func TestSMTPImplicitTLSDefaultsPort465(t *testing.T) {
	cases := []struct {
		tls  string
		want uint16
	}{{"tls", 465}, {"starttls", 587}, {"", 587}}
	for _, tc := range cases {
		extra := map[string]string{}
		if tc.tls != "" {
			extra["PINGWARD_SMTP_TLS"] = tc.tls
		}
		if got := load(smtpEnv(extra)).SMTP.Port; got != tc.want {
			t.Errorf("tls %q gave port %d, want %d", tc.tls, got, tc.want)
		}
	}
}

func TestSMTPAuthWithTLSNoneWarns(t *testing.T) {
	env := smtpEnv(map[string]string{
		"PINGWARD_SMTP_TLS": "none", "PINGWARD_SMTP_USERNAME": "u", "PINGWARD_SMTP_PASSWORD": "p",
	})
	c := load(env)
	want := "SMTP AUTH credentials set with PINGWARD_SMTP_TLS=none: credentials will be sent unencrypted"
	if len(c.Warnings) != 1 || c.Warnings[0] != want {
		t.Fatalf("warnings: %v", c.Warnings)
	}
	delete(env, "PINGWARD_SMTP_PASSWORD")
	if got := load(env).Warnings; len(got) != 0 {
		t.Fatalf("username alone must not warn: %v", got)
	}
}

func TestCookieSecureDerivedFromBaseURLScheme(t *testing.T) {
	cases := map[string]bool{
		"https://pingward.example": true,
		"HTTPS://pingward.example": true,
		"  https://x":              true,
		"http://pingward.example":  false,
		"http://localhost:8080":    false,
	}
	for baseURL, want := range cases {
		got, warning := parseCookieSecure("", baseURL)
		if got != want || warning != "" {
			t.Errorf("%q gave %v, %q", baseURL, got, warning)
		}
	}
}

func TestCookieSecureExplicitValueOverridesDerivation(t *testing.T) {
	cases := []struct {
		raw, baseURL string
		want         bool
	}{
		{"true", "http://pingward.example", true},
		{"TRUE", "http://pingward.example", true},
		{"1", "http://pingward.example", true},
		{"false", "https://pingward.example", false},
		{"0", "https://pingward.example", false},
	}
	for _, tc := range cases {
		if got, warning := parseCookieSecure(tc.raw, tc.baseURL); got != tc.want || warning != "" {
			t.Errorf("%q on %q gave %v, %q", tc.raw, tc.baseURL, got, warning)
		}
	}
}

func TestCookieSecureInvalidValueFallsBackToDerivedAndWarns(t *testing.T) {
	got, warning := parseCookieSecure("yes", "http://pingward.example")
	if got || !strings.Contains(warning, "invalid PINGWARD_COOKIE_SECURE 'yes'") {
		t.Fatalf("got %v, %q", got, warning)
	}
	if got, _ := parseCookieSecure("yes", "https://pingward.example"); !got {
		t.Fatal("must derive true from https")
	}
}

func TestCookieSecureEndToEnd(t *testing.T) {
	if !load(map[string]string{"PINGWARD_BASE_URL": "https://p.example"}).CookieSecure {
		t.Fatal("https base URL must set Secure")
	}
	c := load(map[string]string{"PINGWARD_BASE_URL": "https://p.example", "PINGWARD_COOKIE_SECURE": "0"})
	if c.CookieSecure {
		t.Fatal("explicit 0 must win")
	}
}

func TestHSTSMaxAge(t *testing.T) {
	if got := load(nil).HSTSMaxAgeSecs; got != 0 {
		t.Fatalf("default %d", got)
	}
	if got := load(map[string]string{"PINGWARD_HSTS_MAX_AGE": "365d"}).HSTSMaxAgeSecs; got != 31536000 {
		t.Fatalf("365d gave %d", got)
	}
	if got := load(map[string]string{"PINGWARD_HSTS_MAX_AGE": "not-a-duration"}).HSTSMaxAgeSecs; got != 0 {
		t.Fatalf("invalid gave %d", got)
	}
}

func TestSecretResolution(t *testing.T) {
	long := "0123456789abcdef"
	c := load(map[string]string{"PINGWARD_SECRET": "  " + long + "  "})
	if c.SecretSource != secret.SourceEnv || string(c.Secret) != long {
		t.Fatalf("source %d secret %q", c.SecretSource, c.Secret)
	}
	if c := load(map[string]string{"PINGWARD_SECRET": "short"}); c.SecretSource != secret.SourceRejected || len(c.Secret) != 32 {
		t.Fatalf("short: source %d len %d", c.SecretSource, len(c.Secret))
	}
	if c := load(map[string]string{"PINGWARD_SECRET": "   "}); c.SecretSource != secret.SourceGenerated {
		t.Fatalf("blank: source %d", c.SecretSource)
	}
	if bytes.Equal(load(nil).Secret, load(nil).Secret) {
		t.Fatal("generated secrets must differ between loads")
	}
}

func TestConfigNeverPrintsSecrets(t *testing.T) {
	c := load(map[string]string{
		"PINGWARD_SECRET":        "topsecret-topsecret-topsecret",
		"PINGWARD_SMTP_HOST":     "mail.x",
		"PINGWARD_SMTP_FROM":     "a@x",
		"PINGWARD_SMTP_USERNAME": "u",
		"PINGWARD_SMTP_PASSWORD": "smtp-password-123",
	})
	var logged bytes.Buffer
	slog.New(slog.NewTextHandler(&logged, nil)).Info("config", "config", c)
	outputs := []string{
		fmt.Sprint(c), fmt.Sprintf("%v", c), fmt.Sprintf("%+v", c), fmt.Sprintf("%#v", c),
		fmt.Sprintf("%+v", &c), fmt.Sprintf("%+v", c.SMTP), fmt.Sprintf("%#v", c.SMTP), logged.String(),
	}
	for _, out := range outputs {
		for _, leaked := range []string{"topsecret", "smtp-password-123", "116 111 112"} {
			if strings.Contains(out, leaked) {
				t.Fatalf("%q leaked into %q", leaked, out)
			}
		}
	}
}

func TestEffectiveScanIntervalPrefersMostSpecific(t *testing.T) {
	i := func(v int64) *int64 { return &v }
	cases := []struct {
		check, project, global *int64
		envDefault, want       int64
	}{
		{i(5), i(10), i(20), 30, 5},
		{nil, i(10), i(20), 30, 10},
		{nil, nil, i(20), 30, 20},
		{nil, nil, nil, 30, 30},
		{i(0), i(-1), nil, 30, 30},
		{nil, nil, nil, 0, 1},
	}
	for n, tc := range cases {
		if got := EffectiveScanInterval(tc.check, tc.project, tc.global, tc.envDefault); got != tc.want {
			t.Errorf("case %d: got %d, want %d", n, got, tc.want)
		}
	}
}

func TestEffectiveNagIntervalPrefersMostSpecificAndIsOptIn(t *testing.T) {
	i := func(v int64) *int64 { return &v }
	cases := []struct {
		check, project, global *int64
		want                   int64
		wantOK                 bool
	}{
		{i(5), i(10), i(20), 5, true},
		{nil, i(10), i(20), 10, true},
		{nil, nil, i(20), 20, true},
		{nil, nil, nil, 0, false},
		{i(0), i(-1), i(30), 30, true},
		{i(0), nil, nil, 0, false},
	}
	for n, tc := range cases {
		got, ok := EffectiveNagInterval(tc.check, tc.project, tc.global)
		if got != tc.want || ok != tc.wantOK {
			t.Errorf("case %d: got %d, %v", n, got, ok)
		}
	}
}

func TestEphemeralSecretWarning(t *testing.T) {
	if EphemeralSecretWarning(secret.SourceEnv) != "" {
		t.Fatal("env secret must not warn")
	}
	if got := EphemeralSecretWarning(secret.SourceGenerated); !strings.HasPrefix(got, "PINGWARD_SECRET is not set; using a secret generated") {
		t.Fatalf("generated: %q", got)
	}
	if got := EphemeralSecretWarning(secret.SourceRejected); !strings.Contains(got, "shorter than the 16-byte minimum and was ignored") {
		t.Fatalf("rejected: %q", got)
	}
}

func TestOrphanLogoutURLWarning(t *testing.T) {
	logout, header := "https://auth/logout", "X-User"
	cases := []struct {
		name string
		c    Config
		want bool
	}{
		{"logout without header", Config{ForwardAuthLogout: &logout}, true},
		{"both set", Config{ForwardAuthLogout: &logout, ForwardAuthHeader: &header}, false},
		{"neither set", Config{}, false},
		{"header only", Config{ForwardAuthHeader: &header}, false},
	}
	for _, tc := range cases {
		if got := OrphanLogoutURLWarning(tc.c) != ""; got != tc.want {
			t.Errorf("%s: got %v", tc.name, got)
		}
	}
}

func TestSMTPAuthWithTLSNoneWarnsOnlyWithBothCredentials(t *testing.T) {
	cases := []struct {
		name  string
		extra map[string]string
		want  int
	}{
		{"password alone", map[string]string{"PINGWARD_SMTP_TLS": "none", "PINGWARD_SMTP_PASSWORD": "p"}, 0},
		{"username alone", map[string]string{"PINGWARD_SMTP_TLS": "none", "PINGWARD_SMTP_USERNAME": "u"}, 0},
		{"both with starttls", map[string]string{"PINGWARD_SMTP_USERNAME": "u", "PINGWARD_SMTP_PASSWORD": "p"}, 0},
		{"both with tls", map[string]string{"PINGWARD_SMTP_TLS": "tls", "PINGWARD_SMTP_USERNAME": "u", "PINGWARD_SMTP_PASSWORD": "p"}, 0},
		{"both with none", map[string]string{"PINGWARD_SMTP_TLS": "none", "PINGWARD_SMTP_USERNAME": "u", "PINGWARD_SMTP_PASSWORD": "p"}, 1},
	}
	for _, tc := range cases {
		if got := load(smtpEnv(tc.extra)).Warnings; len(got) != tc.want {
			t.Errorf("%s: warnings = %v, want %d", tc.name, got, tc.want)
		}
	}
}

func TestSMTPStringReportsAuthOnlyWithBothCredentials(t *testing.T) {
	user, pass := "u", "p"
	cases := []struct {
		name string
		cfg  SMTPConfig
		want string
	}{
		{"none", SMTPConfig{}, "auth=false"},
		{"username only", SMTPConfig{Username: &user}, "auth=false"},
		{"password only", SMTPConfig{Password: &pass}, "auth=false"},
		{"both", SMTPConfig{Username: &user, Password: &pass}, "auth=true"},
	}
	for _, tc := range cases {
		if got := tc.cfg.String(); !strings.Contains(got, tc.want) {
			t.Errorf("%s: %q lacks %q", tc.name, got, tc.want)
		}
	}
}
