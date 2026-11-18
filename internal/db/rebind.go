package db

import "strings"

// Rebind rewrites Postgres-style $N placeholders for the dialect. SQLite
// treats "$1" as a *named* parameter and only "?NNN" as a numbered one, so the
// SQLite form is rewritten before execution. String literals, quoted
// identifiers and comments are left alone.
func (d Dialect) Rebind(query string) string {
	if d != SQLite || !strings.Contains(query, "$") {
		return query
	}
	var out strings.Builder
	out.Grow(len(query))
	for i := 0; i < len(query); {
		switch c := query[i]; {
		case c == '\'' || c == '"':
			end := skipQuoted(query, i)
			out.WriteString(query[i:end])
			i = end
		case c == '-' && strings.HasPrefix(query[i:], "--"):
			end := indexOrEnd(query, i, "\n")
			out.WriteString(query[i:end])
			i = end
		case c == '/' && strings.HasPrefix(query[i:], "/*"):
			end := indexOrEnd(query, i+2, "*/")
			if end < len(query) {
				end += 2
			}
			out.WriteString(query[i:end])
			i = end
		case c == '$' && i+1 < len(query) && isDigit(query[i+1]):
			out.WriteByte('?')
			i++
		default:
			out.WriteByte(c)
			i++
		}
	}
	return out.String()
}

// skipQuoted returns the index just past the quoted run that starts at start.
// A doubled quote character is an escaped quote, not the end.
func skipQuoted(query string, start int) int {
	quote := query[start]
	for i := start + 1; i < len(query); i++ {
		if query[i] != quote {
			continue
		}
		if i+1 < len(query) && query[i+1] == quote {
			i++
			continue
		}
		return i + 1
	}
	return len(query)
}

func indexOrEnd(query string, from int, marker string) int {
	index := strings.Index(query[from:], marker)
	if index < 0 {
		return len(query)
	}
	return from + index
}

func isDigit(c byte) bool { return c >= '0' && c <= '9' }
