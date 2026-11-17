ALTER TABLE checks ADD COLUMN nag_interval_secs INTEGER;
ALTER TABLE checks ADD COLUMN last_alert_at TEXT;
ALTER TABLE checks ADD COLUMN acknowledged INTEGER NOT NULL DEFAULT 0;
ALTER TABLE projects ADD COLUMN nag_interval_secs INTEGER;

-- Widen notifications.event CHECK to include 'reminder'. SQLite cannot ALTER a
-- CHECK constraint, so rebuild the table. Nothing references notifications, so
-- the drop is safe.
CREATE TABLE notifications_new (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  check_id INTEGER NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  event TEXT NOT NULL CHECK (event IN ('down','up','reminder')),
  status TEXT NOT NULL CHECK (status IN ('ok','error')),
  error TEXT,
  created_at TEXT NOT NULL
);
INSERT INTO notifications_new (id, check_id, channel_id, event, status, error, created_at)
  SELECT id, check_id, channel_id, event, status, error, created_at FROM notifications;
DROP TABLE notifications;
ALTER TABLE notifications_new RENAME TO notifications;
-- The DROP above also dropped the index created in 0002_indexes.sql; recreate it
-- so the rebuilt table keeps parity with Postgres (which only alters the CHECK).
CREATE INDEX idx_notifications_check ON notifications(check_id, created_at);
