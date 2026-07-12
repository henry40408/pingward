# pingward — Design Spec

- **Date:** 2026-07-12
- **Status:** Approved (design), pending implementation plan
- **Working name:** `pingward` (preliminary availability screened: crates.io/npm free, `.dev`/`.app`/`.io` registrable, no same-name product/trademark found — NOT a formal trademark clearance; do a proper clearance before any commercial use)

## 1. Overview

`pingward` is a self-hostable, multi-user web application for monitoring cron jobs (and any scheduled task) using the **dead-man's-switch** pattern: each monitored job periodically sends an HTTP request ("ping") to report success / failure / logs. If a job fails to check in within its expected schedule (plus a grace period), `pingward` proactively notifies the user through configured channels.

Conceptually similar to Healthchecks.io, but scoped deliberately smaller for a first version.

### Goals

- Receive check-in pings from jobs over plain HTTP (works with `curl`, k8s, Lambda, anything).
- Detect overdue jobs and notify the user without the user having to poll.
- Support both simple period-based and cron-expression-based schedules.
- Multi-user, self-host friendly, single binary deployment.

### Non-goals (v1)

- Email notification channel (deferred; the notifier abstraction makes it a drop-in later).
- Repeated "nag" re-notifications while a job stays down (deferred; v1 notifies on state transition only).
- Billing / plans / quotas.
- Metrics dashboards beyond a per-check event list.

## 2. Tech Stack

Aligned with the user's existing Rust projects (`rdrs`, `noadd`, `lur`):

- **Language / runtime:** Rust + `tokio`
- **HTTP framework:** `axum` 0.8 (+ `axum-extra` for cookies)
- **Database:** `sqlx` 0.9 with **both SQLite and PostgreSQL** support (one codebase, DB-agnostic SQL — TEXT storage for portability)
- **Templating:** `askama` 0.16 (server-side rendered) + vanilla JS (no frontend build step)
- **Password hashing:** `argon2`
- **UUID:** `uuid` crate, **v4** for ping keys
- **Testing:** `axum-test` for integration, plus unit tests

## 3. Architecture

Single Rust binary with three logical surfaces plus shared modules.

```
┌─────────────────────────────────────────────┐
│ pingward (single binary)                      │
│                                               │
│  ① Ping API (public, keyed by check UUID)     │
│     /ping/<uuid>[/fail|/start|/log|/<code>]   │
│                                               │
│  ② Web UI + management API (authenticated)    │
│     dashboard / CRUD projects,checks,channels │
│     askama templates + vanilla JS             │
│                                               │
│  ③ Background scan loop (tokio task)          │
│     periodically scans active checks →        │
│     overdue → transition to down → notify     │
└─────────────────────────────────────────────┘
                    │
              sqlx (sqlite / postgres)
```

### Module boundaries

- `ping` — handle inbound check-in requests, record pings, drive state transitions on success/fail.
- `web` — UI pages and management endpoints (projects, checks, channels, settings).
- `auth` — local username/password sessions + forward-auth header trust.
- `scheduler` — the scan loop, due-time computation (period and cron), overdue detection.
- `notify` — `Notifier` trait and per-channel implementations, delivery with retry.
- `store` — sqlx data-access layer, DB-agnostic queries.
- `config` — cascade resolution of settings (env → global → project → check).

Each module has a single purpose and a well-defined interface; the scan loop and notifiers are decoupled behind traits so they can be tested with mocks (no real network / no real clock dependence in unit tests).

## 4. Data Model

Entity hierarchy: **User → Project → Check**. Channels belong to a Project; checks bind to channels many-to-many.

```
users        (id, username, password_hash?, is_admin, created_at)
projects     (id, user_id, name, scan_interval_secs?, created_at)
checks       (id, project_id, name, ping_uuid,
              schedule_kind[period|cron],
              period_secs?, grace_secs, cron_expr?, timezone,
              status[new|up|down|paused],
              last_ping_at?, last_start_at?, next_due_at?,
              scan_interval_secs?, created_at)
channels     (id, project_id, kind[webhook|telegram|slack|ntfy],
              name, config_json, created_at)
check_channels (check_id, channel_id)          -- many-to-many
pings        (id, check_id, kind[success|fail|start|log|exitcode],
              exit_code?, body, source_ip, created_at)
notifications(id, check_id, channel_id, event[down|up],
              status[ok|error], error?, created_at)
sessions     (id, user_id, expires_at)          -- cookie session store
settings     (key, value)                        -- global instance settings (admin-editable)
```

Notes:

- **Primary keys:** BIGINT autoincrement (simple and portable across SQLite/Postgres; no need for time-ordered UUID locality).
- **`ping_uuid`:** UUIDv4, stored as hyphenated TEXT, `UNIQUE` index for O(1) lookup. This is a capability-URL secret — see §5. Regenerable from the UI (invalidates the old URL).
- **`password_hash`:** nullable — forward-auth-only users have no local password.
- **`config_json`:** channel-specific settings (webhook URL, Telegram bot token + chat id, Slack webhook URL, ntfy server + topic + token).

## 5. Ping Protocol (public API)

Each check has a secret `ping_uuid` in its URL. Both `GET` and `POST` are accepted (convenience for `curl`). The request body (length-capped, e.g. 10 KB) is stored as that ping's `body`/log; oversized bodies are truncated, not rejected.

| Endpoint | Meaning | Effect |
|---|---|---|
| `GET/POST /ping/<uuid>` | success | Record check-in, reset the due clock; if state was `down`, fire recovery notification |
| `/ping/<uuid>/fail` | failure | Immediately transition to `down`, fire notification |
| `/ping/<uuid>/start` | job started | Record `last_start_at` (for run-duration); does **not** reset the due clock |
| `/ping/<uuid>/log` | log only | Store in `pings`; no state change |
| `/ping/<uuid>/<code>` | exit code | `0` → treated as success; non-zero → treated as fail |

