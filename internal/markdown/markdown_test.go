package markdown

import (
	"regexp"
	"strings"
	"testing"
)

func TestCasesCapturedFromRust(t *testing.T) {
	if len(rustCases) != 112 {
		t.Fatalf("%d cases, want 112", len(rustCases))
	}
	for _, c := range rustCases {
		check := func(name, got, want string) {
			if got != want {
				t.Errorf("%s(%q):\n got %q\nwant %q", name, c.input, got, want)
			}
		}
		check("Render", Render(c.input), c.render)
		check("ToPlain", ToPlain(c.input), c.toPlain)
		check("TruncatePlain(5)", TruncatePlain(c.input, 5), c.truncatePlain5)
		check("TruncatePlain(20)", TruncatePlain(c.input, 20), c.truncatePlain20)
		check("TruncatePlain(0)", TruncatePlain(c.input, 0), c.truncatePlain0)
	}
}

func TestEmptyStringRendersEmpty(t *testing.T) {
	if Render("") != "" || ToPlain("") != "" || TruncatePlain("", 10) != "" {
		t.Fatal("empty input must render empty")
	}
}

func TestParagraphWithSingleNewlineBecomesBr(t *testing.T) {
	assertRender(t, "line one\nline two", "<p>line one<br>line two</p>")
}

func TestBlankLineSeparatesParagraphs(t *testing.T) {
	assertRender(t, "first\n\nsecond", "<p>first</p><p>second</p>")
}

func TestDashListBecomesUl(t *testing.T) {
	assertRender(t, "- one\n- two\n- three", "<ul><li>one</li><li>two</li><li>three</li></ul>")
}

func TestMixedBlockIsNotAList(t *testing.T) {
	assertRender(t, "- one\ntwo", "<p>- one<br>two</p>")
}

func TestBoldRendersStrong(t *testing.T) {
	assertRender(t, "**hi**", "<p><strong>hi</strong></p>")
}

func TestItalicRendersEm(t *testing.T) {
	assertRender(t, "*hi*", "<p><em>hi</em></p>")
}

func TestUnmatchedBoldOpenerIsLiteralNotEmptyEm(t *testing.T) {
	assertRender(t, "**bold", "<p>**bold</p>")
}

func TestUnmatchedBoldOpenerThenRealItalic(t *testing.T) {
	assertRender(t, "**a*b*", "<p>**a<em>b</em></p>")
}

func TestBoldTakesPriorityOverItalic(t *testing.T) {
	assertRender(t, "**bold** and *italic*", "<p><strong>bold</strong> and <em>italic</em></p>")
}

func TestInlineCodeRendersCodeTag(t *testing.T) {
	assertRender(t, "`x`", "<p><code>x</code></p>")
}

func TestCodeContentIsNotFurtherTransformed(t *testing.T) {
	assertRender(t, "`**x**`", "<p><code>**x**</code></p>")
}

func TestUnmatchedTrailingBacktickIsLiteral(t *testing.T) {
	assertRender(t, "a `b", "<p>a `b</p>")
}

func TestExplicitLinkRendersAnchor(t *testing.T) {
	assertRender(t, "[site](https://example.com)",
		`<p><a href="https://example.com" rel="noopener noreferrer">site</a></p>`)
}

func TestMailtoLinkIsAllowed(t *testing.T) {
	assertRender(t, "[mail](mailto:a@example.com)",
		`<p><a href="mailto:a@example.com" rel="noopener noreferrer">mail</a></p>`)
}

func TestJavascriptLinkIsLiteralNoAnchor(t *testing.T) {
	out := Render("[click](javascript:alert(1))")
	if strings.Contains(out, "<a") || strings.Contains(out, `javascript:alert(1))">`) {
		t.Fatalf("javascript: link became an anchor: %q", out)
	}
}

func TestDataLinkIsLiteralNoAnchor(t *testing.T) {
	if strings.Contains(Render("[img](data:text/html,evil)"), "<a") {
		t.Fatal("data: link became an anchor")
	}
}

func TestBareUrlAutolinks(t *testing.T) {
	assertRender(t, "see https://example.com/x for more",
		`<p>see <a href="https://example.com/x" rel="noopener noreferrer">https://example.com/x</a> for more</p>`)
}

func TestBareUrlTrimsTrailingPunctuation(t *testing.T) {
	assertRender(t, "visit https://example.com.",
		`<p>visit <a href="https://example.com" rel="noopener noreferrer">https://example.com</a>.</p>`)
}

func TestHeadingIsNotSupportedRendersLiteral(t *testing.T) {
	assertRender(t, "# heading", "<p># heading</p>")
}

