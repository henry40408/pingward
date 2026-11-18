// Package seed builds deterministic demo data for the README screenshots: SQL
// for a stopped database holding only the POST /setup admin.
//
// The scan runs at boot, so every row must stay inside its budget or the scan
// rewrites the status being shown:
//
//   - up/new is downed once last_ping_at + period + grace <= now;
//   - an in-flight run is downed once last_start_at + max_runtime <= now.
//
// next_due_at is seeded too, since the dashboard reads it for late.
package seed

import "strings"

// AdminUsername is the POST /setup admin; seeded users copy its password hash.
const AdminUsername = "demo"

// AdminPassword is the POST /setup admin's password.
const AdminPassword = "screenshot-demo-password"

const randSeed = 20_260_722

// SQL builds the demo dataset as one SQL transaction, anchored at nowMs.
//
// The order of the steps is part of the output: every step that draws from the
// random stream shifts the numbers the later ones see.
func SQL(nowMs int64) (string, error) {
	now := float64(nowMs)
	rand := &mulberry32{state: randSeed}
	statements := []string{"BEGIN;"}
	statements = append(statements, userStatements(now)...)
	statements = append(statements, projectStatements(now)...)
	channelStmts, err := channelStatements(now)
	if err != nil {
		return "", err
	}
	statements = append(statements, channelStmts...)
	checkStmts, notifications, err := checkStatements(nowMs, rand)
	if err != nil {
		return "", err
	}
	statements = append(statements, checkStmts...)
	for _, n := range notifications {
		statements = append(statements, notificationSQL(n))
	}
	statements = append(statements, auditStatements(now)...)
	statements = append(statements, settingsStatements()...)
	statements = append(statements, "COMMIT;")
	return strings.Join(statements, "\n"), nil
}
