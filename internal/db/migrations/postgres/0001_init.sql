CREATE TABLE users (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT,
  is_admin BIGINT NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);

CREATE TABLE projects (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  scan_interval_secs BIGINT,
  created_at TEXT NOT NULL
);

CREATE TABLE checks (
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
  created_at TEXT NOT NULL
);
CREATE INDEX idx_checks_status ON checks(status);

CREATE TABLE channels (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  config_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE check_channels (
  check_id BIGINT NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  channel_id BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  PRIMARY KEY (check_id, channel_id)
);

CREATE TABLE pings (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  check_id BIGINT NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('success','fail','start','log','exitcode')),
  exit_code BIGINT,
  body TEXT NOT NULL DEFAULT '',
  source_ip TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX idx_pings_check ON pings(check_id, created_at);

CREATE TABLE notifications (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  check_id BIGINT NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  channel_id BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  event TEXT NOT NULL CHECK (event IN ('down','up')),
  status TEXT NOT NULL CHECK (status IN ('ok','error')),
  error TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at TEXT NOT NULL
);

CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
