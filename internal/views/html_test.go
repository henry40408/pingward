package views

import (
	"regexp"
	"strings"
	"testing"

	"github.com/henry40408/pingward/internal/assets"
)

func TestEscapeCoversTheFourMaudCharactersAndLeavesApostrophes(t *testing.T) {
	if got, want := string(text(`<a href="x">&'</a>`)), `&lt;a href=&quot;x&quot;&gt;&amp;'&lt;/a&gt;`; got != want {
		t.Fatalf("%s != %s", got, want)
	}
}

func TestAttributeValuesAreEscapedAndFlagsRenderBare(t *testing.T) {
	got := string(el("script", []attr{a("src", `a"b`), flag("defer")}))
	if want := `<script src="a&quot;b" defer></script>`; got != want {
		t.Fatalf("%s != %s", got, want)
	}
}

func TestVoidElementsHaveNoClosingTag(t *testing.T) {
	if got, want := string(void("input", a("name", "n"))), `<input name="n">`; got != want {
		t.Fatalf("%s != %s", got, want)
	}
}

func TestRawIsNotEscapedButTextIs(t *testing.T) {
	if got := string(group(raw("<b>"), text("<b>"))); got != "<b>&lt;b&gt;" {
		t.Fatal(got)
	}
}

var (
	csrfValue   = regexp.MustCompile(`name="_csrf" value="[^"]*"`)
	versionArgs = regexp.MustCompile(`\?v=[0-9a-f]+`)
	footerBuild = regexp.MustCompile(`pingward [^<]*</span>`)
)

func normalise(html string) string {
	html = csrfValue.ReplaceAllString(html, `name="_csrf" value="X"`)
	html = versionArgs.ReplaceAllString(html, "?v=V")
	return footerBuild.ReplaceAllString(html, "pingward VER</span>")
}

func TestPageWithoutNavMatchesTheRustLoginShell(t *testing.T) {
	assertGolden(t, "login-shell", Page(Chrome{}, raw("BODY")))
}

func TestPageWithNavMatchesTheRustDashboardNav(t *testing.T) {
	page := Page(Chrome{ShowNav: true, IsAdmin: true, CSRF: "c"}, raw(""))
	start := strings.Index(page, "<header")
	end := strings.Index(page, "</header>") + len("</header>")
	if start < 0 || end < len("</header>") {
		t.Fatalf("no <header> in %s", page)
	}
	assertGolden(t, "dashboard-nav", page[start:end])
}

func TestNavHidesAdminLinkFromNonAdmins(t *testing.T) {
	page := Page(Chrome{ShowNav: true}, raw(""))
	if strings.Contains(page, "nav-admin") || !strings.Contains(page, "nav-account") {
		t.Fatal(page)
	}
}

func TestPageLinksTheContentHashedAssets(t *testing.T) {
	page := Page(Chrome{}, raw(""))
	for _, want := range []string{
		"/assets/app.css?v=" + assets.CSSVersion(),
		"/favicon.svg?v=" + assets.IconVersion(),
		"/apple-touch-icon.png?v=" + assets.IconVersion(),
		"/assets/app.js?v=" + assets.JSVersion(),
	} {
		if !strings.Contains(page, want) {
			t.Fatalf("missing %s", want)
		}
	}
}

func TestFooterShowsTheBuildVersionEvenWithoutNav(t *testing.T) {
	page := Page(Chrome{}, raw(""))
	if !strings.Contains(page, `<span data-testid="app-version">pingward `) || strings.Contains(page, "nav-account") {
		t.Fatal(page)
	}
}
