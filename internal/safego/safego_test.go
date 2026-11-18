package safego

import (
	"bytes"
	"context"
	"log/slog"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestRecoverLogsAPanic(t *testing.T) {
	var out bytes.Buffer
	func() {
		defer Recover(slog.New(slog.NewTextHandler(&out, nil)), "boom")
		panic("kaboom")
	}()
	if log := out.String(); !strings.Contains(log, "kaboom") || !strings.Contains(log, "name=boom") {
		t.Fatalf("log = %q", log)
	}
}

func TestSuperviseRestartsAfterAPanic(t *testing.T) {
	var runs atomic.Int32
	Supervise(t.Context(), slog.New(slog.DiscardHandler), "loop", time.Millisecond, func() {
		if runs.Add(1) < 3 {
			panic("again")
		}
	})
	if got := runs.Load(); got != 3 {
		t.Fatalf("runs = %d, want 3", got)
	}
}

func TestSuperviseStopsWaitingWhenCancelled(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	var runs atomic.Int32
	Supervise(ctx, slog.New(slog.DiscardHandler), "loop", time.Hour, func() {
		runs.Add(1)
		panic("always")
	})
	if got := runs.Load(); got != 1 {
		t.Fatalf("runs = %d, want 1", got)
	}
}
