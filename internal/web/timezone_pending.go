package web

import (
	"fmt"
	"time"
)

// validateTimezone stands in for the chrono-tz lookup until the timezone
// list lands with Step 9. It uses the Go runtime's zone data, which is
// case-sensitive where chrono-tz is not; the canonical name is the loaded one.
func validateTimezone(raw string) (string, error) {
	name := trimRustSpace(raw)
	if name == "" {
		return "UTC", nil
	}
	location, err := time.LoadLocation(name)
	if err != nil || name == "Local" {
		return "", fmt.Errorf("unknown timezone %q — use an IANA name such as UTC or Asia/Taipei", name)
	}
	return location.String(), nil
}
