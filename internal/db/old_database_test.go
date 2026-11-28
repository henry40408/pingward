package db

import (
	"context"
	"testing"
)

// oldSchema is what sqlite_master holds in a database created by pingward
// v0.12.0 (ghcr.io/henry40408/pingward): the shape sixteen separate steps leave
// behind, with the columns added later appended by ALTER TABLE. Releases from
// 0.3.0 to 0.13 produce the same schema.
var oldSchema = []string{
	`CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
)`,
	`CREATE TABLE users (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT,
  is_admin INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
, disabled INTEGER NOT NULL DEFAULT 0)`,
	`CREATE TABLE projects (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  scan_interval_secs INTEGER,
  created_at TEXT NOT NULL
, nag_interval_secs INTEGER, description TEXT NOT NULL DEFAULT '')`,
	`CREATE TABLE checks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  ping_uuid TEXT NOT NULL UNIQUE,
  schedule_kind TEXT NOT NULL CHECK (schedule_kind IN ('period','cron')),
  period_secs INTEGER,
  grace_secs INTEGER NOT NULL DEFAULT 300,
  cron_expr TEXT,
  timezone TEXT NOT NULL DEFAULT 'UTC',
  status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','up','down','paused')),
  last_ping_at TEXT,
  last_start_at TEXT,
  next_due_at TEXT,
  scan_interval_secs INTEGER,
  created_at TEXT NOT NULL
, max_runtime_secs INTEGER, nag_interval_secs INTEGER, last_alert_at TEXT, acknowledged INTEGER NOT NULL DEFAULT 0, description TEXT NOT NULL DEFAULT '')`,
	`CREATE INDEX idx_checks_status ON checks(status)`,
	`CREATE TABLE channels (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,                     -- 'webhook'|'telegram'|'slack'|'ntfy'
  name TEXT NOT NULL,
  config_json TEXT NOT NULL,
  created_at TEXT NOT NULL
)`,
	`CREATE TABLE check_channels (
  check_id INTEGER NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  PRIMARY KEY (check_id, channel_id)
)`,
	`CREATE TABLE pings (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  check_id INTEGER NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('success','fail','start','log','exitcode')),
  exit_code INTEGER,
  body TEXT NOT NULL DEFAULT '',
  source_ip TEXT,
  created_at TEXT NOT NULL
)`,
	`CREATE INDEX idx_pings_check ON pings(check_id, created_at)`,
	`CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at TEXT NOT NULL
, created_at TEXT NOT NULL DEFAULT '', last_seen_at TEXT, user_agent TEXT, ip TEXT, sso INTEGER NOT NULL DEFAULT 0)`,
	`CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
)`,
	`CREATE INDEX idx_channels_project ON channels(project_id)`,
	`CREATE INDEX idx_check_channels_channel ON check_channels(channel_id)`,
	`CREATE INDEX idx_sessions_user ON sessions(user_id)`,
	`CREATE TABLE "notifications" (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  check_id INTEGER NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  event TEXT NOT NULL CHECK (event IN ('down','up','reminder')),
  status TEXT NOT NULL CHECK (status IN ('ok','error')),
  error TEXT,
  created_at TEXT NOT NULL
)`,
	`CREATE INDEX idx_notifications_check ON notifications(check_id, created_at)`,
	`CREATE TABLE audit_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  actor_user_id INTEGER,
  actor_username TEXT NOT NULL,
  action TEXT NOT NULL,
  target_type TEXT,
  target_id INTEGER,
  target_owner_id INTEGER,
  method TEXT,
  path TEXT,
  detail TEXT,
  created_at TEXT NOT NULL
)`,
	`CREATE INDEX idx_audit_created ON audit_log(created_at)`,
	`CREATE INDEX idx_pings_check_id ON pings(check_id, id)`,
	`CREATE INDEX idx_notifications_check_id ON notifications(check_id, id)`,
	`CREATE TABLE api_keys (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name         TEXT    NOT NULL,
  token_hash   TEXT    NOT NULL UNIQUE,
  prefix       TEXT    NOT NULL,
  created_at   TEXT    NOT NULL,
  last_used_at TEXT,
  expires_at   TEXT
)`,
	`CREATE INDEX idx_api_keys_user ON api_keys(user_id)`,
	`CREATE INDEX idx_sessions_expires ON sessions(expires_at)`,
	`CREATE INDEX idx_sessions_created ON sessions(created_at)`,
	`CREATE INDEX idx_audit_actor ON audit_log(actor_username)`,
	`CREATE INDEX idx_audit_action ON audit_log(action)`,
}

// oldDescriptions are the records of versions 1 to 16 in the same database.
var oldDescriptions = []string{
	"init", "indexes", "max runtime", "nag", "user disabled", "audit log", "session csrf",
	"ping history index", "api keys", "session metadata", "descriptions", "session secret",
	"session sso", "session expiry index", "invalidate legacy sessions", "audit filter indexes",
}

// insertOldRecords writes the rows a release up to 0.13 kept. Their checksums
// and timings are left out: nothing reads them any more.
func insertOldRecords(t *testing.T, database *DB) {
	t.Helper()
	for i, description := range oldDescriptions {
		mustExec(t, database, `INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
			VALUES ($1, $2, TRUE, '', 0)`, i+1, description)
	}
}

// openOld returns a database as a release up to 0.13 left it, with two users.
func openOld(t *testing.T) *DB {
	t.Helper()
	database := openMemory(t)
	for _, statement := range oldSchema {
		mustExec(t, database, statement)
	}
	insertOldRecords(t, database)
	mustExec(t, database, `INSERT INTO users (username, is_admin, created_at) VALUES
		('admin', 1, '2026-11-17T15:24:56.085137657+00:00'),
		('carol', 0, '2026-11-17T15:30:10.190704932+00:00')`)
	return database
}

func mustExec(t *testing.T, database *DB, query string, args ...any) {
	t.Helper()
	if _, err := database.ExecContext(context.Background(), query, args...); err != nil {
		t.Fatal(err)
	}
}
