ALTER TABLE checks ADD COLUMN nag_interval_secs BIGINT;
ALTER TABLE checks ADD COLUMN last_alert_at TEXT;
ALTER TABLE checks ADD COLUMN acknowledged BIGINT NOT NULL DEFAULT 0;
ALTER TABLE projects ADD COLUMN nag_interval_secs BIGINT;

ALTER TABLE notifications DROP CONSTRAINT notifications_event_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_event_check
  CHECK (event IN ('down','up','reminder'));
