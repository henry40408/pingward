// Package markdown renders the minimal, escape-first markdown subset used for
// project and check descriptions (port of src/markdown.rs).
//
// Render HTML-escapes the whole input first and then emits only whitelisted
// tags built from the escaped text, so raw HTML injection is structurally
// impossible. Do not swap this for a sanitiser pass over generated HTML.
//
// Supported: **bold**, *italic*, `code`, [text](url) (only http(s):// and
// mailto: become links), bare http(s):// autolinks and "- " lists (every line
// of the block). Anything else renders as literal text. Inline constructs do
// not nest and generated output is never re-scanned.
//
// Render is worst-case O(n^2) (repeated unclosed "["), which is only safe
// because descriptions are capped at 2000 characters by the web layer.
package markdown

import (
	"strings"
	"unicode"
	"unicode/utf8"
)

// Render turns user markdown into the whitelisted HTML subset.
func Render(source string) string {
	escaped := escapeHTML(normalizeNewlines(source))
	var out strings.Builder
	for _, block := range splitBlocks(escaped) {
		if isListBlock(block) {
			renderList(&out, block)
			continue
		}
		rendered := make([]string, len(block))
		for i, line := range block {
			rendered[i] = renderInline(line)
		}
		out.WriteString("<p>" + strings.Join(rendered, "<br>") + "</p>")
	}
	return out.String()
}

func renderList(out *strings.Builder, block []string) {
	out.WriteString("<ul>")
	for _, line := range block {
		trimmed := trimStart(line)
		content := strings.TrimPrefix(trimmed, "- ")
		out.WriteString("<li>" + renderInline(content) + "</li>")
	}
	out.WriteString("</ul>")
}

// ToPlain strips markdown markers and collapses whitespace. The result is not
// HTML-escaped: the view layer escapes on render.
func ToPlain(source string) string {
	stripped := stripMarkdownMarkers(normalizeNewlines(source))
	return strings.Join(strings.Fields(stripped), " ")
}

// TruncatePlain is ToPlain cut to maxChars characters (not bytes) plus "…".
func TruncatePlain(source string, maxChars int) string {
	plain := ToPlain(source)
	if utf8.RuneCountInString(plain) <= maxChars {
		return plain
	}
	return string([]rune(plain)[:maxChars]) + "…"
}

func normalizeNewlines(s string) string {
	return strings.ReplaceAll(strings.ReplaceAll(s, "\r\n", "\n"), "\r", "\n")
}

var htmlEscaper = strings.NewReplacer("&", "&amp;", "<", "&lt;", ">", "&gt;", `"`, "&quot;", "'", "&#39;")

func escapeHTML(s string) string { return htmlEscaper.Replace(s) }

func trimStart(s string) string { return strings.TrimLeftFunc(s, unicode.IsSpace) }

// splitBlocks splits on blank lines; each block is non-empty.
func splitBlocks(s string) [][]string {
	var blocks [][]string
	var current []string
	for _, line := range strings.Split(s, "\n") {
		if strings.TrimSpace(line) != "" {
			current = append(current, line)
			continue
		}
		if len(current) > 0 {
			blocks = append(blocks, current)
			current = nil
		}
	}
	if len(current) > 0 {
		blocks = append(blocks, current)
	}
	return blocks
}

func isListBlock(block []string) bool {
	for _, line := range block {
		if !strings.HasPrefix(trimStart(line), "- ") {
			return false
		}
	}
	return true
}

// isAllowedScheme lists the only schemes that become a live <a href>.
func isAllowedScheme(url string) bool {
	return strings.HasPrefix(url, "http://") || strings.HasPrefix(url, "https://") || strings.HasPrefix(url, "mailto:")
}

// renderInline is one left-to-right pass: code, bold, italic, link, autolink,
// else a literal character.
func renderInline(line string) string {
	var out strings.Builder
	rest := line
	for rest != "" {
		consumed, rendered := matchInline(rest)
		if consumed > 0 {
			out.WriteString(rendered)
			rest = rest[consumed:]
			continue
		}
		r, size := utf8.DecodeRuneInString(rest)
		out.WriteRune(r)
		rest = rest[size:]
	}
	return out.String()
}

