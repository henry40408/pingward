// Command pingward is a self-hosted cron-job and scheduled-task monitor.
package main

import (
	"fmt"
	"io"
	"log/slog"
	"os"
	"os/signal"
	"syscall"

	"github.com/henry40408/pingward/internal/config"
)

const exitFailure = 1

// process is everything run needs from the outside world.
type process struct {
	lookupEnv func(string) (string, bool)
	stdout    io.Writer
	stderr    io.Writer
	// interrupts delivers SIGINT and SIGTERM.
	interrupts <-chan os.Signal
	// serve runs the server after the startup checks pass; nil stops there,
	// so the tests of that phase need no database.
	serve func(p process, s started) int
}

// started is what the startup checks produce.
type started struct {
	cfg    config.Config
	logger *slog.Logger
}

func main() {
	// Buffered: a signal that arrives before serve listens must not be lost.
	interrupts := make(chan os.Signal, 1)
	signal.Notify(interrupts, syscall.SIGINT, syscall.SIGTERM)
	os.Exit(run(process{
		lookupEnv:  os.LookupEnv,
		stdout:     os.Stdout,
		stderr:     os.Stderr,
		interrupts: interrupts,
		serve:      serve,
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
	if p.serve == nil {
		return 0
	}
	return p.serve(p, started{cfg: cfg, logger: logger})
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
