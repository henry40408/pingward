-- Metadata for the session-management UI (/sessions): when a session was
-- created and last seen, and what browser/IP it was seen from. Existing rows
-- get created_at = '' (parsed as None, same as any other unparsable RFC3339
-- text via `parse_ts`) since their real creation time was never recorded.
ALTER TABLE sessions ADD COLUMN created_at TEXT NOT NULL DEFAULT '';
ALTER TABLE sessions ADD COLUMN last_seen_at TEXT;
ALTER TABLE sessions ADD COLUMN user_agent TEXT;
ALTER TABLE sessions ADD COLUMN ip TEXT;
