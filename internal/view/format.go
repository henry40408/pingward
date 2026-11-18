// Package view holds the display helpers the pages share: status, heartbeat
// bars and time formatting. Markup lives in internal/views.
package view

import (
	"fmt"
	"time"
)

// FmtSecs renders a duration as "42s", "3m 05s" or "2h 07m".
func FmtSecs(secs int64) string {
	s := max(secs, 0)
	switch {
	case s < 60:
		return fmt.Sprintf("%ds", s)
	case s < 3600:
		return fmt.Sprintf("%dm %02ds", s/60, s%60)
	default:
		return fmt.Sprintf("%dh %02dm", s/3600, (s%3600)/60)
	}
}

// FmtUTC is the fallback text of a .localtime[data-ts] span, shown when app.js
// does not localise it; it names the zone so it is never mistaken for local time.
func FmtUTC(at time.Time) string {
	return at.UTC().Format("2006-01-02 15:04:05") + " UTC"
}

// FmtRelative renders "12m ago".
func FmtRelative(then, now time.Time) string {
	return coarse(now.Sub(then), "", " ago")
}

// FmtUntil is the forward mirror of FmtRelative: "in 45s", "in 12m".
func FmtUntil(then, now time.Time) string {
	return coarse(then.Sub(now), "in ", "")
}

func coarse(span time.Duration, prefix, suffix string) string {
	s := max(int64(span/time.Second), 0)
	switch {
	case s < 60:
		return fmt.Sprintf("%s%ds%s", prefix, s, suffix)
	case s < 3600:
		return fmt.Sprintf("%s%dm%s", prefix, s/60, suffix)
	case s < 86400:
		return fmt.Sprintf("%s%dh%s", prefix, s/3600, suffix)
	default:
		return fmt.Sprintf("%s%dd%s", prefix, s/86400, suffix)
	}
}

// DurationSuggestions are <datalist> hints for interval fields. Handlers still
// validate; every entry must parse as a positive duration.
func DurationSuggestions() []string {
	return []string{"30s", "1m", "5m", "15m", "30m", "1h", "6h", "12h", "1d", "7d"}
}

// ExpirySuggestions is DurationSuggestions at API-key-expiry scale.
func ExpirySuggestions() []string {
	return []string{"7d", "30d", "90d", "365d"}
}
