BEGIN;
INSERT INTO users (username, password_hash, is_admin, created_at)
  SELECT 'maya', password_hash, 1, '2026-12-06T08:00:00.000Z'
  FROM users WHERE username = 'demo';
INSERT INTO users (username, password_hash, is_admin, created_at)
  SELECT 'sam', password_hash, 0, '2026-12-12T08:00:00.000Z'
  FROM users WHERE username = 'demo';
INSERT INTO projects (user_id, name, description, scan_interval_secs, nag_interval_secs, created_at)
  VALUES ((SELECT id FROM users WHERE username = 'demo'), 'Backups', 'Nightly database dumps and offsite sync. **Paging** goes to the on-call rotation.', NULL, 1800, '2026-11-16T08:00:00.000Z');
INSERT INTO projects (user_id, name, description, scan_interval_secs, nag_interval_secs, created_at)
  VALUES ((SELECT id FROM users WHERE username = 'demo'), 'Data pipeline', 'Hourly ETL plus the downstream dbt models.', 60, NULL, '2026-11-21T08:00:00.000Z');
INSERT INTO projects (user_id, name, description, scan_interval_secs, nag_interval_secs, created_at)
  VALUES ((SELECT id FROM users WHERE username = 'demo'), 'Website', 'Certificate renewal and the housekeeping crons for the public site.', NULL, NULL, '2026-11-26T08:00:00.000Z');
INSERT INTO projects (user_id, name, description, scan_interval_secs, nag_interval_secs, created_at)
  VALUES ((SELECT id FROM users WHERE username = 'sam'), 'Staging (sam)', 'Another user''s project — only reachable from /admin.', NULL, NULL, '2026-12-01T08:00:00.000Z');
