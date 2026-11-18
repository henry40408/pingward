package seed

import (
	"fmt"
	"strconv"
	"strings"
	"time"
	_ "time/tzdata" // the seed must not depend on the host's zoneinfo
)

// A minimal cron evaluator. It anchors a cron check's last ping exactly on its
// last fire; anchored off a fire, the boot scan sees an unserved fire and
// downs the check. It supports only `*`, `*/n`, plain numbers and `,` lists.

const minuteMs = 60_000

// scanLimitMinutes is over a year of minutes, so a yearly cron still resolves.
const scanLimitMinutes = 400 * 24 * 60

type cronFields struct {
	sec, min, hour, dayOfMonth, month, dayOfWeek int
}

// localFields splits an instant into cron fields in the zone. The weekday
// counts from Sunday = 1, as the server's cron evaluator numbers it.
func localFields(ms int64, zone *time.Location) cronFields {
	local := time.UnixMilli(ms).In(zone)
	return cronFields{
		sec:        local.Second(),
		min:        local.Minute(),
		hour:       local.Hour(),
		dayOfMonth: local.Day(),
		month:      int(local.Month()),
		dayOfWeek:  int(local.Weekday()) + 1,
	}
}

func fieldMatches(spec string, value int) bool {
	for _, part := range strings.Split(spec, ",") {
		if part == "*" {
			return true
		}
		if step, ok := strings.CutPrefix(part, "*/"); ok {
			divisor, err := strconv.ParseUint(step, 10, 32)
			if err == nil && divisor != 0 && value%int(divisor) == 0 {
				return true
			}
			continue
		}
		if number, err := strconv.ParseUint(part, 10, 32); err == nil && int(number) == value {
			return true
		}
	}
	return false
}

func cronMatches(expression string, zone *time.Location, ms int64) (bool, error) {
	parts := strings.Fields(expression)
	if len(parts) != 6 {
		return false, fmt.Errorf("`%s` is not a 6-field cron expression", expression)
	}
	fields := localFields(ms, zone)
	return fieldMatches(parts[0], fields.sec) &&
		fieldMatches(parts[1], fields.min) &&
		fieldMatches(parts[2], fields.hour) &&
		fieldMatches(parts[3], fields.dayOfMonth) &&
		fieldMatches(parts[4], fields.month) &&
		fieldMatches(parts[5], fields.dayOfWeek), nil
}

func floorToMinute(ms int64) int64 {
	return ms - ((ms%minuteMs)+minuteMs)%minuteMs
}

// lastFireAtOrBefore is the last fire at or before fromMs. Minute steps are
// exact because every expression here has a zero seconds field.
func lastFireAtOrBefore(expression string, zone *time.Location, fromMs int64) (int64, error) {
	return scanMinutes(expression, zone, floorToMinute(fromMs), -minuteMs, "past")
}

// nextFireAfter is the first fire strictly after fromMs.
func nextFireAfter(expression string, zone *time.Location, fromMs int64) (int64, error) {
	return scanMinutes(expression, zone, floorToMinute(fromMs)+minuteMs, minuteMs, "next")
}

func scanMinutes(expression string, zone *time.Location, start, step int64, direction string) (int64, error) {
	at := start
	for range scanLimitMinutes {
		matches, err := cronMatches(expression, zone, at)
		if err != nil {
			return 0, err
		}
		if matches {
			return at, nil
		}
		at += step
	}
	return 0, fmt.Errorf("cron `%s` (%s) has no fire in the %s year", expression, zone, direction)
}
