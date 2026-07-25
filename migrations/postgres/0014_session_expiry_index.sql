-- prune scans sessions by expires_at (idle expiry) and by created_at
-- (absolute cap); the only existing index is idx_sessions_user.
CREATE INDEX idx_sessions_expires ON sessions(expires_at);
CREATE INDEX idx_sessions_created ON sessions(created_at);
