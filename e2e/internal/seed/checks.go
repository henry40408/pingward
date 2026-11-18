package seed

import (
	"fmt"
	"time"
)

// checkPlan is where a check's last run, next deadline and in-flight run fall,
// in epoch ms.
type checkPlan struct {
	finishAt float64
	nextDue  float64
	// period is nil for cron checks.
	period *int64
	// runningStart is a start newer than the last finish, kept inside
	// maxRuntime so the scan does not down it. Nil when nothing is running.
	runningStart *float64
}

// lastFinishOffset is the seconds before now of the last finished run, scaled
// by cadence.
func lastFinishOffset(c check, rand *mulberry32) float64 {
	cadence, grace := float64(c.cadence), float64(c.grace)
	switch c.state {
	case stateDown:
		return cadence*2.0 + grace + 900.0
	case stateLate:
		// Inside grace: the scan leaves it, the dashboard says late.
		return cadence + grace*0.5
	case stateRunning:
		return cadence * 0.95
	case statePaused:
		return cadence * 0.4
	default:
		return cadence * (0.15 + float64(0.35*rand.next()))
	}
}

func planCheck(c check, nowMs int64, rand *mulberry32) (checkPlan, error) {
	now := float64(nowMs)
	zone, err := time.LoadLocation(c.timezone)
	if err != nil {
		return checkPlan{}, fmt.Errorf("unknown timezone `%s`", c.timezone)
	}
	plan := checkPlan{}
	if c.cron != "" {
		return planCronCheck(c, zone, nowMs)
	}
	plan.finishAt = now - lastFinishOffset(c, rand)*1000.0
	plan.nextDue = plan.finishAt + float64(c.cadence+c.grace)*1000.0
	plan.period = &c.cadence
	if c.state == stateRunning {
		start := now - 6.0*60.0*1000.0
		plan.runningStart = &start
	}
	return plan, nil
}

// planCronCheck anchors on a real fire.
func planCronCheck(c check, zone *time.Location, nowMs int64) (checkPlan, error) {
	last, err := lastFireAtOrBefore(c.cron, zone, nowMs)
	if err != nil {
		return checkPlan{}, err
	}
	next, err := nextFireAfter(c.cron, zone, last)
	if err != nil {
		return checkPlan{}, err
	}
	plan := checkPlan{finishAt: float64(last), nextDue: float64(next + c.grace*1000)}
	if c.state == stateRunning {
		start := float64(nowMs) - 6.0*60.0*1000.0
		plan.runningStart = &start
	}
	return plan, nil
}

// checkStatements are the checks rows, their ping history and channel
// bindings, plus the notifications to insert once every check exists.
func checkStatements(nowMs int64, rand *mulberry32) ([]string, []notification, error) {
	now := float64(nowMs)
	var statements []string
	var notifications []notification
	for _, c := range checks {
		plan, err := planCheck(c, nowMs, rand)
		if err != nil {
			return nil, nil, err
		}
		insert, err := checkInsertSQL(c, plan, now, rand)
		if err != nil {
			return nil, nil, err
		}
		statements = append(statements, insert)
		check := checkSQL(c.name)
		for _, key := range c.channels {
			channel, err := channelSQL(key)
			if err != nil {
				return nil, nil, err
			}
			statements = append(statements,
				fmt.Sprintf("INSERT INTO check_channels (check_id, channel_id) VALUES (%s, %s);", check, channel))
		}
		statements = append(statements, historyStatements(c, plan, check, rand)...)
		extra, err := checkNotifications(c, check, now)
		if err != nil {
			return nil, nil, err
		}
		notifications = append(notifications, extra...)
	}
	return statements, notifications, nil
}

// uuidFor is deterministic, since the ping URL is in the check-page screenshot.
func uuidFor(rand *mulberry32) string {
	const digits = "0123456789abcdef"
	hex := func(count int) string {
		out := make([]byte, count)
		for i := range out {
			out[i] = digits[rand.index(len(digits))]
		}
		return string(out)
	}
	// Arguments evaluate left to right, as the format string reads.
	a, b, c, d, e := hex(8), hex(4), hex(3), hex(3), hex(12)
	return fmt.Sprintf("%s-%s-4%s-a%s-%s", a, b, c, d, e)
}

