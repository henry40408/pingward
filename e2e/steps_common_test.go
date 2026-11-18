package e2e

import "strings"

// quoted matches a double-quoted step argument, keeping backslash escapes for
// unquote.
const quoted = `"((?:[^"\\]|\\.)*)"`

// unquote undoes the backslash escaping of a quoted argument. Only \" and \\
// are recognised, so other backslashes survive intact.
func unquote(value string) string {
	var out strings.Builder
	for i := 0; i < len(value); i++ {
		if value[i] == '\\' && i+1 < len(value) && (value[i+1] == '"' || value[i+1] == '\\') {
			i++
		}
		out.WriteByte(value[i])
	}
	return out.String()
}
