package web

import (
	"fmt"

	"github.com/henry40408/pingward/internal/tz"
)

// validateTimezone: blank means UTC (the column default). A typo is rejected
// and echoed back, because a stored bad zone would silently fall back to UTC
// when the check's deadline is computed.
func validateTimezone(raw string) (zone, refusal string) {
	name := trimRustSpace(raw)
	if name == "" {
		return "UTC", ""
	}
	canonical, ok := tz.Canonical(name)
	if !ok {
		return "", fmt.Sprintf("Unknown timezone %q. Use an IANA name such as UTC or Asia/Taipei.", name)
	}
	return canonical, ""
}
