// Command pingward is a self-hosted cron-job and scheduled-task monitor.
package main

import (
	"fmt"
	"io"
	"os"

	"github.com/henry40408/pingward/internal/config"
)

const exitFailure = 1

// process is everything run needs from the outside world.
type process struct {
	lookupEnv func(string) (string, bool)
	stdout    io.Writer
	stderr    io.Writer
}

func main() {
	os.Exit(run(process{
		lookupEnv: os.LookupEnv,
		stdout:    os.Stdout,
		stderr:    os.Stderr,
	}))
}

// run returns the exit code. Logs go to stdout; stderr stays free for the
// setup token and for startup errors.
func run(p process) int {
	if err := ensureNoLegacyEnvVars(p.lookupEnv); err != nil {
		fmt.Fprintf(p.stderr, "pingward: %v\n", err)
		return exitFailure
	}
	cfg := config.Load(p.lookupEnv)
	levelText, _ := p.lookupEnv("PINGWARD_LOG_LEVEL")
	level, err := parseLogLevel(levelText)
	if err != nil {
		fmt.Fprintf(p.stderr, "pingward: PINGWARD_LOG_LEVEL: %v\n", err)
		return exitFailure
	}
	logger := newLogger(p.stdout, cfg.LogFormat, level)
	for _, warning := range startupWarnings(cfg) {
		logger.Warn(warning)
	}
	return 0
}

func startupWarnings(cfg config.Config) []string {
	warnings := append([]string(nil), cfg.Warnings...)
	for _, warning := range []string{
		config.EphemeralSecretWarning(cfg.SecretSource),
		config.OrphanLogoutURLWarning(cfg),
	} {
		if warning != "" {
			warnings = append(warnings, warning)
		}
	}
	return warnings
}
