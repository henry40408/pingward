// Package safego keeps a panic in a background goroutine from killing the
// process or silently ending a loop.
package safego

import (
	"context"
	"log/slog"
	"runtime/debug"
	"time"
)

const maxBackoff = time.Minute

// Recover is deferred at the top of a goroutine: it logs a panic with its stack.
func Recover(logger *slog.Logger, name string) {
	if value := recover(); value != nil {
		logger.Error("goroutine panicked", "name", name, "panic", value, "stack", string(debug.Stack()))
	}
}

// Go runs fn in a goroutine that survives its own panic.
func Go(logger *slog.Logger, name string, fn func()) {
	go func() {
		defer Recover(logger, name)
		fn()
	}()
}

// Supervise runs loop until it returns normally. After a panic it logs, waits
// (doubling from backoff up to a minute, so a persistent panic cannot spin the
// CPU) and runs it again. It returns early when ctx is cancelled during a wait.
func Supervise(ctx context.Context, logger *slog.Logger, name string, backoff time.Duration, loop func()) {
	for {
		if runOnce(logger, name, loop) {
			return
		}
		logger.Error("loop restarting after panic", "name", name, "backoff", backoff.String())
		select {
		case <-time.After(backoff):
		case <-ctx.Done():
			logger.Error("loop ended after panic; not restarting", "name", name)
			return
		}
		backoff = min(backoff*2, maxBackoff)
	}
}

// runOnce reports whether loop returned without panicking.
func runOnce(logger *slog.Logger, name string, loop func()) (completed bool) {
	defer func() {
		if value := recover(); value != nil {
			logger.Error("goroutine panicked", "name", name, "panic", value, "stack", string(debug.Stack()))
		}
	}()
	loop()
	return true
}
