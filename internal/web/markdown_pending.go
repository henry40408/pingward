package web

import (
	"html"
	"strings"
)

// Step 10 (internal/markdown) is blocked on its fixture. Until it lands these
// two stand-ins keep pages rendering: no markdown syntax is interpreted, so
// descriptions show as escaped plain text. Replace the bodies, not the callers.

// renderMarkdown stands in for markdown.Render.
func renderMarkdown(source string) string {
	trimmed := strings.TrimSpace(source)
	if trimmed == "" {
		return ""
	}
	return "<p>" + html.EscapeString(trimmed) + "</p>"
}

// plainSummary stands in for markdown.TruncatePlain: one line, at most limit
// characters plus an ellipsis.
func plainSummary(source string, limit int) string {
	plain := strings.Join(strings.Fields(source), " ")
	runes := []rune(plain)
	if len(runes) <= limit {
		return plain
	}
	return string(runes[:limit]) + "…"
}
