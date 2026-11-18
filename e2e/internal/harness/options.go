package harness

import "slices"

// Options is the server environment a scenario's tags select.
type Options struct {
	// ScanIntervalSecs is PINGWARD_SCAN_INTERVAL, for @fast-scan: on a fresh
	// database the scan loop's first sleep is the environment default (30 s),
	// since no per-check override exists yet. Zero keeps the default.
	ScanIntervalSecs int
	// ExtraEnv is for @smtp-env and @trusted-proxy, as KEY=value.
	ExtraEnv []string
}

// OptionsFromTags maps scenario tags (without the @) to the server
// environment.
func OptionsFromTags(tags []string) Options {
	tagged := func(name string) bool { return slices.Contains(tags, name) }
	var options Options
	if tagged("fast-scan") {
		options.ScanIntervalSecs = 1
	}
	// Gives /admin's Environment card an SMTP config, and a password it must
	// not print.
	if tagged("smtp-env") {
		options.ExtraEnv = append(options.ExtraEnv,
			"PINGWARD_SMTP_HOST=smtp.e2e.test",
			"PINGWARD_SMTP_FROM=alerts@e2e.test",
			"PINGWARD_SMTP_PASSWORD=e2e-secret-password")
	}
	// So the client IP honours the scenario's X-Forwarded-For.
	if tagged("trusted-proxy") {
		options.ExtraEnv = append(options.ExtraEnv, "PINGWARD_TRUSTED_PROXIES=127.0.0.1")
	}
	return options
}
