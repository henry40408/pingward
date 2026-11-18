// Package tz is the IANA zone list the Rust build got from chrono-tz.
package tz

import (
	"fmt"
	"time"
	_ "time/tzdata" // distroless images ship no zoneinfo
)

var known = func() map[string]bool {
	set := make(map[string]bool, len(names))
	for _, name := range names {
		set[name] = true
	}
	return set
}()

// Names lists every zone in chrono-tz's order, for the timezone datalist.
func Names() []string { return append([]string(nil), names...) }

// Canonical returns name when chrono-tz would parse it. chrono-tz is case
// sensitive and does not trim, so "utc" and "UTC " are refused.
func Canonical(name string) (string, bool) {
	return name, known[name]
}

// Load resolves a chrono-tz zone name to a Go location.
func Load(name string) (*time.Location, error) {
	if _, ok := Canonical(name); !ok {
		return nil, fmt.Errorf("unknown timezone %q", name)
	}
	return time.LoadLocation(name)
}
