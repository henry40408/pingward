package web

import (
	"strings"
	"testing"
)

func TestRedactDBURL(t *testing.T) {
	cases := map[string]string{
		"postgres://user:pw@host:5432/db":      "postgres://***@host:5432/db",
		"sqlite://pingward.sqlite3?mode=rwc":   "sqlite://pingward.sqlite3?mode=rwc",
		"sqlite:///var/data/a@b.db":            "sqlite:///var/data/a@b.db",
		"postgres://host/db?user=a@b":          "postgres://host/db?user=a@b",
		"postgres://host#frag@x":               "postgres://host#frag@x",
		"postgres://u:p@w@host/db":             "postgres://***@w@host/db",
		"pingward.sqlite3":                     "pingward.sqlite3",
		"postgres://user@host":                 "postgres://***@host",
		"postgres://us%40er:p@ss@host/db?x=@y": "postgres://***@ss@host/db?x=@y",
	}
	for input, want := range cases {
		if got := redactDBURL(input); got != want {
			t.Errorf("redactDBURL(%q) = %q, want %q", input, got, want)
		}
	}
}

// The Rust redact_db_url tests, one by one, so the map to src/web.rs stays 1:1.
func TestRedactDBURLLeavesAPlainSQLiteURLUnchanged(t *testing.T) {
	const url = "sqlite://pingward.sqlite3?mode=rwc"
	if got := redactDBURL(url); got != url {
		t.Errorf("got %q", got)
	}
}

func TestRedactDBURLLeavesABarePathUnchanged(t *testing.T) {
	if got := redactDBURL("pingward.sqlite3"); got != "pingward.sqlite3" {
		t.Errorf("got %q", got)
	}
}

func TestRedactDBURLStripsUserAndPassword(t *testing.T) {
	got := redactDBURL("postgres://user:pass@host/db")
	if got != "postgres://***@host/db" || strings.Contains(got, "pass") || strings.Contains(got, "user") {
		t.Errorf("got %q", got)
	}
}

func TestRedactDBURLStripsBareUsernameWithNoPassword(t *testing.T) {
	got := redactDBURL("postgres://user@host/db")
	if got != "postgres://***@host/db" || strings.Contains(got, "user@") {
		t.Errorf("got %q", got)
	}
}

func TestRedactDBURLIgnoresAnAtSignOnlyInTheQueryString(t *testing.T) {
	const url = "postgres://host/db?target_session_attrs=x&opt=a@b"
	if got := redactDBURL(url); got != url {
		t.Errorf("got %q", got)
	}
}

func TestRedactDBURLEmptyStringIsUnchanged(t *testing.T) {
	if got := redactDBURL(""); got != "" {
		t.Errorf("got %q", got)
	}
}