// matchInline reports the construct starting rest, or 0 consumed for a literal.
func matchInline(rest string) (int, string) {
	switch {
	case strings.HasPrefix(rest, "`"):
		return matchDelimited(rest, "`", "code")
	case strings.HasPrefix(rest, "**"):
		if consumed, rendered := matchDelimited(rest, "**", "strong"); consumed > 0 {
			return consumed, rendered
		}
		// An unclosed "**" is literal; the italic rule would emit <em></em>.
		return 2, "**"
	case strings.HasPrefix(rest, "*"):
		return matchDelimited(rest, "*", "em")
	case strings.HasPrefix(rest, "["):
		return matchLink(rest)
	case strings.HasPrefix(rest, "http://") || strings.HasPrefix(rest, "https://"):
		return matchAutolink(rest)
	}
	return 0, ""
}

// matchDelimited renders marker content marker as <tag>content</tag>.
func matchDelimited(rest, marker, tag string) (int, string) {
	after := rest[len(marker):]
	end := strings.Index(after, marker)
	if end < 0 {
		return 0, ""
	}
	return len(marker) + end + len(marker), "<" + tag + ">" + after[:end] + "</" + tag + ">"
}

// matchLink renders [text](url) as an anchor, or as literal text for a scheme
// outside the whitelist.
func matchLink(rest string) (int, string) {
	afterBracket := rest[1:]
	closeIndex := strings.Index(afterBracket, "]")
	if closeIndex < 0 {
		return 0, ""
	}
	linkText := afterBracket[:closeIndex]
	afterParen, found := strings.CutPrefix(afterBracket[closeIndex+1:], "(")
	if !found {
		return 0, ""
	}
	parenClose := strings.Index(afterParen, ")")
	if parenClose < 0 {
		return 0, ""
	}
	url := afterParen[:parenClose]
	consumed := 1 + closeIndex + 1 + 1 + parenClose + 1
	if !isAllowedScheme(url) {
		return consumed, "[" + linkText + "](" + url + ")"
	}
	return consumed, `<a href="` + url + `" rel="noopener noreferrer">` + linkText + "</a>"
}

// matchAutolink links a bare http(s):// run minus trailing punctuation. It
// always consumes at least one character, so the scan makes progress.
func matchAutolink(rest string) (int, string) {
	end := strings.IndexFunc(rest, unicode.IsSpace)
	if end < 0 {
		end = len(rest)
	}
	run := rest[:end]
	url := strings.TrimRight(run, ".,;:!?)")
	trailing := run[len(url):]
	if !strings.HasPrefix(url, "http://") && !strings.HasPrefix(url, "https://") {
		return end, run
	}
	return end, `<a href="` + url + `" rel="noopener noreferrer">` + url + "</a>" + trailing
}

// stripMarkdownMarkers drops "- " list markers, backticks, "*" emphasis and
// turns [text](url) into text.
func stripMarkdownMarkers(s string) string {
	lines := strings.Split(s, "\n")
	// Rust's lines() has no empty last element for a trailing newline.
	if len(lines) > 0 && lines[len(lines)-1] == "" {
		lines = lines[:len(lines)-1]
	}
	for i, line := range lines {
		if stripped, found := strings.CutPrefix(trimStart(line), "- "); found {
			lines[i] = stripped
		}
	}
	noCode := strings.ReplaceAll(strings.Join(lines, "\n"), "`", "")
	return stripLinksToText(strings.ReplaceAll(noCode, "*", ""))
}

// stripLinksToText turns [text](url) into text; malformed or unmatched
// brackets stay literal.
func stripLinksToText(s string) string {
	var out strings.Builder
	rest := s
	for {
		start := strings.Index(rest, "[")
		if start < 0 {
			out.WriteString(rest)
			return out.String()
		}
		out.WriteString(rest[:start])
		afterBracket := rest[start+1:]
		closeIndex := strings.Index(afterBracket, "]")
		if closeIndex < 0 {
			out.WriteString("[")
			rest = afterBracket
			continue
		}
		text := afterBracket[:closeIndex]
		afterClose := afterBracket[closeIndex+1:]
		afterParen, found := strings.CutPrefix(afterClose, "(")
		parenClose := strings.Index(afterParen, ")")
		if found && parenClose >= 0 {
			out.WriteString(text)
			rest = afterParen[parenClose+1:]
			continue
		}
		out.WriteString("[" + text + "]")
		rest = afterClose
	}
}
