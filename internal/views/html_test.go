package views

import (
	"os"
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

func goldenShell(t *testing.T, name string) string {
	t.Helper()
	body, err := os.ReadFile("testdata/" + name)
	if err != nil {
		t.Fatal(err)
	}
	return normalise(string(body))
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
	golden := goldenShell(t, "login.golden.html")
	start := strings.Index(golden, "<main")
	end := strings.Index(golden, "</main>") + len("</main>")
	page := normalise(Page(Chrome{}, raw("BODY")))
	rebuilt := golden[:start] + `<main class="wrap">BODY</main>` + golden[end:]
	if page != rebuilt {
		t.Fatalf("\n%s\n!=\n%s", page, rebuilt)
	}
}

func TestPageWithNavMatchesTheRustDashboardNav(t *testing.T) {
	golden := goldenShell(t, "dashboard.golden.html")
	start := strings.Index(golden, "<header")
	end := strings.Index(golden, "</header>") + len("</header>")
	page := normalise(Page(Chrome{ShowNav: true, IsAdmin: true, CSRF: "c"}, raw("")))
	if !strings.Contains(page, golden[start:end]) {
		t.Fatalf("nav differs:\n%s\nin\n%s", golden[start:end], page)
	}
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
