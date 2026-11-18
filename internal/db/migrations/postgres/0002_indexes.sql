CREATE INDEX idx_channels_project ON channels(project_id);
CREATE INDEX idx_check_channels_channel ON check_channels(channel_id);
CREATE INDEX idx_notifications_check ON notifications(check_id, created_at);
CREATE INDEX idx_sessions_user ON sessions(user_id);
