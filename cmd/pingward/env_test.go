package main

import (
	"strings"
	"testing"
)

func lookupFrom(env map[string]string) func(string) (string, bool) {
	return func(key string) (string, bool) {
		value, ok := env[key]
		return value, ok
	}
}

func TestNoLegacyVariablesIsFine(t *testing.T) {
	if err := ensureNoLegacyEnvVars(lookupFrom(map[string]string{"PINGWARD_SECRET": "x"})); err != nil {
		t.Fatal(err)
	}
}

func TestRustLogFailsEvenWhenEmpty(t *testing.T) {
	for _, value := range []string{"", "debug"} {
		err := ensureNoLegacyEnvVars(lookupFrom(map[string]string{"RUST_LOG": value}))
		if err == nil {
			t.Fatalf("RUST_LOG=%q must fail", value)
		}
		want := "these environment variables no longer exist; rename (or unset) them to continue:\n  RUST_LOG -> PINGWARD_LOG_LEVEL"
		if err.Error() != want {
			t.Fatalf("message %q", err)
		}
	}
}

func TestEveryLegacyVariableNamesItsReplacement(t *testing.T) {
	for _, legacy := range legacyEnvVars {
		if legacy.old == "" || !strings.HasPrefix(legacy.replacement, "PINGWARD_") {
			t.Errorf("bad entry %+v", legacy)
		}
	}
}