func checkInsertSQL(c check, plan checkPlan, now float64, rand *mulberry32) (string, error) {
	status := "up"
	switch c.state {
	case stateDown:
		status = "down"
	case statePaused:
		status = "paused"
	}
	lastStart := plan.finishAt - 60.0*1000.0
	if plan.runningStart != nil {
		lastStart = *plan.runningStart
	}
	project, err := projectSQL(c.project)
	if err != nil {
		return "", err
	}
	scheduleKind := "period"
	if c.cron != "" {
		scheduleKind = "cron"
	}
	lastAlert := "NULL"
	if c.state == stateDown {
		lastAlert = quote(iso(now - 10.0*60.0*1000.0))
	}
	var maxRuntime *int64
	if c.maxRuntime != 0 {
		maxRuntime = &c.maxRuntime
	}
	return fmt.Sprintf("INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, "+
		"period_secs, grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at, "+
		"next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)\n"+
		"  VALUES (%s, %s, %s, %s, %s, %s, %d, %s, %s, %s, %s, %s, %s, %s, %s, 0, %s);",
		project, quote(c.name), quote(c.description), quote(uuidFor(rand)), quote(scheduleKind),
		nullableNumber(plan.period), c.grace, nullable(c.cron), quote(c.timezone), quote(status),
		quote(iso(plan.finishAt)), quote(iso(lastStart)), quote(iso(plan.nextDue)),
		nullableNumber(maxRuntime), lastAlert, quote(iso(now-45.0*day*1000.0))), nil
}

// historyStatements are runs start/finish pairs ending at the plan's last
// finish, plus the in-flight start of a running check.
func historyStatements(c check, plan checkPlan, check string, rand *mulberry32) []string {
	var statements []string
	for index := runs - 1; index >= 0; index-- {
		jitter := float64((rand.next() - 0.5) * float64(c.cadence) * 0.06)
		end := plan.finishAt - float64(int64(index)*c.cadence)*1000.0 + float64(jitter*1000.0)
		// Every ninth run is slow (amber bar, if maxRuntime is set); two runs of
		// the down check fail with captured output.
		slow := index%9 == 4
		failed := c.state == stateDown && (index == 0 || index == 12)
		duration := rand.between(c.runtimeLow, c.runtimeHigh)
		if c.maxRuntime != 0 && slow {
			duration = float64(c.maxRuntime) * (0.82 + float64(0.1*rand.next()))
		}
		start := end - float64(duration*1000.0)
		ip := sourceIPs[rand.index(len(sourceIPs))]
		finish := pingRow{check: check, kind: "success", atMs: end, ip: ip}
		if failed {
			finish.kind, finish.body = "fail", failBodies[index%len(failBodies)]
		}
		statements = append(statements,
			pingSQL(pingRow{check: check, kind: "start", atMs: start, ip: ip}),
			pingSQL(finish))
	}
	if plan.runningStart != nil {
		statements = append(statements,
			pingSQL(pingRow{check: check, kind: "start", atMs: *plan.runningStart, ip: sourceIPs[0]}))
	}
	return statements
}

// checkNotifications are the down check's alert chain plus one recovered
// incident on the hourly sync, for a mixed log.
func checkNotifications(c check, check string, now float64) ([]notification, error) {
	var notifications []notification
	add := func(channelKey, event, status, errText string, ago float64) error {
		channel, err := channelSQL(channelKey)
		if err != nil {
			return err
		}
		notifications = append(notifications, notification{
			check: check, channel: channel, event: event, status: status, err: errText,
			at: now - ago*1000.0,
		})
		return nil
	}
	if c.state == stateDown {
		chain := []struct {
			event, channel, status, errText string
			ago                             float64
		}{
			{"down", "ops-slack", "ok", "", 62.0 * 60.0},
			{"down", "oncall-pushover", "ok", "", 62.0 * 60.0},
			{"down", "alerts-email", "error", "smtp: connection refused", 62.0 * 60.0},
			{"reminder", "ops-slack", "ok", "", 32.0 * 60.0},
			{"reminder", "oncall-pushover", "ok", "", 32.0 * 60.0},
		}
		for _, link := range chain {
			if err := add(link.channel, link.event, link.status, link.errText, link.ago); err != nil {
				return nil, err
			}
		}
	}
	if c.key == "s3-sync" {
		for _, incident := range []struct {
			event string
			hours float64
		}{{"down", 26.0}, {"up", 25.4}} {
			if err := add("ops-slack", incident.event, "ok", "", incident.hours*hour); err != nil {
				return nil, err
			}
		}
	}
	return notifications, nil
}
