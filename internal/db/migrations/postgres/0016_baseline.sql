-- Schema version 16 as a whole. It replaces the sixteen steps that releases up to
-- 0.13 applied one by one, and creates only what is missing, so running it on a
-- database that already is at version 16 changes nothing.

CREATE TABLE IF NOT EXISTS users (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT,
  is_admin BIGINT NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  disabled BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS projects (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  scan_interval_secs BIGINT,
  created_at TEXT NOT NULL,
  nag_interval_secs BIGINT,
  description TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS checks (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  ping_uuid TEXT NOT NULL UNIQUE,
  schedule_kind TEXT NOT NULL CHECK (schedule_kind IN ('period','cron')),
  period_secs BIGINT,
  grace_secs BIGINT NOT NULL DEFAULT 300,
  cron_expr TEXT,
  timezone TEXT NOT NULL DEFAULT 'UTC',
  status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','up','down','paused')),
  last_ping_at TEXT,
  last_start_at TEXT,
  next_due_at TEXT,
  scan_interval_secs BIGINT,
  created_at TEXT NOT NULL,
  max_runtime_secs BIGINT,
  nag_interval_secs BIGINT,
  last_alert_at TEXT,
  acknowledged BIGINT NOT NULL DEFAULT 0,
  description TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_checks_status ON checks(status);

CREATE TABLE IF NOT EXISTS channels (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,                     -- 'webhook'|'telegram'|'slack'|'ntfy'
  name TEXT NOT NULL,
  config_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_channels_project ON channels(project_id);

CREATE TABLE IF NOT EXISTS check_channels (
  check_id BIGINT NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  channel_id BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  PRIMARY KEY (check_id, channel_id)
);
CREATE INDEX IF NOT EXISTS idx_check_channels_channel ON check_channels(channel_id);

CREATE TABLE IF NOT EXISTS pings (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  check_id BIGINT NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('success','fail','start','log','exitcode')),
  exit_code BIGINT,
  body TEXT NOT NULL DEFAULT '',
  source_ip TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pings_check ON pings(check_id, created_at);
CREATE INDEX IF NOT EXISTS idx_pings_check_id ON pings(check_id, id);

-- Session cookies are `<id>.<hmac>` and the CSRF token is derived from the id.
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT '',
  last_seen_at TEXT,
  user_agent TEXT,
  ip TEXT,
  sso BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
-- Prune scans sessions by expires_at (idle expiry) and created_at (absolute cap).
CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at);
CREATE INDEX IF NOT EXISTS idx_sessions_created ON sessions(created_at);

CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS notifications (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  check_id BIGINT NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  channel_id BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  event TEXT NOT NULL CHECK (event IN ('down','up','reminder')),
  status TEXT NOT NULL CHECK (status IN ('ok','error')),
  error TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_notifications_check ON notifications(check_id, created_at);
CREATE INDEX IF NOT EXISTS idx_notifications_check_id ON notifications(check_id, id);

CREATE TABLE IF NOT EXISTS audit_log (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  actor_user_id BIGINT,
  actor_username TEXT NOT NULL,
  action TEXT NOT NULL,
  target_type TEXT,
  target_id BIGINT,
  target_owner_id BIGINT,
  method TEXT,
  path TEXT,
  detail TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_created ON audit_log(created_at);
-- The /admin audit table filters by actor and action and lists their distinct
-- values; nothing prunes audit_log, so both would degrade into full scans.
CREATE INDEX IF NOT EXISTS idx_audit_actor ON audit_log(actor_username);
CREATE INDEX IF NOT EXISTS idx_audit_action ON audit_log(action);

-- Account-bound API keys. Only the SHA-256 hash of a key is stored; the
-- plaintext token is shown once at creation. `prefix` is a non-secret display
-- fragment (e.g. `pw_1a2b3c4d`).
CREATE TABLE IF NOT EXISTS api_keys (
  id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id      BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name         TEXT   NOT NULL,
  token_hash   TEXT   NOT NULL UNIQUE,
  prefix       TEXT   NOT NULL,
  created_at   TEXT   NOT NULL,
  last_used_at TEXT,
  expires_at   TEXT
);
CREATE INDEX IF NOT EXISTS idx_api_keys_user ON api_keys(user_id);

-- Releases up to 0.13 recorded their steps here. This row is a version they do
-- not know, so they refuse to start instead of running against a newer schema.
CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
    success BOOLEAN NOT NULL,
    checksum BYTEA NOT NULL,
    execution_time BIGINT NOT NULL
);
INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
VALUES (99999999999999, 'pingward no longer records migrations here', TRUE, '', 0)
ON CONFLICT (version) DO NOTHING;
