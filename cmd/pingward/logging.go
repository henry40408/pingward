package main

import (
	"fmt"
	"io"
	"log/slog"
	"strings"

	"github.com/henry40408/pingward/internal/config"
)

// parseLogLevel reads PINGWARD_LOG_LEVEL; blank means info. Unlike the Rust
// RUST_LOG it is a single level, not a filter expression.
func parseLogLevel(text string) (slog.Level, error) {
	switch strings.ToLower(strings.TrimSpace(text)) {
	case "", "info":
		return slog.LevelInfo, nil
	case "debug":
		return slog.LevelDebug, nil
	case "warn":
		return slog.LevelWarn, nil
	case "error":
		return slog.LevelError, nil
	}
	return 0, fmt.Errorf("unknown log level %q (want debug, info, warn or error)", text)
}

// newLogger writes to w, which is stdout: stderr is reserved for the setup
// token. slog has one text layout, so full, compact and pretty look alike.
// There is no colour to turn off, so NO_COLOR needs no handling.
func newLogger(w io.Writer, format config.LogFormat, level slog.Level) *slog.Logger {
	handlerOptions := &slog.HandlerOptions{Level: level}
	if format == config.LogFormatJSON {
		return slog.New(slog.NewJSONHandler(w, handlerOptions))
	}
	return slog.New(slog.NewTextHandler(w, handlerOptions))
}
