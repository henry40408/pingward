package testutil

import (
	"bytes"
	"encoding/json"
	"log/slog"
	"strings"
	"sync"
	"testing"
)

// LockedBuffer is a log sink safe to write from server goroutines while the
// test reads it.
type LockedBuffer struct {
	mu   sync.Mutex
	data bytes.Buffer
}

func (b *LockedBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.data.Write(p)
}

func (b *LockedBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.data.String()
}

// JSONLogger logs at debug level as JSON into a fresh LockedBuffer.
func JSONLogger() (*slog.Logger, *LockedBuffer) {
	buf := &LockedBuffer{}
	handler := slog.NewJSONHandler(buf, &slog.HandlerOptions{Level: slog.LevelDebug})
	return slog.New(handler), buf
}

// Events decodes every JSON log line.
func (b *LockedBuffer) Events(t *testing.T) []map[string]any {
	t.Helper()
	var events []map[string]any
	for _, line := range strings.Split(strings.TrimSpace(b.String()), "\n") {
		if line == "" {
			continue
		}
		var event map[string]any
		if err := json.Unmarshal([]byte(line), &event); err != nil {
			t.Fatalf("log line is not JSON: %q: %v", line, err)
		}
		events = append(events, event)
	}
	return events
}

// EventsNamed is the events whose msg is msg.
func (b *LockedBuffer) EventsNamed(t *testing.T, msg string) []map[string]any {
	t.Helper()
	var named []map[string]any
	for _, event := range b.Events(t) {
		if event["msg"] == msg {
			named = append(named, event)
		}
	}
	return named
}