INSERT INTO channels (project_id, kind, name, config_json, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Backups'), 'slack', '#ops-alerts', '{"url":"https://hooks.slack.com/services/T000/B000/xxxxxxxxxxxx"}', '2026-11-26T08:00:00.000Z');
INSERT INTO channels (project_id, kind, name, config_json, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Backups'), 'pushover', 'on-call phone', '{"token":"axxxxxxxxxxxxxxxxxxxxxxxxxxxxx","user":"uxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"}', '2026-11-26T08:00:00.000Z');
INSERT INTO channels (project_id, kind, name, config_json, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Backups'), 'email', 'alerts@example.com', '{"to":"alerts@example.com"}', '2026-11-26T08:00:00.000Z');
INSERT INTO channels (project_id, kind, name, config_json, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Data pipeline'), 'ntfy', 'ntfy · data-pipeline', '{"base_url":"https://ntfy.sh","topic":"data-pipeline"}', '2026-11-26T08:00:00.000Z');
INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs, grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at, next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Backups'), 'postgres-nightly-dump', '`pg_dump` of the primary, streamed straight to object storage.', '354a690e-c285-4420-a9d5-545b75cbe518', 'cron', NULL, 900, '0 30 2 * * *', 'Europe/Berlin', 'up', '2027-01-15T01:30:00.000Z', '2027-01-15T01:29:00.000Z', '2027-01-16T01:45:00.000Z', 2700, NULL, 0, '2026-12-01T08:00:00.000Z');
INSERT INTO check_channels (check_id, channel_id) VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), (SELECT id FROM channels WHERE name = '#ops-alerts'));
INSERT INTO check_channels (check_id, channel_id) VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), (SELECT id FROM channels WHERE name = 'on-call phone'));
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.31', '2026-12-13T00:45:49.492Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.31', '2026-12-13T01:09:59.475Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '192.168.20.8', '2026-12-14T01:16:30.960Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '192.168.20.8', '2026-12-14T01:39:36.552Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.31', '2026-12-15T00:35:56.161Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.31', '2026-12-15T01:16:47.876Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.31', '2026-12-16T00:30:32.076Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.31', '2026-12-16T00:52:14.325Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '192.168.20.8', '2026-12-17T01:35:59.940Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '192.168.20.8', '2026-12-17T01:54:24.311Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.15', '2026-12-18T01:12:24.491Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.15', '2026-12-18T01:35:18.754Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.15', '2026-12-19T01:43:06.494Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.15', '2026-12-19T02:05:14.433Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.31', '2026-12-20T01:52:56.652Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.31', '2026-12-20T02:12:43.224Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '192.168.20.8', '2026-12-21T00:49:30.581Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '192.168.20.8', '2026-12-21T01:06:55.984Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.31', '2026-12-22T01:06:14.266Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.31', '2026-12-22T01:29:03.679Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.15', '2026-12-23T00:35:33.965Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.15', '2026-12-23T00:56:21.446Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.15', '2026-12-24T01:24:02.600Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.15', '2026-12-24T02:04:57.665Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '192.168.20.8', '2026-12-25T00:51:08.208Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '192.168.20.8', '2026-12-25T01:06:18.029Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.31', '2026-12-26T00:29:14.568Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.31', '2026-12-26T00:51:15.261Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.31', '2026-12-27T00:36:44.727Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.31', '2026-12-27T00:55:24.361Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '192.168.20.8', '2026-12-28T00:39:05.200Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '192.168.20.8', '2026-12-28T00:59:25.583Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.15', '2026-12-29T01:23:29.652Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.15', '2026-12-29T01:40:24.467Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.31', '2026-12-30T00:42:04.904Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.31', '2026-12-30T01:05:04.840Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '192.168.20.8', '2026-12-31T01:06:14.067Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '192.168.20.8', '2026-12-31T01:26:51.630Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.15', '2027-01-01T00:45:58.427Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.15', '2027-01-01T01:08:24.949Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.15', '2027-01-02T00:10:02.873Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.15', '2027-01-02T00:49:13.592Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.15', '2027-01-03T01:20:05.903Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.15', '2027-01-03T01:37:53.614Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.31', '2027-01-04T00:44:09.213Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.31', '2027-01-04T01:04:50.569Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.31', '2027-01-05T01:06:45.828Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.31', '2027-01-05T01:26:18.812Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.15', '2027-01-06T01:35:48.422Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.15', '2027-01-06T01:53:53.559Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '192.168.20.8', '2027-01-07T01:00:24.683Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '192.168.20.8', '2027-01-07T01:20:33.098Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '192.168.20.8', '2027-01-08T01:39:56.373Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '192.168.20.8', '2027-01-08T01:57:45.913Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.31', '2027-01-09T01:38:30.415Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.31', '2027-01-09T01:53:37.369Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.31', '2027-01-10T00:27:54.503Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.31', '2027-01-10T00:49:52.410Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '192.168.20.8', '2027-01-11T00:37:13.279Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '192.168.20.8', '2027-01-11T01:14:36.418Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '192.168.20.8', '2027-01-12T01:56:10.156Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '192.168.20.8', '2027-01-12T02:11:27.910Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '192.168.20.8', '2027-01-13T00:36:28.018Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '192.168.20.8', '2027-01-13T00:53:23.760Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '10.4.2.31', '2027-01-14T00:49:49.979Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '10.4.2.31', '2027-01-14T01:12:32.123Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'start', NULL, '', '192.168.20.8', '2027-01-15T00:41:12.525Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'postgres-nightly-dump'), 'success', NULL, '', '192.168.20.8', '2027-01-15T00:59:01.095Z');
INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs, grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at, next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Backups'), 's3-offsite-sync', 'Mirrors last night''s dump to a second region.', '56635895-1e57-4cd9-aa72-cba0b5c28d83', 'period', 3600, 300, NULL, 'UTC', 'up', '2027-01-15T07:35:56.785Z', '2027-01-15T07:34:56.785Z', '2027-01-15T08:40:56.785Z', 1200, NULL, 0, '2026-12-01T08:00:00.000Z');
INSERT INTO check_channels (check_id, channel_id) VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), (SELECT id FROM channels WHERE name = '#ops-alerts'));
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.31', '2027-01-13T22:32:49.380Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.31', '2027-01-13T22:34:58.032Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-13T23:29:20.858Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-13T23:34:33.313Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.31', '2027-01-14T00:17:26.827Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.31', '2027-01-14T00:34:21.232Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '192.168.20.8', '2027-01-14T01:33:00.094Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '192.168.20.8', '2027-01-14T01:35:51.993Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-14T02:29:29.148Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-14T02:36:14.320Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-14T03:22:41.130Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-14T03:37:07.647Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '192.168.20.8', '2027-01-14T04:31:05.211Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '192.168.20.8', '2027-01-14T04:35:19.087Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.31', '2027-01-14T05:21:35.529Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.31', '2027-01-14T05:34:51.395Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-14T06:24:10.734Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-14T06:36:17.386Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '192.168.20.8', '2027-01-14T07:32:24.066Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '192.168.20.8', '2027-01-14T07:34:45.986Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-14T08:30:41.146Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-14T08:35:43.983Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '192.168.20.8', '2027-01-14T09:18:17.739Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '192.168.20.8', '2027-01-14T09:35:00.621Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.31', '2027-01-14T10:31:39.212Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.31', '2027-01-14T10:35:20.852Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.31', '2027-01-14T11:28:30.318Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.31', '2027-01-14T11:35:33.881Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-14T12:30:23.892Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-14T12:37:01.015Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.31', '2027-01-14T13:31:39.135Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.31', '2027-01-14T13:37:14.363Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.31', '2027-01-14T14:29:51.148Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.31', '2027-01-14T14:34:45.632Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-14T15:28:51.467Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-14T15:36:31.960Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.31', '2027-01-14T16:30:28.008Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.31', '2027-01-14T16:34:45.338Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-14T17:22:31.625Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-14T17:35:27.152Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-14T18:19:54.050Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-14T18:36:19.736Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.31', '2027-01-14T19:31:43.298Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.31', '2027-01-14T19:34:18.517Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '192.168.20.8', '2027-01-14T20:22:19.618Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '192.168.20.8', '2027-01-14T20:36:38.535Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.31', '2027-01-14T21:30:40.615Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.31', '2027-01-14T21:34:30.276Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-14T22:26:15.133Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-14T22:35:58.674Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '192.168.20.8', '2027-01-14T23:22:05.935Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '192.168.20.8', '2027-01-14T23:36:36.746Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-15T00:22:12.793Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-15T00:36:39.946Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-15T01:22:37.185Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-15T01:35:08.119Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-15T02:31:39.360Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-15T02:37:14.184Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.31', '2027-01-15T03:18:50.005Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.31', '2027-01-15T03:37:09.540Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-15T04:26:59.096Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-15T04:34:56.954Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.15', '2027-01-15T05:31:43.063Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.15', '2027-01-15T05:37:00.068Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.31', '2027-01-15T06:31:37.454Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.31', '2027-01-15T06:37:32.681Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'start', NULL, '', '10.4.2.31', '2027-01-15T07:23:31.388Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), 'success', NULL, '', '10.4.2.31', '2027-01-15T07:37:31.262Z');
INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs, grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at, next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Backups'), 'home-nas-snapshot', 'ZFS snapshot + scrub on the NAS. Missed its window last night.', 'e0257e7f-22f6-408f-aff6-628a01cba12b', 'period', 21600, 1800, NULL, 'UTC', 'down', '2027-01-14T19:15:00.000Z', '2027-01-14T19:14:00.000Z', '2027-01-15T01:45:00.000Z', 5400, '2027-01-15T07:50:00.000Z', 0, '2026-12-01T08:00:00.000Z');
INSERT INTO check_channels (check_id, channel_id) VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), (SELECT id FROM channels WHERE name = '#ops-alerts'));
INSERT INTO check_channels (check_id, channel_id) VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), (SELECT id FROM channels WHERE name = 'on-call phone'));
INSERT INTO check_channels (check_id, channel_id) VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), (SELECT id FROM channels WHERE name = 'alerts@example.com'));
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.31', '2027-01-06T12:44:02.044Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.31', '2027-01-06T13:23:49.241Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '192.168.20.8', '2027-01-06T18:42:05.539Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '192.168.20.8', '2027-01-06T19:18:18.540Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '192.168.20.8', '2027-01-06T23:54:27.132Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '192.168.20.8', '2027-01-07T01:13:29.558Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.15', '2027-01-07T06:41:29.251Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.15', '2027-01-07T07:11:47.271Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.15', '2027-01-07T12:12:39.826Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.15', '2027-01-07T13:05:00.009Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.31', '2027-01-07T18:35:18.390Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.31', '2027-01-07T19:04:18.831Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.15', '2027-01-08T00:49:47.621Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.15', '2027-01-08T01:20:48.844Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.31', '2027-01-08T06:39:20.462Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.31', '2027-01-08T07:16:33.350Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.15', '2027-01-08T12:44:37.372Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.15', '2027-01-08T13:15:54.899Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '192.168.20.8', '2027-01-08T18:42:15.451Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '192.168.20.8', '2027-01-08T19:16:42.597Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '192.168.20.8', '2027-01-09T00:21:05.002Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '192.168.20.8', '2027-01-09T01:07:33.134Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '192.168.20.8', '2027-01-09T06:08:03.436Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '192.168.20.8', '2027-01-09T07:25:03.940Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '192.168.20.8', '2027-01-09T12:22:53.083Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '192.168.20.8', '2027-01-09T13:05:25.510Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.15', '2027-01-09T18:35:04.440Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.15', '2027-01-09T19:10:26.605Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.31', '2027-01-10T00:53:54.203Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.31', '2027-01-10T01:25:30.718Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.15', '2027-01-10T06:53:23.836Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.15', '2027-01-10T07:20:32.815Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '192.168.20.8', '2027-01-10T12:38:06.117Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '192.168.20.8', '2027-01-10T13:23:53.403Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.15', '2027-01-10T18:32:05.800Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.15', '2027-01-10T19:21:51.122Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '192.168.20.8', '2027-01-11T00:52:25.154Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '192.168.20.8', '2027-01-11T01:19:35.388Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.31', '2027-01-11T06:15:31.033Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.31', '2027-01-11T07:04:46.429Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.31', '2027-01-11T12:04:25.171Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.31', '2027-01-11T13:18:16.332Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.31', '2027-01-11T18:14:42.180Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'fail', NULL, 'rsync: connection unexpectedly closed (0 bytes received so far)
rsync error: error in rsync protocol data stream (code 12) at io.c(228)', '10.4.2.31', '2027-01-11T19:04:44.580Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.15', '2027-01-12T00:56:15.556Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.15', '2027-01-12T01:21:57.619Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '192.168.20.8', '2027-01-12T06:35:09.488Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '192.168.20.8', '2027-01-12T07:08:05.550Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.31', '2027-01-12T12:27:52.715Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.31', '2027-01-12T13:15:06.387Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '192.168.20.8', '2027-01-12T18:33:30.544Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '192.168.20.8', '2027-01-12T19:07:21.359Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '192.168.20.8', '2027-01-13T00:40:25.059Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '192.168.20.8', '2027-01-13T01:23:41.652Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.15', '2027-01-13T06:39:18.025Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.15', '2027-01-13T07:20:11.967Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '192.168.20.8', '2027-01-13T12:18:08.391Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '192.168.20.8', '2027-01-13T13:10:10.570Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '192.168.20.8', '2027-01-13T17:48:18.809Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '192.168.20.8', '2027-01-13T19:05:06.657Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.31', '2027-01-14T00:41:22.673Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.31', '2027-01-14T01:09:33.525Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.31', '2027-01-14T06:20:26.343Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.31', '2027-01-14T07:04:22.542Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.15', '2027-01-14T12:50:26.331Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'success', NULL, '', '10.4.2.15', '2027-01-14T13:25:28.281Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'start', NULL, '', '10.4.2.15', '2027-01-14T18:26:32.254Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), 'fail', NULL, 'rsync: connection unexpectedly closed (0 bytes received so far)
rsync error: error in rsync protocol data stream (code 12) at io.c(228)', '10.4.2.15', '2027-01-14T19:12:11.925Z');
INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs, grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at, next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Backups'), 'photo-archive-verify', 'Weekly checksum sweep over the cold archive. Paused during the migration.', '3e4752bc-f190-43b3-a455-088696bdfc59', 'period', 604800, 86400, NULL, 'UTC', 'paused', '2027-01-12T12:48:00.000Z', '2027-01-12T12:47:00.000Z', '2027-01-20T12:48:00.000Z', NULL, NULL, 0, '2026-12-01T08:00:00.000Z');
INSERT INTO check_channels (check_id, channel_id) VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), (SELECT id FROM channels WHERE name = 'alerts@example.com'));
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '192.168.20.8', '2026-05-26T15:16:47.788Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '192.168.20.8', '2026-05-26T17:42:16.807Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '192.168.20.8', '2026-06-02T09:03:05.786Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '192.168.20.8', '2026-06-02T11:24:28.847Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.31', '2026-06-09T12:47:17.926Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.31', '2026-06-09T14:58:52.097Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.31', '2026-06-16T08:25:11.780Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.31', '2026-06-16T10:50:22.123Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '192.168.20.8', '2026-06-23T12:58:42.124Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '192.168.20.8', '2026-06-23T15:23:42.031Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.31', '2026-06-30T13:41:50.479Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.31', '2026-06-30T15:15:21.070Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.31', '2026-07-07T07:01:47.514Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.31', '2026-07-07T08:54:16.629Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '192.168.20.8', '2026-07-14T06:28:48.030Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '192.168.20.8', '2026-07-14T08:58:10.309Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.15', '2026-07-21T16:32:28.989Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.15', '2026-07-21T17:46:36.981Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.31', '2026-07-28T08:44:32.296Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.31', '2026-07-28T10:09:41.402Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.31', '2026-08-04T06:36:47.178Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.31', '2026-08-04T08:19:29.939Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.31', '2026-08-11T14:13:19.280Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.31', '2026-08-11T16:40:42.116Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.15', '2026-08-18T15:43:42.264Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.15', '2026-08-18T17:04:37.682Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.31', '2026-08-25T15:51:05.339Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.31', '2026-08-25T17:39:24.211Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '192.168.20.8', '2026-09-01T11:28:56.376Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '192.168.20.8', '2026-09-01T13:54:37.185Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '192.168.20.8', '2026-09-08T07:02:16.299Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '192.168.20.8', '2026-09-08T08:33:17.712Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '192.168.20.8', '2026-09-15T11:01:21.788Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '192.168.20.8', '2026-09-15T13:21:31.725Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.15', '2026-09-22T12:25:52.411Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.15', '2026-09-22T14:54:03.819Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.15', '2026-09-29T16:03:36.241Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.15', '2026-09-29T17:18:08.511Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.31', '2026-10-06T10:54:48.627Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.31', '2026-10-06T12:57:28.920Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.15', '2026-10-13T06:42:10.571Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.15', '2026-10-13T09:02:20.574Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '192.168.20.8', '2026-10-20T06:47:46.176Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '192.168.20.8', '2026-10-20T08:03:47.921Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '192.168.20.8', '2026-10-27T07:07:51.269Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '192.168.20.8', '2026-10-27T09:18:43.810Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.31', '2026-11-03T07:43:35.707Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.31', '2026-11-03T10:07:13.017Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '192.168.20.8', '2026-11-10T07:59:11.283Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '192.168.20.8', '2026-11-10T09:06:22.854Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '192.168.20.8', '2026-11-17T07:24:33.101Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '192.168.20.8', '2026-11-17T08:54:55.162Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.31', '2026-11-24T10:47:34.088Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.31', '2026-11-24T12:56:05.263Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.31', '2026-12-01T12:31:38.200Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.31', '2026-12-01T13:58:27.562Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.15', '2026-12-08T10:57:47.086Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.15', '2026-12-08T12:29:14.864Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.31', '2026-12-15T13:32:49.389Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.31', '2026-12-15T14:41:48.181Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.15', '2026-12-22T08:13:30.633Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.15', '2026-12-22T10:32:32.507Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.15', '2026-12-29T10:12:48.021Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.15', '2026-12-29T11:57:54.981Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '192.168.20.8', '2027-01-05T12:25:16.791Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '192.168.20.8', '2027-01-05T13:41:48.387Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'start', NULL, '', '10.4.2.15', '2027-01-12T07:53:38.475Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'photo-archive-verify'), 'success', NULL, '', '10.4.2.15', '2027-01-12T10:16:23.136Z');
INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs, grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at, next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Data pipeline'), 'etl-hourly', 'Extracts yesterday''s events into the warehouse.', 'ed6ed08e-0236-4a2c-a7e1-900ada1027d7', 'period', 3600, 600, NULL, 'UTC', 'up', '2027-01-15T07:03:00.000Z', '2027-01-15T07:54:00.000Z', '2027-01-15T08:13:00.000Z', 1500, NULL, 0, '2026-12-01T08:00:00.000Z');
INSERT INTO check_channels (check_id, channel_id) VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), (SELECT id FROM channels WHERE name = 'ntfy · data-pipeline'));
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '192.168.20.8', '2027-01-13T21:47:47.383Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '192.168.20.8', '2027-01-13T22:04:12.484Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-13T22:54:13.431Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-13T23:02:19.953Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-13T23:41:46.961Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-14T00:03:13.831Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.15', '2027-01-14T00:44:22.974Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.15', '2027-01-14T01:01:34.855Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-14T01:51:09.407Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-14T02:04:03.037Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '192.168.20.8', '2027-01-14T02:54:24.865Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '192.168.20.8', '2027-01-14T03:03:31.454Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.15', '2027-01-14T03:54:49.893Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.15', '2027-01-14T04:03:26.498Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '192.168.20.8', '2027-01-14T04:53:54.318Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '192.168.20.8', '2027-01-14T05:01:20.980Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-14T05:45:04.250Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-14T06:02:02.002Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-14T06:50:37.444Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-14T07:01:22.608Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '192.168.20.8', '2027-01-14T07:52:16.093Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '192.168.20.8', '2027-01-14T08:02:47.367Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.15', '2027-01-14T08:38:57.044Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.15', '2027-01-14T09:01:22.514Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '192.168.20.8', '2027-01-14T09:51:09.155Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '192.168.20.8', '2027-01-14T10:02:36.985Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-14T10:54:36.097Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-14T11:03:43.596Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.15', '2027-01-14T11:46:23.530Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.15', '2027-01-14T12:02:57.972Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.15', '2027-01-14T12:54:26.369Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.15', '2027-01-14T13:04:19.279Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-14T13:57:49.034Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-14T14:04:36.967Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-14T14:43:35.302Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-14T15:01:14.466Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-14T15:52:28.066Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-14T16:01:42.274Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-14T16:53:28.341Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-14T17:04:40.887Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-14T17:39:37.145Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-14T18:01:55.095Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.15', '2027-01-14T18:48:54.303Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.15', '2027-01-14T19:01:56.013Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-14T19:56:31.466Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-14T20:03:22.629Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '192.168.20.8', '2027-01-14T20:45:12.023Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '192.168.20.8', '2027-01-14T21:03:21.798Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-14T21:54:45.619Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-14T22:03:47.648Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-14T22:48:49.231Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-14T23:02:41.674Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-14T23:52:11.661Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-15T00:04:29.372Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '192.168.20.8', '2027-01-15T00:53:25.441Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '192.168.20.8', '2027-01-15T01:02:31.760Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.15', '2027-01-15T01:51:00.969Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.15', '2027-01-15T02:02:39.416Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.31', '2027-01-15T02:38:35.675Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.31', '2027-01-15T03:01:18.269Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.15', '2027-01-15T03:50:05.881Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.15', '2027-01-15T04:04:36.680Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.15', '2027-01-15T04:49:42.638Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.15', '2027-01-15T05:02:15.192Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '192.168.20.8', '2027-01-15T05:48:03.308Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '192.168.20.8', '2027-01-15T06:02:35.973Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.15', '2027-01-15T06:48:26.706Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'success', NULL, '', '10.4.2.15', '2027-01-15T07:01:29.222Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'etl-hourly'), 'start', NULL, '', '10.4.2.15', '2027-01-15T07:54:00.000Z');
INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs, grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at, next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Data pipeline'), 'dbt-run', 'Rebuilds the marts once the ETL lands.', '0198aee7-016f-45c7-a5c6-de69c3a83936', 'cron', NULL, 600, '0 15 */4 * * *', 'UTC', 'up', '2027-01-15T04:15:00.000Z', '2027-01-15T04:14:00.000Z', '2027-01-15T08:25:00.000Z', 1800, NULL, 0, '2026-12-01T08:00:00.000Z');
INSERT INTO check_channels (check_id, channel_id) VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), (SELECT id FROM channels WHERE name = 'ntfy · data-pipeline'));
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.15', '2027-01-09T15:57:04.423Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.15', '2027-01-09T16:17:20.664Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.15', '2027-01-09T20:10:38.194Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.15', '2027-01-09T20:21:08.544Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.15', '2027-01-09T23:56:09.070Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.15', '2027-01-10T00:20:52.954Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '192.168.20.8', '2027-01-10T03:57:32.806Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '192.168.20.8', '2027-01-10T04:16:34.700Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.15', '2027-01-10T07:51:24.446Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.15', '2027-01-10T08:08:48.677Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.15', '2027-01-10T11:47:59.981Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.15', '2027-01-10T12:09:54.834Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '192.168.20.8', '2027-01-10T15:46:04.048Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '192.168.20.8', '2027-01-10T16:09:13.830Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.15', '2027-01-10T19:53:03.048Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.15', '2027-01-10T20:13:31.025Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.31', '2027-01-11T00:03:18.180Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.31', '2027-01-11T00:18:59.189Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.31', '2027-01-11T04:05:50.158Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.31', '2027-01-11T04:15:52.007Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '192.168.20.8', '2027-01-11T07:53:37.502Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '192.168.20.8', '2027-01-11T08:08:45.258Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.31', '2027-01-11T11:42:56.263Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.31', '2027-01-11T12:09:27.119Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '192.168.20.8', '2027-01-11T15:52:35.417Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '192.168.20.8', '2027-01-11T16:16:47.266Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.15', '2027-01-11T19:55:39.829Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.15', '2027-01-11T20:18:34.912Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.31', '2027-01-11T23:53:32.668Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.31', '2027-01-12T00:15:22.017Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.31', '2027-01-12T03:57:57.495Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.31', '2027-01-12T04:14:36.228Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.31', '2027-01-12T07:48:52.125Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.31', '2027-01-12T08:12:38.798Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '192.168.20.8', '2027-01-12T12:01:07.977Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '192.168.20.8', '2027-01-12T12:19:06.500Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.31', '2027-01-12T16:01:43.714Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.31', '2027-01-12T16:21:44.074Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.31', '2027-01-12T19:52:15.129Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.31', '2027-01-12T20:10:14.378Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '192.168.20.8', '2027-01-12T23:50:29.521Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '192.168.20.8', '2027-01-13T00:17:36.157Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.31', '2027-01-13T04:01:43.204Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.31', '2027-01-13T04:12:24.525Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.15', '2027-01-13T08:02:34.954Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.15', '2027-01-13T08:16:30.000Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '192.168.20.8', '2027-01-13T11:45:16.756Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '192.168.20.8', '2027-01-13T12:09:03.007Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.15', '2027-01-13T15:58:01.240Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.15', '2027-01-13T16:12:01.408Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.31', '2027-01-13T19:55:25.337Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.31', '2027-01-13T20:08:03.448Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '192.168.20.8', '2027-01-13T23:43:37.284Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '192.168.20.8', '2027-01-14T00:08:17.319Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.15', '2027-01-14T03:49:07.781Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.15', '2027-01-14T04:14:07.025Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '192.168.20.8', '2027-01-14T08:01:22.816Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '192.168.20.8', '2027-01-14T08:17:54.858Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '192.168.20.8', '2027-01-14T11:49:00.688Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '192.168.20.8', '2027-01-14T12:14:17.726Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.15', '2027-01-14T16:04:07.884Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.15', '2027-01-14T16:16:05.659Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.15', '2027-01-14T19:52:58.200Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.15', '2027-01-14T20:12:23.072Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '10.4.2.15', '2027-01-14T23:58:09.389Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '10.4.2.15', '2027-01-15T00:15:13.555Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'start', NULL, '', '192.168.20.8', '2027-01-15T03:52:03.517Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'dbt-run'), 'success', NULL, '', '192.168.20.8', '2027-01-15T04:14:06.104Z');
INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs, grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at, next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Data pipeline'), 'feature-export', 'Pushes the feature table to the serving store. Running behind.', 'f14af316-d82e-4f00-a442-1093de0f0339', 'period', 1800, 600, NULL, 'UTC', 'up', '2027-01-15T07:25:00.000Z', '2027-01-15T07:24:00.000Z', '2027-01-15T08:05:00.000Z', 900, NULL, 0, '2026-12-01T08:00:00.000Z');
INSERT INTO check_channels (check_id, channel_id) VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), (SELECT id FROM channels WHERE name = 'ntfy · data-pipeline'));
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '192.168.20.8', '2027-01-14T14:49:03.458Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '192.168.20.8', '2027-01-14T14:54:37.437Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-14T15:14:11.146Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-14T15:25:25.610Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '192.168.20.8', '2027-01-14T15:40:53.190Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '192.168.20.8', '2027-01-14T15:54:23.257Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-14T16:17:29.778Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-14T16:24:19.326Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.31', '2027-01-14T16:49:28.983Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.31', '2027-01-14T16:54:46.087Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-14T17:19:15.746Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-14T17:25:46.040Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.31', '2027-01-14T17:47:12.041Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.31', '2027-01-14T17:54:22.026Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-14T18:15:36.328Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-14T18:24:33.275Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-14T18:48:34.686Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-14T18:54:43.100Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-14T19:12:58.890Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-14T19:24:23.416Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.31', '2027-01-14T19:46:45.955Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.31', '2027-01-14T19:54:54.520Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-14T20:12:05.271Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-14T20:25:09.669Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-14T20:46:58.606Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-14T20:54:08.623Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-14T21:14:14.497Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-14T21:25:33.482Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.31', '2027-01-14T21:50:12.789Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.31', '2027-01-14T21:55:22.967Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.31', '2027-01-14T22:21:34.295Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.31', '2027-01-14T22:25:17.738Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-14T22:50:59.234Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-14T22:55:43.984Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '192.168.20.8', '2027-01-14T23:19:54.905Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '192.168.20.8', '2027-01-14T23:24:07.707Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '192.168.20.8', '2027-01-14T23:49:32.007Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '192.168.20.8', '2027-01-14T23:55:32.372Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '192.168.20.8', '2027-01-15T00:18:13.982Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '192.168.20.8', '2027-01-15T00:24:15.969Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.31', '2027-01-15T00:42:51.841Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.31', '2027-01-15T00:55:16.494Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '192.168.20.8', '2027-01-15T01:16:05.172Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '192.168.20.8', '2027-01-15T01:24:56.197Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.31', '2027-01-15T01:47:12.900Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.31', '2027-01-15T01:54:16.995Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-15T02:14:01.290Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-15T02:24:10.013Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '192.168.20.8', '2027-01-15T02:45:02.831Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '192.168.20.8', '2027-01-15T02:55:22.165Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-15T03:18:57.751Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-15T03:25:08.407Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-15T03:47:26.210Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-15T03:54:39.137Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '192.168.20.8', '2027-01-15T04:14:17.472Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '192.168.20.8', '2027-01-15T04:25:38.718Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-15T04:46:30.235Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-15T04:54:43.788Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.31', '2027-01-15T05:12:39.886Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.31', '2027-01-15T05:25:51.347Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '192.168.20.8', '2027-01-15T05:49:11.104Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '192.168.20.8', '2027-01-15T05:54:43.407Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '192.168.20.8', '2027-01-15T06:18:21.800Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '192.168.20.8', '2027-01-15T06:24:40.061Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-15T06:47:27.659Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-15T06:55:00.374Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'start', NULL, '', '10.4.2.15', '2027-01-15T07:14:18.108Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'feature-export'), 'success', NULL, '', '10.4.2.15', '2027-01-15T07:24:31.328Z');
INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs, grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at, next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Website'), 'certbot-renew', 'Weekly ACME renewal for `www` and the wildcard.', 'b6731471-0278-47b7-a3c9-ae2386f69c4b', 'cron', NULL, 21600, '0 0 4 * * 1', 'UTC', 'up', '2027-01-10T04:00:00.000Z', '2027-01-10T03:59:00.000Z', '2027-01-17T10:00:00.000Z', 240, NULL, 0, '2026-12-01T08:00:00.000Z');
INSERT INTO check_channels (check_id, channel_id) VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), (SELECT id FROM channels WHERE name = '#ops-alerts'));
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2026-05-23T23:09:52.818Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2026-05-23T23:10:47.403Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '192.168.20.8', '2026-05-31T05:48:21.720Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '192.168.20.8', '2026-05-31T05:49:29.673Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '192.168.20.8', '2026-06-07T06:52:50.756Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '192.168.20.8', '2026-06-07T06:56:28.224Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2026-06-14T06:40:01.177Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2026-06-14T06:42:45.132Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2026-06-21T07:04:18.866Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2026-06-21T07:06:55.605Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2026-06-28T07:10:30.528Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2026-06-28T07:13:14.529Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '192.168.20.8', '2026-07-04T22:55:46.115Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '192.168.20.8', '2026-07-04T22:58:31.065Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2026-07-12T06:25:04.979Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2026-07-12T06:25:48.653Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2026-07-19T00:21:50.183Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2026-07-19T00:23:23.847Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2026-07-26T08:13:52.801Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2026-07-26T08:15:48.844Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.15', '2026-08-02T05:47:45.980Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.15', '2026-08-02T05:50:33.542Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '192.168.20.8', '2026-08-09T00:47:53.867Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '192.168.20.8', '2026-08-09T00:51:33.127Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2026-08-16T02:18:43.746Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2026-08-16T02:21:04.129Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.15', '2026-08-23T01:33:49.658Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.15', '2026-08-23T01:34:49.315Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2026-08-30T07:53:22.259Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2026-08-30T07:55:44.633Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2026-09-06T07:15:43.064Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2026-09-06T07:17:13.386Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.15', '2026-09-13T01:00:18.732Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.15', '2026-09-13T01:02:44.331Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.15', '2026-09-20T08:43:23.105Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.15', '2026-09-20T08:44:40.514Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2026-09-27T02:36:55.885Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2026-09-27T02:38:55.710Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2026-10-04T04:11:08.912Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2026-10-04T04:13:30.254Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.15', '2026-10-10T23:52:27.033Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.15', '2026-10-10T23:55:59.558Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.15', '2026-10-18T02:43:31.315Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.15', '2026-10-18T02:46:06.740Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.15', '2026-10-25T07:25:37.502Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.15', '2026-10-25T07:28:03.807Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '192.168.20.8', '2026-11-01T08:38:07.358Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '192.168.20.8', '2026-11-01T08:40:01.958Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '192.168.20.8', '2026-11-08T06:30:28.238Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '192.168.20.8', '2026-11-08T06:33:06.819Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '192.168.20.8', '2026-11-15T00:15:44.965Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '192.168.20.8', '2026-11-15T00:16:52.958Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.15', '2026-11-22T06:00:08.800Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.15', '2026-11-22T06:01:18.341Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.15', '2026-11-29T06:56:27.592Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.15', '2026-11-29T06:58:54.982Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '192.168.20.8', '2026-12-06T02:22:49.049Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '192.168.20.8', '2026-12-06T02:24:21.829Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '192.168.20.8', '2026-12-13T01:09:50.205Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '192.168.20.8', '2026-12-13T01:13:14.777Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.15', '2026-12-20T03:17:50.940Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.15', '2026-12-20T03:18:43.016Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2026-12-27T07:45:27.991Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2026-12-27T07:47:45.683Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.15', '2027-01-03T02:59:27.965Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.15', '2027-01-03T03:01:10.696Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'start', NULL, '', '10.4.2.31', '2027-01-09T23:24:44.754Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'certbot-renew'), 'success', NULL, '', '10.4.2.31', '2027-01-09T23:26:03.423Z');
INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs, grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at, next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Website'), 'sitemap-rebuild', '', 'e8088dc8-25cc-41e2-abbf-7ee7c9dd5768', 'period', 86400, 7200, NULL, 'UTC', 'up', '2027-01-14T20:44:06.001Z', '2027-01-14T20:43:06.001Z', '2027-01-15T22:44:06.001Z', 900, NULL, 0, '2026-12-01T08:00:00.000Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '192.168.20.8', '2026-12-12T19:56:32.710Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '192.168.20.8', '2026-12-12T20:01:57.976Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '192.168.20.8', '2026-12-13T20:55:41.099Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '192.168.20.8', '2026-12-13T21:02:03.735Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '192.168.20.8', '2026-12-14T20:46:04.672Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '192.168.20.8', '2026-12-14T20:59:21.585Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.15', '2026-12-15T21:16:39.251Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.15', '2026-12-15T21:25:33.447Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '192.168.20.8', '2026-12-16T20:40:51.354Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '192.168.20.8', '2026-12-16T20:46:04.236Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.15', '2026-12-17T20:48:57.792Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.15', '2026-12-17T20:53:05.783Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.15', '2026-12-18T21:19:22.990Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.15', '2026-12-18T21:25:11.432Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.15', '2026-12-19T20:09:01.582Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.15', '2026-12-19T20:13:39.593Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.15', '2026-12-20T20:02:03.357Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.15', '2026-12-20T20:12:18.975Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.15', '2026-12-21T20:31:39.268Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.15', '2026-12-21T20:41:59.694Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '192.168.20.8', '2026-12-22T20:06:35.232Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '192.168.20.8', '2026-12-22T20:14:11.218Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.31', '2026-12-23T19:51:21.193Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.31', '2026-12-23T20:03:39.383Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.15', '2026-12-24T20:11:12.790Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.15', '2026-12-24T20:16:43.161Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.15', '2026-12-25T20:22:48.583Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.15', '2026-12-25T20:29:24.125Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.15', '2026-12-26T19:57:45.359Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.15', '2026-12-26T20:03:06.194Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.31', '2026-12-27T20:21:22.597Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.31', '2026-12-27T20:25:25.013Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.15', '2026-12-28T20:57:02.345Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.15', '2026-12-28T21:06:59.458Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.31', '2026-12-29T20:30:03.594Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.31', '2026-12-29T20:34:36.865Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '192.168.20.8', '2026-12-30T20:42:46.197Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '192.168.20.8', '2026-12-30T20:53:17.047Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '192.168.20.8', '2026-12-31T20:21:10.386Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '192.168.20.8', '2026-12-31T20:27:39.766Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.15', '2027-01-01T19:56:23.924Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.15', '2027-01-01T20:09:55.793Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.31', '2027-01-02T20:35:12.318Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.31', '2027-01-02T20:41:30.590Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.15', '2027-01-03T21:08:44.794Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.15', '2027-01-03T21:17:07.473Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.31', '2027-01-04T21:05:57.998Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.31', '2027-01-04T21:14:16.039Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.31', '2027-01-05T21:13:22.630Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.31', '2027-01-05T21:20:49.819Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '192.168.20.8', '2027-01-06T21:08:42.555Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '192.168.20.8', '2027-01-06T21:14:57.520Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.31', '2027-01-07T20:40:17.766Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.31', '2027-01-07T20:44:40.793Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.15', '2027-01-08T20:55:34.107Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.15', '2027-01-08T21:04:39.950Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '192.168.20.8', '2027-01-09T21:07:09.245Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '192.168.20.8', '2027-01-09T21:11:20.164Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.31', '2027-01-10T20:09:49.681Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.31', '2027-01-10T20:22:26.858Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '192.168.20.8', '2027-01-11T21:00:53.310Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '192.168.20.8', '2027-01-11T21:10:08.195Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.31', '2027-01-12T20:42:11.206Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.31', '2027-01-12T20:45:21.366Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '192.168.20.8', '2027-01-13T20:41:04.197Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '192.168.20.8', '2027-01-13T20:44:55.857Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'start', NULL, '', '10.4.2.31', '2027-01-14T20:14:12.964Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'sitemap-rebuild'), 'success', NULL, '', '10.4.2.31', '2027-01-14T20:23:13.450Z');
INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs, grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at, next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Website'), 'broken-link-sweep', 'Crawls the docs for dead links.', '8c33af1b-6022-4806-ac5e-f3e17d506f9f', 'period', 43200, 3600, NULL, 'UTC', 'up', '2027-01-15T02:18:10.984Z', '2027-01-15T02:17:10.984Z', '2027-01-15T15:18:10.984Z', 2400, NULL, 0, '2026-12-01T08:00:00.000Z');
INSERT INTO check_channels (check_id, channel_id) VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), (SELECT id FROM channels WHERE name = '#ops-alerts'));
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '192.168.20.8', '2026-12-29T13:59:03.976Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '192.168.20.8', '2026-12-29T14:22:52.627Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2026-12-30T01:57:27.269Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2026-12-30T02:12:28.586Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2026-12-30T13:41:40.893Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2026-12-30T14:17:59.525Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '192.168.20.8', '2026-12-31T02:10:54.161Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '192.168.20.8', '2026-12-31T02:30:52.902Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2026-12-31T13:58:10.386Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2026-12-31T14:22:04.166Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.15', '2027-01-01T02:00:39.489Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.15', '2027-01-01T02:30:33.534Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-01T14:02:44.720Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-01T14:17:43.487Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-02T01:56:16.646Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-02T02:22:38.719Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-02T14:03:52.164Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-02T14:18:10.308Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '192.168.20.8', '2027-01-03T01:33:28.910Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '192.168.20.8', '2027-01-03T02:01:58.704Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.15', '2027-01-03T13:27:24.964Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.15', '2027-01-03T13:59:13.077Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-04T01:41:39.357Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-04T02:15:12.325Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.15', '2027-01-04T14:11:15.180Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.15', '2027-01-04T14:38:45.259Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '192.168.20.8', '2027-01-05T02:15:58.944Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '192.168.20.8', '2027-01-05T02:37:46.390Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '192.168.20.8', '2027-01-05T13:56:09.285Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '192.168.20.8', '2027-01-05T14:10:58.131Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '192.168.20.8', '2027-01-06T01:38:40.295Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '192.168.20.8', '2027-01-06T02:10:12.746Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.15', '2027-01-06T14:14:09.524Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.15', '2027-01-06T14:38:35.707Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-07T01:58:22.487Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-07T02:13:19.262Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.15', '2027-01-07T13:28:19.775Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.15', '2027-01-07T14:03:14.455Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.15', '2027-01-08T02:11:56.367Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.15', '2027-01-08T02:32:41.421Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-08T13:28:49.680Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-08T14:04:06.911Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '192.168.20.8', '2027-01-09T01:51:24.579Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '192.168.20.8', '2027-01-09T02:19:22.448Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '192.168.20.8', '2027-01-09T14:09:53.516Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '192.168.20.8', '2027-01-09T14:35:08.199Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-10T01:41:00.447Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-10T02:06:37.840Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-10T13:44:25.074Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-10T14:10:55.740Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-11T02:22:31.807Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-11T02:36:26.541Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.15', '2027-01-11T14:14:05.572Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.15', '2027-01-11T14:30:44.244Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-12T01:56:08.439Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-12T02:10:24.033Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-12T13:56:11.404Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-12T14:16:23.353Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.15', '2027-01-13T01:22:10.590Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.15', '2027-01-13T01:58:06.829Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-13T13:50:26.347Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-13T14:19:35.196Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-14T01:37:07.906Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-14T02:04:20.194Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.15', '2027-01-14T14:06:41.345Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.15', '2027-01-14T14:26:11.403Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'start', NULL, '', '10.4.2.31', '2027-01-15T01:55:42.832Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'broken-link-sweep'), 'success', NULL, '', '10.4.2.31', '2027-01-15T02:12:04.186Z');
INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs, grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at, next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)
  VALUES ((SELECT id FROM projects WHERE name = 'Staging (sam)'), 'staging-db-reseed', '', '0700f814-479f-4bbf-a0f3-ce698710a1ff', 'period', 86400, 3600, NULL, 'UTC', 'up', '2027-01-14T22:42:42.828Z', '2027-01-14T22:41:42.828Z', '2027-01-15T23:42:42.828Z', NULL, NULL, 0, '2026-12-01T08:00:00.000Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2026-12-12T22:30:31.057Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2026-12-12T22:41:39.480Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2026-12-13T22:51:52.254Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2026-12-13T23:03:29.072Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2026-12-14T22:00:24.805Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2026-12-14T22:13:32.202Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '192.168.20.8', '2026-12-15T22:54:16.159Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '192.168.20.8', '2026-12-15T23:05:06.631Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.15', '2026-12-16T21:52:51.384Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.15', '2026-12-16T22:03:40.084Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '192.168.20.8', '2026-12-17T23:17:27.017Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '192.168.20.8', '2026-12-17T23:24:27.977Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '192.168.20.8', '2026-12-18T22:32:19.507Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '192.168.20.8', '2026-12-18T22:42:46.983Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2026-12-19T22:39:33.550Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2026-12-19T22:51:54.202Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '192.168.20.8', '2026-12-20T22:22:11.027Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '192.168.20.8', '2026-12-20T22:29:29.237Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.15', '2026-12-21T21:51:40.970Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.15', '2026-12-21T22:03:18.039Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2026-12-22T22:50:47.122Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2026-12-22T22:56:27.273Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2026-12-23T22:32:51.133Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2026-12-23T22:46:02.703Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '192.168.20.8', '2026-12-24T22:24:55.189Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '192.168.20.8', '2026-12-24T22:33:54.011Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.15', '2026-12-25T22:06:39.822Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.15', '2026-12-25T22:13:52.849Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.15', '2026-12-26T23:03:24.037Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.15', '2026-12-26T23:12:23.169Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2026-12-27T21:52:28.587Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2026-12-27T22:05:03.088Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2026-12-28T23:16:24.796Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2026-12-28T23:23:34.579Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '192.168.20.8', '2026-12-29T22:46:42.581Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '192.168.20.8', '2026-12-29T22:58:40.768Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '192.168.20.8', '2026-12-30T22:13:02.476Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '192.168.20.8', '2026-12-30T22:19:02.521Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2026-12-31T22:18:48.363Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2026-12-31T22:30:53.589Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '192.168.20.8', '2027-01-01T23:00:09.950Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '192.168.20.8', '2027-01-01T23:05:26.147Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.15', '2027-01-02T23:11:38.172Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.15', '2027-01-02T23:24:17.906Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2027-01-03T22:00:12.253Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2027-01-03T22:12:24.087Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2027-01-04T22:44:03.217Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2027-01-04T22:54:22.928Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2027-01-05T23:03:53.437Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2027-01-05T23:16:29.159Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.15', '2027-01-06T22:20:23.681Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.15', '2027-01-06T22:32:34.147Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2027-01-07T22:03:09.950Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2027-01-07T22:15:52.364Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2027-01-08T22:11:01.726Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2027-01-08T22:19:20.969Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '192.168.20.8', '2027-01-09T22:43:36.685Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '192.168.20.8', '2027-01-09T22:56:26.544Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '192.168.20.8', '2027-01-10T21:52:31.759Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '192.168.20.8', '2027-01-10T22:01:45.549Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2027-01-11T22:50:05.463Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2027-01-11T22:57:10.077Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '192.168.20.8', '2027-01-12T21:58:10.939Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '192.168.20.8', '2027-01-12T22:10:16.802Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '192.168.20.8', '2027-01-13T22:50:57.818Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '192.168.20.8', '2027-01-13T22:59:16.981Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'start', NULL, '', '10.4.2.31', '2027-01-14T22:11:52.429Z');
INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'staging-db-reseed'), 'success', NULL, '', '10.4.2.31', '2027-01-14T22:17:01.295Z');
INSERT INTO notifications (check_id, channel_id, event, status, error, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), (SELECT id FROM channels WHERE name = '#ops-alerts'), 'down', 'ok', NULL, '2027-01-14T06:00:00.000Z');
INSERT INTO notifications (check_id, channel_id, event, status, error, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 's3-offsite-sync'), (SELECT id FROM channels WHERE name = '#ops-alerts'), 'up', 'ok', NULL, '2027-01-14T06:36:00.000Z');
INSERT INTO notifications (check_id, channel_id, event, status, error, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), (SELECT id FROM channels WHERE name = '#ops-alerts'), 'down', 'ok', NULL, '2027-01-15T06:58:00.000Z');
INSERT INTO notifications (check_id, channel_id, event, status, error, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), (SELECT id FROM channels WHERE name = 'on-call phone'), 'down', 'ok', NULL, '2027-01-15T06:58:00.000Z');
INSERT INTO notifications (check_id, channel_id, event, status, error, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), (SELECT id FROM channels WHERE name = 'alerts@example.com'), 'down', 'error', 'smtp: connection refused', '2027-01-15T06:58:00.000Z');
INSERT INTO notifications (check_id, channel_id, event, status, error, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), (SELECT id FROM channels WHERE name = '#ops-alerts'), 'reminder', 'ok', NULL, '2027-01-15T07:28:00.000Z');
INSERT INTO notifications (check_id, channel_id, event, status, error, created_at)
  VALUES ((SELECT id FROM checks WHERE name = 'home-nas-snapshot'), (SELECT id FROM channels WHERE name = 'on-call phone'), 'reminder', 'ok', NULL, '2027-01-15T07:28:00.000Z');
