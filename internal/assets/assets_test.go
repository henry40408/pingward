package assets

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func get(path string) *httptest.ResponseRecorder {
	recorder := httptest.NewRecorder()
	Handler().ServeHTTP(recorder, httptest.NewRequest(http.MethodGet, path, nil))
	return recorder
}

func TestServesTheStylesheet(t *testing.T) {
	res := get("/assets/app.css")
	if res.Code != 200 || res.Header().Get("Content-Type") != "text/css; charset=utf-8" ||
		!strings.Contains(res.Body.String(), "--brand") {
		t.Fatalf("%d %v", res.Code, res.Header())
	}
}

func TestUnknownFontFile404s(t *testing.T) {
	if res := get("/assets/fonts/does-not-exist.woff2"); res.Code != 404 {
		t.Fatal(res.Code)
	}
}

func TestUnknownScript404s(t *testing.T) {
	if res := get("/assets/nope.js"); res.Code != 404 {
		t.Fatal(res.Code)
	}
}

func TestServesAFont(t *testing.T) {
	res := get("/assets/fonts/inter-400.woff2")
	if res.Code != 200 || res.Header().Get("Content-Type") != "font/woff2" {
		t.Fatalf("%d %v", res.Code, res.Header())
	}
}

func TestServesBothScripts(t *testing.T) {
	for _, name := range []string{"app.js", "theme-init.js"} {
		res := get("/assets/" + name)
		if res.Code != 200 || res.Header().Get("Content-Type") != "text/javascript; charset=utf-8" || res.Body.Len() == 0 {
			t.Fatalf("%s: %d %v", name, res.Code, res.Header())
		}
	}
}

func TestEveryAssetIsCachedImmutably(t *testing.T) {
	for _, path := range []string{"/assets/app.css", "/assets/app.js", "/assets/fonts/inter-400.woff2", "/favicon.svg"} {
		if got := get(path).Header().Get("Cache-Control"); got != "public, max-age=31536000, immutable" {
			t.Fatalf("%s: %s", path, got)
		}
	}
}

func TestStylesheetFontURLsAreCacheBusted(t *testing.T) {
	css := get("/assets/app.css").Body.String()
	if strings.Contains(css, "{{") {
		t.Fatal("unsubstituted placeholder in the served stylesheet")
	}
	if !strings.Contains(css, "/assets/fonts/inter-400.woff2?v="+FontVersion()) {
		t.Fatal("font URL is not cache-busted")
	}
}

func TestServesTheAppIcons(t *testing.T) {
	svg := get("/favicon.svg")
	if svg.Code != 200 || svg.Header().Get("Content-Type") != "image/svg+xml" || !strings.Contains(svg.Body.String(), "<svg") {
		t.Fatalf("%d %v", svg.Code, svg.Header())
	}
	png := get("/apple-touch-icon.png")
	if png.Code != 200 || png.Header().Get("Content-Type") != "image/png" ||
		!bytes.HasPrefix(png.Body.Bytes(), []byte("\x89PNG\r\n\x1a\n")) {
		t.Fatalf("%d %v", png.Code, png.Header())
	}
}

func TestVersionsAreNonEmptyAndTheCSSVersionFollowsTheFonts(t *testing.T) {
	for _, v := range []string{CSSVersion(), JSVersion(), IconVersion(), FontVersion()} {
		if v == "" {
			t.Fatal("empty version")
		}
	}
	if hashBytes([]byte(strings.ReplaceAll(string(read("app.css")), fontPlaceholder, "other"))) == CSSVersion() {
		t.Fatal("the stylesheet version must depend on the font version")
	}
}