- Unknown `uuid` → `404` (this also avoids leaking existence, since valid-but-wrong and nonexistent both return 404).
- Rapid duplicate pings are accepted idempotently.

### UUID rationale (v4, not v7)

`ping_uuid` is a bearer-style capability secret; the only property that matters is **unguessability**. v4 gives 122 bits of CSPRNG randomness. v7's only advantage is time-ordered index locality, which is irrelevant for an exact-match `UNIQUE` lookup, while its downsides (74 random bits, leaked creation time, time-sortable enumeration) are pure cost here. So v4.

## 6. Timeout Detection & Notification

### Check state machine

```
new ──success──▶ up ──(overdue OR fail ping)──▶ down
                 ▲                                │
                 └──────────── success ───────────┘   (down→up fires recovery)
paused: excluded from scanning; no overdue evaluation
```

### Due-time computation (scheduler core)

- **period mode:** `due = last_ping_at + period_secs + grace_secs`
- **cron mode:** `due = (next cron trigger after last_ping_at) + grace_secs`, computed in the check's `timezone`
- **first run** (`last_ping_at` is null): a grace window starts from creation time.

### Scan loop (approach: single polling loop)

A single background `tokio` task. It wakes at the frequency of the **smallest effective scan interval** across all active checks; on each wake it re-evaluates only the checks whose own interval has elapsed since their last evaluation. For each due check, if `now > due` and status is `up`/`new`, it transitions to `down` and fires notifications.

- Detection latency is bounded by one scan interval — acceptable for minute-scale cron monitoring.
- Per-check faults are isolated: an error evaluating one check does not abort the scan round.
- The loop is stateless across restarts; state is recomputed from the DB, so restarts recover automatically.

Rejected alternatives: per-check `tokio` timers (complex restart/race handling, hard to test) and external-cron-driven scanning (extra deployment dependency).

### Notification triggers (v1: state transitions only)

- `up`/`new` → `down`: send "down" notification to all channels bound to the check.
- `down` → `up`: send "recovery" notification.
- No repeated nag notifications in v1 (listed as future work).

### Delivery

Each attempt is recorded in `notifications`. Failed deliveries retry with exponential backoff (e.g. up to 3 attempts); both success and failure are persisted for display. Delivery is decoupled from state transitions — a failing channel never blocks or reverts a state change.

## 7. Authentication

- **Local username/password:** `argon2`-hashed, cookie-based sessions (`sessions` table + `axum-extra` cookies, mirroring `rdrs`).
- **Forward-auth:** trust an identity header injected by a trusted reverse proxy (e.g. Authelia / Authentik / oauth2-proxy). The header name and the set of trusted proxy sources are configurable. A request bearing a valid header maps to (or auto-provisions) the corresponding user.
- **Security boundary:** forward-auth is honored only when the request originates from a configured trusted proxy source, preventing header spoofing / privilege escalation.
- **Ping API:** unauthenticated — the `ping_uuid` is the credential.

## 8. Configuration (cascade)

Resolution order for an effective setting, from most specific to least (first defined wins):

```
check.scan_interval_secs  →  project.scan_interval_secs  →  global (DB settings)  →  env PINGWARD_SCAN_INTERVAL (default 30s)
```

- **Scan interval:** default `30s`, overridable at check / project / global / env level.
- **Environment variables:** `DATABASE_URL`, `PINGWARD_BIND`, `PINGWARD_BASE_URL` (for rendering ping URLs), `PINGWARD_SCAN_INTERVAL`, `PINGWARD_FORWARD_AUTH_HEADER`, `PINGWARD_TRUSTED_PROXIES`.
- **Global defaults** live in the DB `settings` table and are editable in the UI by an `is_admin` user.

## 9. Error Handling

- **Ping:** unknown uuid → 404; oversized body → truncate-and-store; rapid duplicates → idempotent accept.
- **Notification delivery failure:** retry + record; never affects state transitions.
- **DB errors:** 5xx + structured log.
- **Scan loop:** a single check's error is isolated and does not abort the round.
- **Config errors** (e.g. malformed `cron_expr`): validated at check-creation time and rejected up front.

## 10. Testing Strategy

- **Unit:** due-time computation (period and cron next-due, boundary cases, timezones); state-machine transitions; cascade resolution.
- **Integration** (`axum-test`, mirroring `rdrs`): hit each ping endpoint → assert DB state and ping records; drive the scan loop to overdue → assert transition to `down` and that the notifier was invoked.
- **Notifier:** mock HTTP server to verify webhook / Telegram / Slack / ntfy payload format and retry behavior.
- **Injection:** `Notifier` trait is mocked so state-machine tests never make real network calls.

## 11. Notification Channels (v1)

Pluggable behind a `Notifier` trait (`async fn send(&self, event: &NotificationEvent) -> Result<()>`). Channel type + settings stored as `(kind, config_json)`; dispatched via an enum. v1 channels:

- **Webhook** — HTTP POST to a user-supplied URL.
- **Telegram** — via bot token + chat id.
- **Slack** — incoming webhook URL.
- **ntfy** — ntfy.sh or self-hosted server (topic + optional token).

Adding **email** later requires only a new `EmailNotifier` impl + enum variant + a UI form option; core scheduling/state logic is untouched.

## 12. Future Work (explicitly deferred)

- Email notification channel.
- Repeated nag re-notifications with configurable interval while down.
- "Started but never finished" detection using `start` pings + max-runtime.
- Retention / pruning policy for pings and notifications.
- API tokens for programmatic management (beyond the ping capability URLs).
