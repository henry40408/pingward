package testutil

import (
	"context"
	"testing"

	"github.com/henry40408/pingward/internal/config"
	"github.com/henry40408/pingward/internal/secret"
	"github.com/henry40408/pingward/internal/store"
)

// TestSecret pins the session and CSRF secret so helpers can derive tokens
// without a Config. Rotation tests build their own.
const TestSecret = "pingward-test-secret-32-bytes-xx"

// TestSetupToken is the setup token tests arm; the real one is random.
const TestSetupToken = "TEST0-TOKEN"

// TestConfig is the default Config, pinned to TestSecret. Extra variables
// override or add to the environment.
func TestConfig(extra map[string]string) *config.Config {
	cfg := config.Load(func(key string) (string, bool) {
		if value, ok := extra[key]; ok {
			return value, true
		}
		if key == "PINGWARD_SECRET" {
			return TestSecret, true
		}
		return "", false
	})
	return &cfg
}

// SignedFlashCookie is "name=value" as a TestConfig server sets it: signed
// under TestSecret, unprefixed name. The server ignores unsigned values.
func SignedFlashCookie(value string) string {
	return "pingward_flash=" + secret.SignFlash([]byte(TestSecret), value)
}

// NewestSessionCSRF derives the CSRF token for the newest session row. It
// orders by rowid: sessions in one test share a second, so timestamps cannot
// tell them apart. SQLite only.
func NewestSessionCSRF(t *testing.T, s *store.Store) string {
	t.Helper()
	var id string
	err := s.DB.QueryRowContext(context.Background(),
		"SELECT id FROM sessions ORDER BY rowid DESC LIMIT 1").Scan(&id)
	if err != nil {
		t.Fatalf("a session row should exist: %v", err)
	}
	return secret.DeriveCSRF([]byte(TestSecret), id)
}

// AnonymousCSRF starts a fresh anonymous session and returns its CSRF token,
// for the _csrf field of POST /login and /setup. It reads the cookie because an
// anonymous session has no row; cookies are cleared first so this request
// mints one. probePath is any web page, such as "/login".
func AnonymousCSRF(t *testing.T, c *Client, probePath string) string {
	t.Helper()
	c.ClearCookies()
	c.Get(probePath)
	for _, name := range []string{"__Host-pingward_session", "pingward_session"} {
		value := c.Cookie(name)
		if value == "" {
			continue
		}
		id, ok := secret.VerifySession([]byte(TestSecret), value)
		if !ok {
			t.Fatal("the anonymous cookie should be signed with the test secret")
		}
		return secret.DeriveCSRF([]byte(TestSecret), id)
	}
	t.Fatal("the anonymous-session layer should set a cookie on a session-less request")
	return ""
}
