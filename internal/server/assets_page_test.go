package server_test

import (
	"net/http"
	"strings"
	"testing"

	"github.com/henry40408/pingward/internal/assets"
	"github.com/henry40408/pingward/internal/version"
)

// /setup is a nav-less page that renders without a session.
func setupPage(t *testing.T) string {
	t.Helper()
	res := newApp(t, nil).client.Get("/setup")
	wantStatus(t, res, http.StatusOK)
	return res.Body
}

func TestPagesLinkTheContentHashedIcons(t *testing.T) {
	if assets.IconVersion() == "" {
		t.Fatal("icon version must not be empty")
	}
	body := setupPage(t)
	for _, want := range []string{
		"/favicon.svg?v=" + assets.IconVersion(),
		"/apple-touch-icon.png?v=" + assets.IconVersion(),
	} {
		if !strings.Contains(body, want) {
			t.Errorf("versioned icon link missing from the rendered page: %s", want)
		}
	}
}

func TestPagesLinkTheContentHashedStylesheet(t *testing.T) {
	if assets.CSSVersion() == "" {
		t.Fatal("css version must not be empty")
	}
	if want := "/assets/app.css?v=" + assets.CSSVersion(); !strings.Contains(setupPage(t), want) {
		t.Fatalf("versioned stylesheet link missing: %s", want)
	}
}

// The footer sits outside the nav guard, so it must render on a nav-less
// page; the second check pins that /setup is still one.
func TestNavLessPagesStillRenderTheBuildVersion(t *testing.T) {
	if version.Version() == "" {
		t.Fatal("build version must not be empty")
	}
	body := setupPage(t)
	if want := `<span data-testid="app-version">pingward ` + version.Version() + `</span>`; !strings.Contains(body, want) {
		t.Fatalf("build version missing from the footer: %s", want)
	}
	if strings.Contains(body, `data-testid="nav-account"`) {
		t.Fatal("/setup grew a nav: pick another nav-less page for this test")
	}
}