func TestImageIsNotSupportedRendersLiteral(t *testing.T) {
	if strings.Contains(Render("![alt](https://example.com/x.png)"), "<img") {
		t.Fatal("image syntax produced an <img>")
	}
}

func TestBlockquoteIsNotSupportedRendersLiteral(t *testing.T) {
	assertRender(t, "> quoted", "<p>&gt; quoted</p>")
}

func TestUnderscoreEmphasisIsNotSupported(t *testing.T) {
	assertRender(t, "_not italic_", "<p>_not italic_</p>")
}

func TestNestedListIsNotSupportedStaysFlatText(t *testing.T) {
	if strings.Contains(Render("- one\n  - two"), "<ul><li><ul>") {
		t.Fatal("nested list produced a nested ul")
	}
}

func TestScriptTagIsEscapedNotExecuted(t *testing.T) {
	out := Render("<script>alert(1)</script>")
	if strings.Contains(out, "<script>") || !strings.Contains(out, "&lt;script&gt;") {
		t.Fatalf("script tag not escaped: %q", out)
	}
}

func TestImgOnerrorIsEscaped(t *testing.T) {
	out := Render("<img src=x onerror=alert(1)>")
	if strings.Contains(out, "<img") || strings.Contains(out, "onerror=alert(1)>") || !strings.Contains(out, "&lt;img") {
		t.Fatalf("img tag not escaped: %q", out)
	}
}

func TestQuotesAreEscaped(t *testing.T) {
	out := Render(`she said "hi" and 'bye'`)
	if !strings.Contains(out, "&quot;hi&quot;") || !strings.Contains(out, "&#39;bye&#39;") {
		t.Fatalf("quotes not escaped: %q", out)
	}
}

func TestAmpersandIsEscapedFirst(t *testing.T) {
	assertRender(t, "<", "<p>&lt;</p>")
	assertRender(t, "&lt;", "<p>&amp;lt;</p>")
}

func TestToPlainStripsMarkersAndCollapsesWhitespace(t *testing.T) {
	got := ToPlain("**bold**  *italic*\n\n- one\n- two `code`")
	if got != "bold italic one two code" {
		t.Fatalf("got %q", got)
	}
}

func TestToPlainReducesLinkToText(t *testing.T) {
	if got := ToPlain("[site](https://example.com)"); got != "site" {
		t.Fatalf("got %q", got)
	}
}

func TestTruncatePlainAppendsEllipsisWhenTruncated(t *testing.T) {
	if got := TruncatePlain("hello world", 5); got != "hello…" {
		t.Fatalf("got %q", got)
	}
	if got := TruncatePlain("hello", 5); got != "hello" {
		t.Fatalf("got %q", got)
	}
}

func TestTruncatePlainIsCharBoundarySafeOnMultibyte(t *testing.T) {
	out := TruncatePlain("你好世界这是一段測試文字用來確認多位元組字元不會導致崩潰", 5)
	if len([]rune(out)) != 6 || !strings.HasSuffix(out, "…") {
		t.Fatalf("got %q", out)
	}
}

func assertRender(t *testing.T, source, want string) {
	t.Helper()
	if got := Render(source); got != want {
		t.Fatalf("Render(%q) = %q, want %q", source, got, want)
	}
}

var allowedTags = regexp.MustCompile(`</?(p|ul|li|strong|em|code|br)>|<a href="(https?://|mailto:)[^"<>]*" rel="noopener noreferrer">|</a>`)

// 待 Opus 審查：XSS. Whatever the input, once the whitelisted tags are removed
// no markup may remain, and no href may break out of its attribute.
func TestOutputContainsOnlyWhitelistedMarkup(t *testing.T) {
	hostile := []string{
		`<script>alert(1)</script>`, `[x](https://a.b" onmouseover="alert(1))`, `[x](https://a.b><script>)`,
		`https://a.b"onmouseover="x`, `https://a.b<img src=x onerror=alert(1)>`, "[x](javascript:alert(1))",
		"[<img src=x onerror=1>](https://a.b)", "**<b>**", "`<script>`", "- <script>", "[x](HTTPS://A.B)",
		"[a](mailto:\"><script>)", "&lt;script&gt;", "\u2028<script>", "[x](https://a.b\n)",
	}
	for _, c := range rustCases {
		hostile = append(hostile, c.input)
	}
	for _, input := range hostile {
		out := Render(input)
		if leftover := allowedTags.ReplaceAllString(out, ""); strings.ContainsAny(leftover, "<>\"'") {
			t.Errorf("Render(%q) leaks markup %q", input, out)
		}
	}
}