INSERT INTO audit_log (actor_user_id, actor_username, action, target_type, target_id, target_owner_id, method, path, detail, created_at)
  VALUES ((SELECT id FROM users WHERE username = 'demo'), 'demo', 'user.create', 'user', 3, 3, 'POST', '/admin/users', 'username=sam is_admin=false', '2026-12-16T08:00:00.000Z');
INSERT INTO audit_log (actor_user_id, actor_username, action, target_type, target_id, target_owner_id, method, path, detail, created_at)
  VALUES ((SELECT id FROM users WHERE username = 'demo'), 'demo', 'user.set_admin', 'user', 2, 2, 'POST', '/admin/users/2/admin', 'is_admin=true', '2026-12-20T08:00:00.000Z');
INSERT INTO audit_log (actor_user_id, actor_username, action, target_type, target_id, target_owner_id, method, path, detail, created_at)
  VALUES ((SELECT id FROM users WHERE username = 'maya'), 'maya', 'user.password_reset', 'user', 3, 3, 'POST', '/admin/users/3/password', NULL, '2027-01-09T08:00:00.000Z');
INSERT INTO audit_log (actor_user_id, actor_username, action, target_type, target_id, target_owner_id, method, path, detail, created_at)
  VALUES ((SELECT id FROM users WHERE username = 'maya'), 'maya', 'admin.access', 'project', 1, 1, 'GET', '/admin/projects/1', NULL, '2027-01-15T05:30:00.000Z');
INSERT INTO audit_log (actor_user_id, actor_username, action, target_type, target_id, target_owner_id, method, path, detail, created_at)
  VALUES ((SELECT id FROM users WHERE username = 'maya'), 'maya', 'admin.access', 'check', 3, 3, 'GET', '/admin/checks/3', NULL, '2027-01-15T05:36:00.000Z');
INSERT INTO settings (key, value) VALUES ('scan_interval', '30')
  ON CONFLICT(key) DO UPDATE SET value = excluded.value;
INSERT INTO settings (key, value) VALUES ('nag_interval', '1800')
  ON CONFLICT(key) DO UPDATE SET value = excluded.value;
INSERT INTO settings (key, value) VALUES ('pings_retention_days', '90')
  ON CONFLICT(key) DO UPDATE SET value = excluded.value;
INSERT INTO settings (key, value) VALUES ('notifications_retention_days', '90')
  ON CONFLICT(key) DO UPDATE SET value = excluded.value;
COMMIT;