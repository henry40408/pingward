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

`build.rs` stamps the binary with `GIT_VERSION` (`git describe --tags --always
--dirty`), read back by `view::version()` and rendered in the global footer
(`templates/base.html`, outside the `show_nav` guard so signed-out pages carry
it too). Releases are cut with `gh release create`, so the **git tag is the
source of truth and `Cargo.toml`'s `version` is never bumped** — before the
first tag, or from a shallow CI checkout, the string is a bare short SHA.
An explicit `GIT_VERSION` env var overrides the describe call; the release
image needs that because `.dockerignore` excludes `.git`, so `docker.yml`
resolves the version on the runner and passes it as a `--build-arg`.

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

README assets (Playwright's Chromium, no extra deps; both commit their output):
- `cd e2e && npm run screenshots` — rebuilds `docs/screenshots/*.png`. Wipes a
  throwaway DB, creates the admin via `POST /setup`, seeds backdated demo data
  with the `sqlite3` CLI against the *stopped* DB, reboots, and captures framed
  shots. `screenshots/seed.mjs` must keep every seeded check's timestamps inside
  its schedule budget — the boot `scan_once` would otherwise rewrite the status
  the shot is meant to show (cron checks anchor on a real fire time; see the
  cron helper there).
- `cd e2e && npm run icons` — re-renders `assets/apple-touch-icon.png` from
  `assets/favicon.svg`. Run after editing the SVG.

## Architecture

**Router composition** (`src/lib.rs::app`) merges three sibling routers under
one `AppState`:
- `web::routes()` — the browser UI, wrapped in three layers. Order is
  load-bearing and documented in `ARCHITECTURE.md`: `forward_auth_session` →
  `anonymous_session` → `csrf_guard` → handler.
- `ping::routes()` — machine `/ping/*` endpoints. Merged as a sibling, so they
  are **structurally exempt from CSRF** (public, unauthenticated).
- `assets::routes()` + `/healthz`.

**Session & CSRF secret** (`src/secret.rs`): one process secret
(`PINGWARD_SECRET`) keys both browser credentials, domain-separated —
`cookie = <session_id>.HMAC(secret, "session:" ++ id)` and
`csrf = HMAC(secret, "csrf:" ++ id)`. The prefixes are load-bearing: without
them the two values are equal and every rendered form would print the cookie's
signature. Because the CSRF token is *derived*, `sessions` has no `csrf_token`
column and neither rendering nor checking a token costs a query — and a session
id needs no row behind it to carry a valid token, which is what lets
`web::anonymous_session` hand every logged-out visitor a signed cookie without
a single insert. That in turn is why `csrf_guard` has **no path exemptions**:
`/login` and `/setup` are protected like everything else. **The cookie
value is not the session id** — never use `cookie.value()` as one; go through
`secret::session_id_from_jar`, which verifies the signature before any DB work
(`auth::resolve_user`, `web::csrf_guard`, `logout`, and the `/account` session
list/revoke handlers all do). Rotating the secret ends every browser session
without touching the rows; with `PINGWARD_SECRET` unset a random secret is
generated per process, so **every restart signs everyone out** — startup warns
about exactly that. API keys are unaffected (`src/apikey.rs`, SHA-256 of a
random bearer token, no secret involved).

**Background loops** (`src/main.rs` spawns two tokio tasks, after building
`AppState` so both loops and the HTTP server share `state.events`):
- `scheduler::run_scan_loop` — periodically re-evaluates every check's
  `due_time`, transitions overdue checks to down, and fires notifications.
- `prune::run_prune_loop` — deletes old pings/notifications and expired sessions.

**Graceful shutdown** (`src/shutdown.rs`): one `watch<bool>` flag behind a
`(ShutdownTx, Shutdown)` pair, raised by `os_signal()` on the first
SIGTERM/SIGINT and shared by the HTTP server and both loops (dropping the
`ShutdownTx` also counts as a request). The signal handler is **mandatory, not
polite**: the image's exec-form `ENTRYPOINT` makes pingward PID 1, and Linux
discards any signal still at its default disposition for PID 1 — with no
handler, `docker compose down` sits out its whole 10s grace period before
SIGKILL. `main` drains in order: `with_graceful_shutdown` → each loop returns
from the `select!` at its sleep (an in-flight pass finishes) → **join** both
handles, so no loop query is outstanding → `store.pool.close()` bounded by
`POOL_CLOSE_TIMEOUT` (5s; fire-and-forget `deliver_event` tasks can still hold
a connection). That last step is the SQLite payoff — a clean close of the last
connection checkpoints the WAL and removes the `-wal`/`-shm` sidecars, which
SIGKILL never did. Adding a param to either loop means updating `main.rs` and
`tests/scheduler.rs` together.

**Live-tail signal bus**: `AppState::events` (`broadcast::Sender<i64>`) carries
a `check_id` whenever that check changes — published by `ping::apply` (every
ping kind, even `Log`/paused checks) and `scheduler::run_scan_loop` (each
`Down` transition), both gated on `receiver_count() > 0` so it's free when
unwatched. `GET /checks/{id}/events` / `/admin/checks/{id}/events`
(`web::sse_for_check`) turn it into an SSE stream carrying no data — the
browser re-fetches the existing `/checks/{id}/pings` fragment on each
`"changed"` event, keeping rendering/auth in one place. In-process only: not
shared across replicas (see ARCHITECTURE.md). On the check page this is
opt-in: an id="pings-live" LIVE toggle (`templates/check.html`) opens the
EventSource, since an always-open connection per tab would eat into the
browser's per-origin HTTP/1.1 connection budget; each event debounces ~500ms
before re-fetching the fragment unfiltered/newest-page, with the pager and
filter form hidden while live (`assets/app.css` `.live-on`).

**Persistence** (`src/db.rs`, `src/store.rs`): one sqlx `AnyPool` that dispatches
to **SQLite or Postgres by URL scheme**. All queries go through `Store` and must
work on both backends — use `$N` placeholders + `RETURNING id` (the `Any` driver
does **not** translate `?`). Migrations are duplicated in `migrations/sqlite/`
and `migrations/postgres/`; `db::migrate` picks the migrator from the URL, so a
schema change means writing the SQL **in both**. Both directories are embedded
at compile time with `sqlx::migrate!` (hence sqlx's `macros` feature) — the
release image ships only the binary and runs from `/data`, so migrations must
never be read from the filesystem at startup. SQLite pragmas (foreign keys,
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
  caller cannot spoof it. Each row also shows an "SSO" badge when the session
  was minted by `forward_auth_session` rather than a password/setup login.
- That gate is `auth::is_trusted_proxy`, and a `PINGWARD_TRUSTED_PROXIES`
  entry is a bare address **or a CIDR block** (`172.16.0.0/12`, `fd00::/8`) —
  a containerised reverse proxy draws its address from the bridge network's
  pool, so a pinned literal stops matching when the network is recreated.
  Comparison and storage are canonical (`IpAddr::to_canonical`), so an
  IPv4-mapped IPv6 peer matches an IPv4 entry; unparseable entries match
  nothing and DNS is never consulted.
- `ping::ClientIp` is the extractor that resolves it, and it yields the
  finished `Option<String>` — so `/ping/*` (`pings.source_ip`, the "Source"
  column) and the login/setup handlers share one rule instead of each handler
  deciding. It requires `Arc<Config>: FromRef<S>`, and `ConnectInfo` is only
  populated by `into_make_service_with_connect_info`, so under `axum-test`
  there is no peer at all — the trusted-proxy path is covered in
  `tests/ping_source_ip.rs`, which drives the router with `oneshot` and
  injects `ConnectInfo` itself.
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

**Dashboard** (`src/web.rs::dashboard`): renders one group per project.
Display **order is decided in the handler, not in SQL** — the `Store` list
queries stay in id order because the project page, the admin views and the API
share them. Projects sort by name (case-insensitively, `sort_projects_by_name`);
within a group, checks sort by `last_activity_at` — `max(last_ping_at,
last_start_at)`, so an in-flight `Start` counts — newest first, never-pinged
last, ties broken by id. Both the text (`q`) and status filters run in Rust over
the loaded rows (see `matches_term`), and filtering preserves the sort order.
Loads are **batched, not per-group**: one `list_checks_for_projects` for every
project's checks, one `list_recent_pings_for_checks` for the heartbeat strips,
and one `checks_with_channels` to know which rows get the "no channel" chip
(§ Notifications), so a request is a fixed number of queries regardless of how
many projects or checks a user owns.

**Display status** (`src/view.rs::display_status`/`DisplayStatus`): a
display-only status layered on top of the stored `CheckStatus`
(`new`/`up`/`down`/`paused`) — `late` and `running` exist only here, so the
stored status keeps its narrower meaning. Precedence is `Paused > Down >
Running > Late > Up`. `Running` applies only when the stored status is `Up`
or `New`, via `check.last_start_at > check.last_ping_at` — `store::mark_ping`
`COALESCE`s both timestamps and `ping::apply` only stamps `last_start_at` for
a `Start` ping and `last_ping_at` for a success/fail, so a `Log` ping cannot
clear it, and `Option`'s ordering (`Some(_) > None`, not `None > None`) covers
"started and never finished" with no extra `is_some()` check. `Running` beats
`Late` (a long-running job legitimately drifts past its expected time) but is
itself beaten by `Down`/`Paused`, so an in-flight run never masks an alert.

**Notifications** (`src/notify.rs`): a `Notifier` trait with six implementations
(`webhook`, `telegram`, `slack`, `ntfy`, `pushover`, `email`/SMTP). `notifier_for`
builds one from a stored `Channel`; `deliver_event` applies a `RetryPolicy`.
Delivery is fire-and-forget (`tokio::spawn`) so a ping response is never blocked
on notification I/O. Instance SMTP is configured via env (`config::SmtpConfig`).
Check creation auto-binds every channel already configured on the check's
project (`Store::bind_all_project_channels`, a single `INSERT … SELECT`) —
both `web::check_create_core` and `api::v1::create_check` call it right after
`create_check`, so a check made through either the web UI or the REST API
starts wired to every existing channel instead of silently alerting nobody.
Existing checks are unaffected. A check that still ends up with zero bound
channels (e.g. its project had none at creation time) gets a "no channel" chip
on the dashboard and project page (`Store::checks_with_channels`), and the
project page's empty-channels state is a warning banner naming the
consequence rather than a neutral note.

**Models** (`src/models.rs`): string-backed enums (`CheckStatus`, `PingKind`,
`ScheduleKind`, `ChannelKind`, …) are generated by the `str_enum!` macro, which
also derives `as_str()` / `FromStr` — add variants there.

## Config (env vars, `src/config.rs`)

`DATABASE_URL` (default `sqlite://pingward.sqlite3?mode=rwc`), `PINGWARD_BIND`,
`PINGWARD_BASE_URL` (used to render ping URLs), `PINGWARD_SCAN_INTERVAL`,
`PINGWARD_PRUNE_INTERVAL_SECS`, `PINGWARD_LOG_FORMAT` (`text` default, or `json`
for line-delimited structured logs — parsed into `config::LogFormat`, applied by
`init_tracing` in `main.rs`), `PINGWARD_FORWARD_AUTH_HEADER` +
`PINGWARD_TRUSTED_PROXIES`, `PINGWARD_FORWARD_AUTH_LOGOUT_URL` (logout's
redirect target — the gateway's sign-out endpoint; unset, a forward-auth logout
instead lands on the dashboard with a one-shot flash telling the user to sign
out at their proxy, since a local logout would just be re-authenticated, while a
password logout still goes to `/login` — see ARCHITECTURE.md's "Session
layers"), `PINGWARD_SECRET` (session/CSRF signing key, ≥16
bytes; generated per process when unset — see above), and `PINGWARD_SMTP_*`
(host/from required to enable email; port/TLS defaulted). The scan and prune interval env vars accept raw
seconds or a human-readable duration (`5m`, `1h30m`) via
`duration::parse_duration`; an unparseable value falls back to the default
rather than failing at boot. `Config::from_map` is the testable core —
unit-test config parsing through it rather than real env. These env vars are
also surfaced read-only on `/admin`'s "Environment" card, with secrets (the
SMTP password) shown only as configured/not-set, never their value.
