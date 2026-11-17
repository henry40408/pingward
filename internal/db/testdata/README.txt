Fixtures captured from the published Rust image ghcr.io/henry40408/pingward:latest
(v0.12.0, revision ca5682f, the same commit this branch was cut from), not from a
local cargo build. Re-capture: scripts live outside the repo (job tmp dir pw-capture/).

rust-created.sqlite3
  Created through HTTP only, PINGWARD_SECRET=0123456789abcdef0123456789abcdef-pingward-capture.
  Accounts (username / plaintext password):
    admin / correct horse battery staple   (is_admin=1)
    bob   / 日本語のパスワード十五文字以上です (promoted to admin by the capture, is_admin=1)
    carol / carol carol carol carol        (is_admin=0, no projects)
  No forward-auth user (password_hash NULL) was captured: needs a second server run
  with PINGWARD_FORWARD_AUTH_HEADER. Not done.
  API keys: "ci" (prefix pw_3c1ab0d7, expires 30d; plaintext not kept),
            "script" (pw_00a0e648fc5141e6888cfb1e3e5400fbb5460013c9f84498ba4a7f8d7910186f).
  _sqlx_migrations has 16 rows. Row counts: api_keys 2, audit_log 10, channels 7,
  check_channels 6, checks 6, notifications 47, pings 14, projects 3, sessions 3,
  settings 8, users 3.
  Checkpointed with PRAGMA wal_checkpoint(TRUNCATE): a single file.
  Channels: ch-webhook, ch-slack, ch-telegram, ch-ntfy, ch-pushover, ch-email (project 1),
  bob-hook (project 4, bob).
  Checks: nightly-backup(period,max_runtime,nag), weekly-report(cron, Asia/Taipei),
  heartbeat, flaky(period 2s, down/up/reminder notifications), slow-job(overrun), bob-check.
  Notification rows cover down(overdue, failed, overrun), up, reminder.

sqlx_migrations.sqlite.txt
  version description sha384-hex, read from the real _sqlx_migrations of the captured DB.
  All 16 checksums equal SHA-384 of migrations/sqlite/<n>_*.sql (verified).
  Table DDL: see `sqlite3 rust-created.sqlite3 .schema _sqlx_migrations`.

sqlx_migrations.postgres.txt
  DERIVED, NOT CAPTURED: sha384 of migrations/postgres/*.sql using the rule verified on
  SQLite (sqlx checksum = SHA-384 of the file bytes). A real Postgres capture needs a
  second container (not done under the one-container rule).
