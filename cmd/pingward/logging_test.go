package main

import (
	"bytes"
	"log/slog"
	"strings"
	"testing"

	"github.com/henry40408/pingward/internal/config"
)

func TestParseLogLevel(t *testing.T) {
	cases := map[string]slog.Level{
		"": slog.LevelInfo, "info": slog.LevelInfo, "INFO": slog.LevelInfo, "  debug ": slog.LevelDebug,
		"warn": slog.LevelWarn, "Error": slog.LevelError,
	}
	for text, want := range cases {
		got, err := parseLogLevel(text)
		if err != nil || got != want {
			t.Errorf("%q gave %v, %v", text, got, err)
		}
	}
}

func TestParseLogLevelRejectsFilterSyntaxAndUnknownNames(t *testing.T) {
	for _, text := range []string{"trace", "pingward=debug", "warning", "info,sqlx=warn"} {
		if _, err := parseLogLevel(text); err == nil {
			t.Errorf("%q must fail", text)
		}
	}
}

func TestNewLoggerJSONAndText(t *testing.T) {
	var buf bytes.Buffer
	newLogger(&buf, config.LogFormatJSON, slog.LevelInfo).Info("hello")
	if !strings.HasPrefix(buf.String(), "{") || !strings.Contains(buf.String(), `"msg":"hello"`) {
		t.Fatalf("json output %q", buf.String())
	}
	for _, format := range []config.LogFormat{config.LogFormatFull, config.LogFormatCompact, config.LogFormatPretty} {
		buf.Reset()
		newLogger(&buf, format, slog.LevelInfo).Info("hello")
		if !strings.Contains(buf.String(), "msg=hello") {
			t.Fatalf("text output %q", buf.String())
		}
	}
}

func TestNewLoggerHonoursLevel(t *testing.T) {
	var buf bytes.Buffer
	newLogger(&buf, config.LogFormatFull, slog.LevelWarn).Info("quiet")
	if buf.Len() != 0 {
		t.Fatalf("info leaked at warn level: %q", buf.String())
	}
}
