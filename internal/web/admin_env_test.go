package web

import "testing"

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
