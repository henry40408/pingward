package seed

import (
	"fmt"
	"math"
	"strconv"
	"strings"
	"time"
)

// quote SQL-quotes a value.
func quote(value string) string {
	return "'" + strings.ReplaceAll(value, "'", "''") + "'"
}

// nullable SQL-quotes a value, or NULL when it is empty.
func nullable(value string) string {
	if value == "" {
		return "NULL"
	}
	return quote(value)
}

func nullableNumber(value *int64) string {
	if value == nil {
		return "NULL"
	}
	return strconv.FormatInt(*value, 10)
}

// iso formats epoch ms as RFC3339 with millisecond precision, like
// JavaScript's toISOString.
func iso(ms float64) string {
	return time.UnixMilli(int64(math.Round(ms))).UTC().Format("2006-01-02T15:04:05.000Z")
}

func ownerSQL(index int) string {
	username := AdminUsername
	if index > 0 {
		username = extraUsers[index-1].username
	}
	return "(SELECT id FROM users WHERE username = " + quote(username) + ")"
}

func projectSQL(key string) (string, error) {
	for _, candidate := range projects {
		if candidate.key == key {
			return "(SELECT id FROM projects WHERE name = " + quote(candidate.name) + ")", nil
		}
	}
	return "", fmt.Errorf("no seeded project keyed `%s`", key)
}

func channelSQL(key string) (string, error) {
	for _, candidate := range channels {
		if candidate.key == key {
			return "(SELECT id FROM channels WHERE name = " + quote(candidate.name) + ")", nil
		}
	}
	return "", fmt.Errorf("no seeded channel keyed `%s`", key)
}

func checkSQL(name string) string {
	return "(SELECT id FROM checks WHERE name = " + quote(name) + ")"
}

type pingRow struct {
	// check is a SELECT sub-query naming the check: ids are DB-assigned.
	check string
	kind  string
	atMs  float64
	body  string
	ip    string
}

func pingSQL(ping pingRow) string {
	return fmt.Sprintf("INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)\n"+
		"  VALUES (%s, %s, NULL, %s, %s, %s);",
		ping.check, quote(ping.kind), quote(ping.body), quote(ping.ip), quote(iso(ping.atMs)))
}

// notification holds SELECT sub-queries by name: ids are DB-assigned.
type notification struct {
	check, channel string
	event, status  string
	err            string // empty means NULL
	at             float64
}

func notificationSQL(n notification) string {
	return fmt.Sprintf("INSERT INTO notifications (check_id, channel_id, event, status, error, created_at)\n"+
		"  VALUES (%s, %s, %s, %s, %s, %s);",
		n.check, n.channel, quote(n.event), quote(n.status), nullable(n.err), quote(iso(n.at)))
}

func userStatements(now float64) []string {
	statements := make([]string, 0, len(extraUsers))
	for index, user := range extraUsers {
		created := now - float64((40.0-float64(index)*6.0)*day)*1000.0
		isAdmin := 0
		if user.isAdmin {
			isAdmin = 1
		}
		statements = append(statements, fmt.Sprintf("INSERT INTO users (username, password_hash, is_admin, created_at)\n"+
			"  SELECT %s, password_hash, %d, %s\n  FROM users WHERE username = %s;",
			quote(user.username), isAdmin, quote(iso(created)), quote(AdminUsername)))
	}
	return statements
}

func projectStatements(now float64) []string {
	statements := make([]string, 0, len(projects))
	for index, project := range projects {
		created := now - float64((60.0-float64(index)*5.0)*day)*1000.0
		statements = append(statements, fmt.Sprintf("INSERT INTO projects (user_id, name, description, scan_interval_secs, "+
			"nag_interval_secs, created_at)\n  VALUES (%s, %s, %s, %s, %s, %s);",
			ownerSQL(project.owner), quote(project.name), quote(project.description),
			nullableNumber(project.scanIntervalSecs), nullableNumber(project.nagIntervalSecs), quote(iso(created))))
	}
	return statements
}

func channelStatements(now float64) ([]string, error) {
	statements := make([]string, 0, len(channels))
	for _, channel := range channels {
		project, err := projectSQL(channel.project)
		if err != nil {
			return nil, err
		}
		statements = append(statements, fmt.Sprintf("INSERT INTO channels (project_id, kind, name, config_json, created_at)\n"+
			"  VALUES (%s, %s, %s, %s, %s);",
			project, quote(channel.kind), quote(channel.name), quote(channel.config),
			quote(iso(now-50.0*day*1000.0))))
	}
	return statements, nil
}

func auditStatements(now float64) []string {
	statements := make([]string, 0, len(audits))
	for _, entry := range audits {
		statements = append(statements, fmt.Sprintf("INSERT INTO audit_log (actor_user_id, actor_username, action, target_type, "+
			"target_id, target_owner_id, method, path, detail, created_at)\n"+
			"  VALUES ((SELECT id FROM users WHERE username = %[1]s), %[1]s, %[2]s, %[3]s, %[4]d, %[4]d, "+
			"%[5]s, %[6]s, %[7]s, %[8]s);",
			quote(entry.actor), quote(entry.action), quote(entry.targetType), entry.targetID,
			quote(entry.method), quote(entry.path), nullable(entry.detail), quote(iso(now-entry.ago*1000.0))))
	}
	return statements
}

func settingsStatements() []string {
	statements := make([]string, 0, len(settings))
	for _, setting := range settings {
		statements = append(statements, fmt.Sprintf("INSERT INTO settings (key, value) VALUES (%s, %s)\n"+
			"  ON CONFLICT(key) DO UPDATE SET value = excluded.value;",
			quote(setting[0]), quote(setting[1])))
	}
	return statements
}
