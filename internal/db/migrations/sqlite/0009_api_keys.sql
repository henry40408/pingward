-- Account-bound API keys for the programmatic REST API. Only the SHA-256 hash
-- of a key is stored; the plaintext token is shown once at creation. `prefix`
-- is a non-secret display fragment (e.g. `pw_1a2b3c4d`). Timestamps are RFC3339
-- text, matching every other table.
CREATE TABLE api_keys (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name         TEXT    NOT NULL,
  token_hash   TEXT    NOT NULL UNIQUE,
  prefix       TEXT    NOT NULL,
  created_at   TEXT    NOT NULL,
  last_used_at TEXT,
  expires_at   TEXT
);
CREATE INDEX idx_api_keys_user ON api_keys(user_id);
