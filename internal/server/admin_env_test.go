package server_test

import (
	"net/http"
	"strings"
	"testing"
)

func TestAdminEnvCardNeverLeaksSecrets(t *testing.T) {
	a := newApp(t, map[string]string{
		"DATABASE_URL":           "postgres://pguser:pgsecret@db.internal:5432/pingward",
		"PINGWARD_SMTP_HOST":     "smtp.example.com",
		"PINGWARD_SMTP_FROM":     "alerts@example.com",
		"PINGWARD_SMTP_PASSWORD": "hunter2-smtp",
	})
	a.loggedInAdmin(t)
	res := a.client.Get("/admin")
	wantStatus(t, res, http.StatusOK)
	for _, leaked := range []string{"hunter2-smtp", "pgsecret", "pguser"} {
		if strings.Contains(res.Body, leaked) {
			t.Errorf("the page leaks %q", leaked)
		}
	}
	for _, want := range []string{"configured", "db.internal", "smtp.example.com", `data-testid="env-smtp-password"`} {
		if !strings.Contains(res.Body, want) {
			t.Errorf("the page is missing %q", want)
		}
	}
}

func TestAdminEnvCardShowsUnsetSMTPWhenNotConfigured(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	body := a.client.Get("/admin").Body
	if strings.Contains(body, "configured</span>") && !strings.Contains(body, "not set") {
		t.Fatal("unset SMTP must read as not set")
	}
	if !strings.Contains(body, `data-testid="env-row-PINGWARD_SMTP_HOST"`) {
		t.Fatal("the SMTP rows must still render")
	}
}
