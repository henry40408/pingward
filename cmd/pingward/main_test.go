package main

import (
	"bytes"
	"strings"
	"testing"
)

func runWith(env map[string]string) (code int, stdout, stderr string) {
	var out, errOut bytes.Buffer
	code = run(process{lookupEnv: lookupFrom(env), stdout: &out, stderr: &errOut})
	return code, out.String(), errOut.String()
}

func TestRunSucceedsWithoutConfiguration(t *testing.T) {
	code, _, stderr := runWith(nil)
	if code != 0 || stderr != "" {
		t.Fatalf("code %d, stderr %q", code, stderr)
	}
}

func TestRunRefusesRustLogBeforeReadingAnythingElse(t *testing.T) {
	code, stdout, stderr := runWith(map[string]string{"RUST_LOG": "", "PINGWARD_LOG_LEVEL": "nonsense"})
	if code == 0 {
		t.Fatal("must fail")
	}
	if !strings.Contains(stderr, "RUST_LOG -> PINGWARD_LOG_LEVEL") || stdout != "" {
		t.Fatalf("stdout %q stderr %q", stdout, stderr)
	}
}

func TestRunFailsOnUnknownLogLevel(t *testing.T) {
	code, _, stderr := runWith(map[string]string{"PINGWARD_LOG_LEVEL": "loud"})
	if code == 0 || !strings.Contains(stderr, `unknown log level "loud"`) {
		t.Fatalf("code %d stderr %q", code, stderr)
	}
}

func TestRunLogsEphemeralSecretWarningToStdout(t *testing.T) {
	code, stdout, stderr := runWith(nil)
	if code != 0 || stderr != "" || !strings.Contains(stdout, "PINGWARD_SECRET is not set") {
		t.Fatalf("code %d stdout %q stderr %q", code, stdout, stderr)
	}
}

func TestRunStaysQuietWithAGoodSecret(t *testing.T) {
	_, stdout, _ := runWith(map[string]string{"PINGWARD_SECRET": "0123456789abcdef"})
	if stdout != "" {
		t.Fatalf("stdout %q", stdout)
	}
}

func TestRunLogsOrphanLogoutURLWarningAsJSON(t *testing.T) {
	_, stdout, _ := runWith(map[string]string{
		"PINGWARD_SECRET":                  "0123456789abcdef",
		"PINGWARD_LOG_FORMAT":              "json",
		"PINGWARD_FORWARD_AUTH_LOGOUT_URL": "https://auth/logout",
	})
	if !strings.HasPrefix(stdout, "{") || !strings.Contains(stdout, "PINGWARD_FORWARD_AUTH_LOGOUT_URL is set") {
		t.Fatalf("stdout %q", stdout)
	}
}
