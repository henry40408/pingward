# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

pingward is a self-hosted, healthchecks-style uptime/cron monitor. Jobs "ping"
a per-check URL; a background loop marks a check **down** when a ping is overdue
and delivers notifications through per-check channels. It serves both a
server-rendered web UI (Askama templates) and machine `/ping/*` endpoints from a
single axum process.

## Commands

Build / run:
- `cargo build` — **required after any template or route change**; Askama
  templates are compiled into the binary, and the E2E harness runs the compiled
  `target/debug/pingward`.
- `cargo run` — starts the server (defaults: SQLite file `pingward.sqlite3`,
  bind `127.0.0.1:8080`). Override via env (see Config below).

Lint / format (must pass in CI):
- `cargo fmt --all --check`
- `cargo clippy --all-targets -- -D warnings`

Rust tests (use `cargo nextest`, not `cargo test` — CI does):
- `cargo nextest run` — full suite. SQLite-backed tests run unconditionally.
- Single test: `cargo nextest run -E 'test(success_ping_marks_up)'` (or pass a
  substring: `cargo nextest run success_ping`).
- Postgres integration tests (`tests/pg_store.rs`) **silently skip** unless
  `TEST_DATABASE_URL=postgres://…` is set; SMTP delivery tests
  (`tests/smtp_e2e.rs`) skip unless `PINGWARD_TEST_SMTP_HOST` is set. Start both
  backends with `docker compose up -d` (Postgres on 5432, mailpit on 1025/8025),
  then export `TEST_DATABASE_URL`, `PINGWARD_TEST_SMTP_HOST=localhost`,
  `PINGWARD_TEST_SMTP_PORT=1025`, `PINGWARD_TEST_MAILPIT_API=http://localhost:8025`.

Browser E2E (Playwright + playwright-bdd, in `e2e/`):
- `cd e2e && npm test` — runs `bddgen` (generates specs from `.feature` +
  `.steps.js`) then `playwright test`. A `global-setup` runs `cargo build`
  first; each scenario spawns a **fresh binary + temp SQLite DB** on a random
  port. Selectors use `data-testid`.
- Single feature/scenario: `cd e2e && npx bddgen && npx playwright test ping_kinds -g "POST body"`.

## Architecture

**Router composition** (`src/lib.rs::app`) merges three sibling routers under
one `AppState`:
- `web::routes()` — the browser UI, wrapped in the `csrf_guard` middleware.
- `ping::routes()` — machine `/ping/*` endpoints. Merged as a sibling, so they
  are **structurally exempt from CSRF** (public, unauthenticated).
- `assets::routes()` + `/healthz`.

**Background loops** (`src/main.rs` spawns two tokio tasks, after building
`AppState` so both loops and the HTTP server share `state.events`):
- `scheduler::run_scan_loop` — periodically re-evaluates every check's
  `due_time`, transitions overdue checks to down, and fires notifications.
- `prune::run_prune_loop` — deletes old pings/notifications and expired sessions.

**Live-tail signal bus**: `AppState::events` (`broadcast::Sender<i64>`) carries
a `check_id` whenever that check changes — published by `ping::apply` (every
ping kind, even `Log`/paused checks) and `scheduler::run_scan_loop` (each
`Down` transition), both gated on `receiver_count() > 0` so it's free when
unwatched. `GET /checks/{id}/events` / `/admin/checks/{id}/events`
(`web::sse_for_check`) turn it into an SSE stream carrying no data — the
browser is expected to re-fetch the existing `/checks/{id}/pings` fragment on
each `"changed"` event, keeping rendering/auth in one place. In-process only:
not shared across replicas (see ARCHITECTURE.md).

**Persistence** (`src/db.rs`, `src/store.rs`): one sqlx `AnyPool` that dispatches
to **SQLite or Postgres by URL scheme**. All queries go through `Store` and must
work on both backends — use `$N` placeholders + `RETURNING id` (the `Any` driver
does **not** translate `?`). Migrations are duplicated in `migrations/sqlite/`
and `migrations/postgres/`; `db::migrate` picks the directory from the URL, so a
schema change means writing the SQL **in both**. SQLite pragmas (foreign keys,
busy_timeout, WAL for file DBs) are applied per-connection in `db::connect`.

**Auth & authorization** (`src/auth.rs`):
- Session cookie (`pingward_session`) + argon2 password hashing. An optional
  trusted forward-auth header auto-provisions a passwordless non-admin user.
- Request extractors: `CurrentUser` (401/redirect if none), `OptionalUser`,
  `AdminUser` (403 if not admin).
