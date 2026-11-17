package notify

import (
	"fmt"
	"strings"
	"time"
	"unicode"

	_ "time/tzdata" // zone names must resolve without a system tz database

	"github.com/henry40408/pingward/internal/duration"
	"github.com/henry40408/pingward/internal/models"
)

// ScheduleSummary is the one-line schedule, in the edit forms' duration format.
func ScheduleSummary(check models.Check) string {
	grace := ""
	if check.GraceSecs > 0 {
		grace = fmt.Sprintf(" (grace %s)", duration.Format(check.GraceSecs))
	}
	if check.ScheduleKind == models.ScheduleCron {
		if check.CronExpr == nil {
			return "no cron expression set" + grace
		}
		return fmt.Sprintf("cron \"%s\" %s%s", *check.CronExpr, check.Timezone, grace)
	}
	if check.PeriodSecs == nil {
		return "no period set" + grace
	}
	return fmt.Sprintf("every %s%s", duration.Format(*check.PeriodSecs), grace)
}

// loadZone is UTC when the name is unset or unknown. Names are stored in
// canonical form, so the case-insensitive lookup of some file systems is moot.
func loadZone(name *string) *time.Location {
	if name == nil || *name == "Local" {
		return time.UTC
	}
	zone, err := time.LoadLocation(*name)
	if err != nil {
		return time.UTC
	}
	return zone
}

// fmtAt renders at in the zone, e.g. "2026-07-29 17:03 CST".
func fmtAt(at time.Time, timezone *string) string {
	return at.In(loadZone(timezone)).Format("2006-01-02 15:04 MST")
}

// fmtAtRel is fmtAt plus "(… ago)" when at is in the past.
func fmtAtRel(at, now time.Time, timezone *string) string {
	secs := int64(now.Sub(at) / time.Second)
	if secs > 0 {
		return fmt.Sprintf("%s (%s ago)", fmtAt(at, timezone), duration.Format(secs))
	}
	return fmtAt(at, timezone)
}

// singleLine replaces control characters with spaces: a newline in a header
// or email subject would otherwise corrupt it.
func singleLine(s string) string {
	return strings.Map(func(r rune) rune {
		if unicode.IsControl(r) {
			return ' '
		}
		return r
	}, s)
}

func contextLine(detail EventDetail) (string, bool) {
	switch {
	case detail.ProjectName != nil && detail.Schedule != nil:
		return fmt.Sprintf("Project: %s · %s", *detail.ProjectName, *detail.Schedule), true
	case detail.ProjectName != nil:
		return "Project: " + *detail.ProjectName, true
	case detail.Schedule != nil:
		return "Schedule: " + *detail.Schedule, true
	}
	return "", false
}

// reasonLine: a reminder says "Last ping", not "No ping since", because it has
// no cause and a check downed by a fail ping did ping.
func reasonLine(ev Event) string {
	detail := ev.Detail
	switch ev.Kind {
	case models.EventUp:
		if detail.LastPingAt == nil {
			return "Recovered"
		}
		return "Recovered; previous ping " + fmtAtRel(*detail.LastPingAt, ev.At, detail.Timezone)
	case models.EventReminder:
		if detail.LastPingAt == nil {
			return "Never pinged"
		}
		return "Last ping " + fmtAtRel(*detail.LastPingAt, ev.At, detail.Timezone)
	}
	return downReason(ev)
}

func downReason(ev Event) string {
	detail := ev.Detail
	if cause := detail.Cause; cause != nil {
		switch cause.Kind {
		case CauseOverrun:
			return fmt.Sprintf("Run started %s and exceeded its %s max runtime",
				fmtAtRel(cause.StartedAt, ev.At, detail.Timezone), duration.Format(cause.MaxRuntimeSecs))
		case CauseFailed:
			if cause.ExitCode == nil {
				return "Fail ping received"
			}
			return fmt.Sprintf("Fail ping received (exit code %d)", *cause.ExitCode)
		}
	}
	if detail.LastPingAt == nil {
		return "No ping received yet"
	}
	return "No ping since " + fmtAtRel(*detail.LastPingAt, ev.At, detail.Timezone)
}

// EventText is the message body for every text channel, capped at four lines:
// headline, context, reason, link. Everything else is on the linked page.
func EventText(ev Event) string {
	detail := ev.Detail
	var lines []string
	switch ev.Kind {
	case models.EventTest:
		channel := fmt.Sprintf("Channel \"%s\"", ev.CheckName)
		if detail.ProjectName != nil {
			channel = fmt.Sprintf("Channel \"%s\" · project %s", ev.CheckName, *detail.ProjectName)
		}
		return strings.Join([]string{"\U0001F514 pingward test notification", channel, fmtAt(ev.At, detail.Timezone)}, "\n")
	case models.EventDown:
		lines = append(lines, "\U0001F534 DOWN — "+ev.CheckName)
	case models.EventReminder:
		lines = append(lines, "\U0001F534 STILL DOWN — "+ev.CheckName)
	case models.EventUp:
		lines = append(lines, "\U0001F7E2 UP — "+ev.CheckName)
	}
	if context, ok := contextLine(detail); ok {
		lines = append(lines, context)
	}
	lines = append(lines, reasonLine(ev))
	if detail.URL != nil {
		lines = append(lines, *detail.URL)
	}
	return strings.Join(lines, "\n")
}

// EventTitle is the title for ntfy, Pushover and the email subject.
func EventTitle(ev Event) string {
	subject := singleLine(ev.CheckName)
	if ev.Detail.ProjectName != nil {
		subject = singleLine(*ev.Detail.ProjectName) + "/" + subject
	}
	switch ev.Kind {
	case models.EventTest:
		return fmt.Sprintf("pingward: test notification for \"%s\"", subject)
	case models.EventUp:
		return fmt.Sprintf("pingward: %s is UP", subject)
	case models.EventReminder:
		return fmt.Sprintf("pingward: %s is STILL DOWN", subject)
	}
	return fmt.Sprintf("pingward: %s is DOWN", subject)
}