- Owner scoping goes through `owned_project` / `owned_check` in `web.rs`, which
  return **404 (not 403)** for another user's resource — existence is hidden.
- `/account` is the per-user account page (sessions, then API keys, stacked as
  ordinary cards — no tabs). It lets a user list and revoke their own login
  sessions (each row's `last_seen_at` is refreshed on use, throttled like
  `ApiKey.last_used_at`); since `sessions.id` is the cookie's bearer secret,
  rows are identified in the UI/URLs by a SHA-256 handle
  (`apikey::hash_api_key`) rather than the id itself. A session's stored IP
  comes from `auth::client_ip`: the socket peer, unless that peer is a
  configured trusted proxy, in which case the first `X-Forwarded-For` entry
  wins — the same trust gate `forward_auth_username` uses, so an untrusted
  caller cannot spoof it.
- `/admin` is the single merged admin page (each handler guarded by
  `AdminUser`): site-wide overview, global settings, user management, and
  every project across all users, stacked as ordinary cards top to bottom —
  no tabs, no sub-nav, mirroring how `/account` merges its sections. Former
  `/settings` and `/users` POST routes moved under `/admin/…`
  (`/admin/settings`, `/admin/users`, `/admin/users/{id}/…`) so path grouping
  matches permission grouping. Deeper per-project/per-check cross-user
  management still lives under `/admin/projects/{id}`, `/admin/checks/{id}`,
  etc. — those handlers **reuse the owner templates** by passing an
  `is_admin`/base-prefix flag, so `data-testid`s and most step definitions
  are shared with the owner flow.
  An admin can never delete, disable, or demote their own account — the "All
  users" row renders those controls inert (delete/toggle-admin/toggle-disabled
  become a `<span class="btn disabled">`; reset password stays live) and the
  handlers refuse the same self-targeted request with a one-shot flash.

**Scheduling** (`src/scheduler.rs`, `src/config.rs`): a check is `Period`
(interval) or `Cron` (6-field `sec min hour dom mon dow`, evaluated in the
check's timezone). `due_time` anchors on the last success (else creation) plus
period/cron + grace. Scan and nag/reminder intervals resolve through a
**check → project → global → env** cascade (`effective_scan_interval` /
`effective_nag_interval`); non-positive overrides fall through. Duration form
fields (period/grace/scan/max-runtime/nag overrides, plus the settings-page
scan/nag intervals) accept either raw seconds or a human-readable string
(`5m`, `1h30m`, `2d`) via `duration::parse_duration`, are always stored as
seconds, and are re-rendered on forms via `duration::fmt_duration`; the
retention-days settings fields are plain integers, not durations.
`view::fmt_secs` remains the lossy *display* format used elsewhere (e.g. the
heartbeat strip tooltips, `fmt_secs(d) / fmt_secs(c)`).

**Notifications** (`src/notify.rs`): a `Notifier` trait with six implementations
(`webhook`, `telegram`, `slack`, `ntfy`, `pushover`, `email`/SMTP). `notifier_for`
builds one from a stored `Channel`; `deliver_event` applies a `RetryPolicy`.
Delivery is fire-and-forget (`tokio::spawn`) so a ping response is never blocked
on notification I/O. Instance SMTP is configured via env (`config::SmtpConfig`).

**Models** (`src/models.rs`): string-backed enums (`CheckStatus`, `PingKind`,
`ScheduleKind`, `ChannelKind`, …) are generated by the `str_enum!` macro, which
also derives `as_str()` / `FromStr` — add variants there.

## Config (env vars, `src/config.rs`)

`DATABASE_URL` (default `sqlite://pingward.sqlite3?mode=rwc`), `PINGWARD_BIND`,
`PINGWARD_BASE_URL` (used to render ping URLs), `PINGWARD_SCAN_INTERVAL`,
`PINGWARD_PRUNE_INTERVAL_SECS`, `PINGWARD_LOG_FORMAT` (`text` default, or `json`
for line-delimited structured logs — parsed into `config::LogFormat`, applied by
`init_tracing` in `main.rs`), `PINGWARD_FORWARD_AUTH_HEADER` +
`PINGWARD_TRUSTED_PROXIES`, and `PINGWARD_SMTP_*` (host/from required to enable
email; port/TLS defaulted). The scan and prune interval env vars accept raw
seconds or a human-readable duration (`5m`, `1h30m`) via
`duration::parse_duration`; an unparseable value falls back to the default
rather than failing at boot. `Config::from_map` is the testable core —
unit-test config parsing through it rather than real env. These env vars are
also surfaced read-only on `/admin`'s "Environment" card, with secrets (the
SMTP password) shown only as configured/not-set, never their value.
