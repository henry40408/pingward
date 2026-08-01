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
--dirty`), rendered in the global footer. Releases are cut with `gh release
create`, so the **git tag is the source of truth and `Cargo.toml`'s `version` is
never bumped** — before the first tag, or from a shallow CI checkout, the string
is a bare short SHA. An explicit `GIT_VERSION` env var overrides the describe
call; the release image needs that because `.dockerignore` excludes `.git`, so
`docker.yml` resolves the version on the runner and passes it as a
`--build-arg`.

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
- `cd e2e && npm test` — runs `bddgen` then `playwright test`. A `global-setup`
  runs `cargo build` first; each scenario spawns a **fresh binary + temp SQLite
  DB** on a random port. Selectors use `data-testid`.
- Single feature/scenario: `cd e2e && npx bddgen && npx playwright test ping_kinds -g "POST body"`.

README assets (Playwright's Chromium, no extra deps; both commit their output):
- `cd e2e && npm run screenshots` — rebuilds `docs/screenshots/*.png` by seeding
  backdated demo data against a *stopped* throwaway DB, then rebooting.
  `screenshots/seed.mjs` must keep every seeded check's timestamps inside its
  schedule budget — the boot `scan_once` would otherwise rewrite the status the
  shot is meant to show (cron checks anchor on a real fire time; see the cron
  helper there).
  **Decide whether a change makes these stale, and say so without being
  asked.** They are committed artefacts of the UI, so nothing fails when they
  drift — the README simply keeps advertising a version of the app that no
  longer exists, and the drift is only ever caught by eye. Any change to a
  template, to `assets/app.css`, or to rendered copy is a candidate: open the
  affected PNG, compare, and either regenerate or state that the surface is not
  in frame. The version stamp in the footer is part of the shot, so build from
  a clean tree (or pass `GIT_VERSION` explicitly) — `build.rs` will happily
  reuse a stale `-dirty` stamp naming a commit a rebase has since replaced.
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

**Response headers**: `web::content_security_policy` is layered on the `web`
router (like `no_store`), `web::security_headers` and `web::hsts` app-wide
(nosniff, `X-Frame-Options`, `Referrer-Policy`, `Permissions-Policy`). The CSP
says `script-src 'self'` with **no `'unsafe-inline'` and no nonce**, which
holds only because every script is a file under `/assets` (`assets/app.js` +
the render-blocking `assets/theme-init.js`) and **no template carries an
`onclick=`/`onsubmit=` attribute** — row navigation, confirm prompts and the
non-submitting filter forms are `data-href`/`data-confirm`/`data-nosubmit`
handled by delegation in `app.js`, which also survives a fragment swap. Adding
an inline handler means weakening the policy for the whole UI; add the
behaviour to `app.js` instead. `style-src` keeps `'unsafe-inline'` for the
heartbeat bars' computed `style="height:Npx"`. `/api/docs` is deliberately
outside the CSP — Scalar loads its bundle from a CDN.

**Session & CSRF secret** (`src/secret.rs`): one process secret
(`PINGWARD_SECRET`) keys every browser credential, domain-separated —
`cookie = <session_id>.HMAC(secret, "session:" ++ id)`,
`csrf = HMAC(secret, "csrf:" ++ id)`, and the one-shot flash cookie
`<payload>.HMAC(secret, "flash:" ++ payload)` (so a value planted by a sibling
subdomain never renders). The prefixes are load-bearing: without them the
session signature and the CSRF token are the same value, and every rendered
form would print the cookie's signature. Because the CSRF token is *derived*,
`sessions` has no `csrf_token` column, neither rendering nor checking one costs
a query, and a session id needs no row behind it to carry a valid token — which
is what lets `web::anonymous_session` hand every logged-out visitor a signed
cookie without an insert, and in turn why `csrf_guard` has **no path
exemptions**: `/login` and `/setup` are protected like everything else. **The
cookie value is not the session id** — never use `cookie.value()` as one; go
through `secret::session_id_from_jar`, which verifies the signature before any
DB work. Rotating the secret ends every browser session without touching the
rows; with `PINGWARD_SECRET` unset a random secret is generated per process, so
**every restart signs everyone out** — startup warns about exactly that. API
keys are unaffected (`src/apikey.rs`, SHA-256 of a random bearer token).

**Background loops** (`src/main.rs` spawns two tokio tasks, after building
`AppState` so both loops and the HTTP server share `state.events`):
- `scheduler::run_scan_loop` — periodically re-evaluates every check's
  `due_time`, transitions overdue checks to down, and fires notifications.
- `prune::run_prune_loop` — deletes old pings/notifications and expired sessions.

**Graceful shutdown** (`src/shutdown.rs`): one `watch<bool>` flag behind a
`(ShutdownTx, Shutdown)` pair, raised by `os_signal()` on the first
SIGTERM/SIGINT and shared by the HTTP server and both loops. The signal handler
is **mandatory, not polite**: the image's exec-form `ENTRYPOINT` makes pingward
PID 1, and Linux discards any signal still at its default disposition for PID 1
— with no handler, `docker compose down` sits out its whole 10s grace period
before SIGKILL. `main` drains in order: `with_graceful_shutdown` → each loop
returns from the `select!` at its sleep (an in-flight pass finishes) → **join**
both handles, so no loop query is outstanding → `store.pool.close()` bounded by
`POOL_CLOSE_TIMEOUT` (5s; fire-and-forget `deliver_event` tasks can still hold
a connection). That last step is the SQLite payoff — a clean close of the last
connection checkpoints the WAL and removes the `-wal`/`-shm` sidecars, which
SIGKILL never did. Adding a param to either loop means updating `main.rs` and
`tests/scheduler.rs` together.

**Live-tail signal bus**: `AppState::events` (`broadcast::Sender<i64>`) carries
a `check_id` whenever that check changes — published by `ping::apply` (every
ping kind, even `Log`/paused checks) and `scheduler::run_scan_loop` (each
`Down` transition), both gated on `receiver_count() > 0` so it's free when
unwatched. `web::sse_for_check` turns it into an SSE stream carrying **no
data** — the browser re-fetches the existing pings fragment on each `"changed"`
event, keeping rendering/auth in one place. In-process only: not shared across
replicas (see ARCHITECTURE.md). On the check page it is opt-in behind a LIVE
toggle, since an always-open connection per tab would eat into the browser's
per-origin HTTP/1.1 connection budget; each event debounces ~500ms, and the
pager and filter form are hidden while live (`assets/app.css` `.live-on`).

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
- Session cookie (`session_cookie_name(cookie_secure)` — plain
  `pingward_session`, or `__Host-pingward_session` when
  `PINGWARD_COOKIE_SECURE` is on) + argon2 password hashing. An optional
  trusted forward-auth header auto-provisions a passwordless non-admin user.
- Request extractors: `CurrentUser` (401/redirect if none), `OptionalUser`,
  `AdminUser` (403 if not admin).
- `auth::validate_password` is the single password policy, called by all four
  surfaces that **set** one (`setup_submit`, `account_password`,
  `users_create`, `users_set_password`) — length only
  (`MIN_PASSWORD_CHARS` 15, `MAX_PASSWORD_CHARS` 128, counted in *characters*),
  no composition rules, no trimming, and over-long is a rejection rather than a
  truncation. **`/login` never validates** and must not start to: the floor
  would lock out every credential predating the policy. A new
  password-setting surface must call it; that is the seam a breached-password
  check would be added at (deliberately absent — see ARCHITECTURE.md).
- `login_submit` calls `auth::verify_password_or_dummy`, never a bare
  `verify_password`: an unknown username (or a passwordless forward-auth
  account) must still pay for one argon2 verification, or the *response time*
  discloses which usernames exist and the generic error message buys nothing.
- `POST /login` is guarded by **two** `ratelimit::RateLimiter`s (generic over
  their key so both share one implementation): `login_limiter` is 5 per client
  IP per minute, `account_limiter` is 10 per **account** per 15 minutes — a
  per-address counter cannot see a distributed attack, which just buys `5 × N`
  guesses. The account one is keyed on the *submitted* username **before** the
  lookup, so an invented username throttles identically (otherwise being
  throttled is a username oracle), and a success `clear`s its bucket rather
  than `release`-ing one attempt. An account lockout is inherently a DoS
  primitive; that is accepted and documented, not solved.
- `web::reauthenticate` is the shared gate demanding the signed-in user's own
  password again before a sensitive action; `POST /account/password`,
  `POST /account/api-keys` and `POST /admin/unlock` use it. The API-key one carries its own weight: a
  key is bound by neither session cap and survives `users_set_password`, so a
  borrowed browser would otherwise buy permanent access. A **passwordless
  forward-auth account passes unchallenged** (nothing to verify; its authority
  is at the gateway) and is not rendered the field — `has_password` gates both
  that and the password card. Attempts go through `account_limiter`, which is
  what closed `/account/password`'s previously unmetered password oracle.
- `/admin`'s controls are single-button inline forms (`users_toggle_admin` posts
  no body), so a per-action password field does not fit: re-auth is decoupled
  into `src/elevate.rs`, an in-memory per-session unlock
  (`POST /admin/unlock`, `ELEVATION_TTL_SECS` 15min, keyed by session **handle**,
  dropped on logout — no migration, since a restart just means unlocking again).
  The line is **granting vs removing access**: `users_create`,
  `users_set_password` and `users_toggle_admin` *when promoting* are gated;
  delete, disable and demote deliberately are **not** — an operator who thinks
  they are under attack must not have to find their password first. A refusal
  redirects to `GET /admin/unlock` — an interstitial **page**, not a field,
  because the requirement needs explaining (why a signed-in admin is asked
  again, what it covers, that it is the same password and **not** a second
  factor); `/admin` keeps only a one-line note linking there. The refused action
  is not replayed afterwards.
- Rejected attempts log to the `pingward::auth` target — `login.failed`
  (`reason` = `bad_credentials`/`account_disabled`/`rate_limited`/`account_locked`)
  and `reauth.failed` (`surface` = `password_change`/`api_key_create`). One
  event per layer, discriminated by a field rather than by name, so one alert
  rule catches "somebody is guessing a password". Nothing else observes a failed
  attempt, so this is the only spray signal an operator gets. Never log the
  submitted password, and route an attempted username through
  `auth::log_username` rendered with `Debug` (`?…`), which is what stops an
  embedded newline forging a log entry.
- Owner scoping goes through `owned_project` / `owned_check` in `web.rs`, which
  return **404 (not 403)** for another user's resource — existence is hidden.
- `/account` is the per-user account page (password, sessions, then API keys,
  stacked as ordinary cards — no tabs). `POST /account/password` is the only
  way a non-admin can rotate their own credential; it demands the current
  password, since a session cookie alone must not be enough to lock the owner
  out, and revokes every *other* session on success (API keys survive, as they
  do for the admin-driven `users_set_password`). A passwordless forward-auth
  account has no card and is refused with 403 — its credential lives at the
  gateway, and a local one would be a second way in that the gateway's
  sign-out could not end. Session expiry is two layers: `expires_at` is an
  idle window (`SESSION_IDLE_TTL_HOURS`, 72h) that slides forward on use only
  past the half-life of the window, so it writes far less often than
  `last_seen_at`; a separate absolute cap (`SESSION_ABSOLUTE_MAX_DAYS`, 30d
  from `created_at`) is enforced in Rust rather than SQL and never extends no
  matter how active the session is. Since `sessions.id` is the cookie's bearer
  secret, rows are identified in the UI/URLs by a SHA-256 handle
  (`apikey::hash_api_key`) rather than the id itself. A session's stored IP
  comes from `auth::client_ip`, which shares its trusted-proxy gate with
  `forward_auth_username` so an untrusted caller cannot spoof it.
- That gate is `auth::is_trusted_proxy`, and a `PINGWARD_TRUSTED_PROXIES`
  entry is a bare address **or a CIDR block** (`172.16.0.0/12`, `fd00::/8`) —
  a containerised reverse proxy draws its address from the bridge network's
  pool, so a pinned literal stops matching when the network is recreated.
  Comparison and storage are canonical (`IpAddr::to_canonical`), so an
  IPv4-mapped IPv6 peer matches an IPv4 entry; unparseable entries match
  nothing and DNS is never consulted.
- `ping::ClientIp` is the extractor that resolves it, so `/ping/*`
  (`pings.source_ip`) and the login/setup handlers share one rule instead of
  each handler deciding. `ConnectInfo` is only populated by
  `into_make_service_with_connect_info`, so under `axum-test` there is no peer
  at all — the trusted-proxy path is covered in `tests/ping_source_ip.rs`,
  which drives the router with `oneshot` and injects `ConnectInfo` itself.
- `/admin` is the single merged admin page (every handler guarded by
  `AdminUser`), stacked as ordinary cards — no tabs, no sub-nav, mirroring how
  `/account` merges its sections. Former `/settings` and `/users` POST routes
  live under `/admin/…` so path grouping matches permission grouping. The
  deeper per-project/per-check cross-user handlers **reuse the owner
  templates** via an `is_admin`/base-prefix flag, so `data-testid`s and most
  step definitions are shared with the owner flow.
  **Reads under `/admin` are not audited; disclosures and mutations are.**
  `web::audits_as_mutation` gates the three cross-user resolvers
  (`admin_project`/`admin_check`/`admin_channel`) on the request method —
  those resolvers are the choke point for reads *and* writes, so the gate has
  to live there or dropping the read audit would silently take every admin
  pause/resume/delete/regenerate with it. The one read that still audits is
  `POST /admin/checks/{id}/ping-url`: the ping URL is a bearer credential, so
  an admin looking at someone else's check sees a "Reveal ping URL" control
  instead of the URL, and asking writes `admin.ping_url_reveal`.
  `web::CheckPageViewer` carries that decision — it replaced a separate
  `admin: bool` so the route prefix and the URL's visibility cannot be passed
  contradicting each other; an admin viewing a check they own is not gated.
  The audit card's two filter selects are built from
  `Store::audit_filter_options` (`SELECT DISTINCT`) rather than a hardcoded
  list, so a new `record_audit` call site appears in them by itself.
  `audit_retention_days` prunes the table through the same
  `prune::PruneTable` cascade as pings/notifications, and like them **defaults
  to off** — a default that started deleting a compliance record on upgrade
  would be the wrong surprise. Because shortening that window is precisely how
  an admin would erase their own trail, `settings_save` records a
  `settings.update` entry naming the changed keys (a no-op save writes
  nothing); that is the only mitigation available, since anyone able to set an
  env var or reach the database could tamper regardless.
  An admin can never delete, disable, or demote their own account — the "All
  users" row renders those controls inert and the handlers refuse the same
  self-targeted request with a one-shot flash.

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
heartbeat strip tooltips).

**Dashboard** (`src/web.rs::dashboard`): renders one group per project.
Display **order is decided in the handler, not in SQL** — the `Store` list
queries stay in id order because the project page, the admin views and the API
share them. Projects sort by name case-insensitively; within a group, checks
sort by `last_activity_at` (`max(last_ping_at, last_start_at)`, so an in-flight
`Start` counts), never-pinged last. Both the text (`q`) and status filters run
in Rust over the loaded rows — `LIKE` is case-insensitive on SQLite but not on
Postgres, and `ILIKE` is untranslated by the `Any` driver.
Loads are **batched, not per-group** (`list_checks_for_projects`,
`list_recent_ping_summaries_for_checks`, `checks_with_channels`), so a request
is a fixed number of queries however many projects a user owns. The heartbeat
window is deliberately a **narrow projection** (`models::PingSummary`) rather
than whole ping rows: selecting `body` meant decoding every captured POST
output — up to `ping::MAX_BODY` (10 KiB) per row, 40 rows per check — only for
`view::heartbeat` to drop it. That was most of the dashboard's render time
(measured in #116, which records the before/after).

The **check page's** strip is sized differently from the dashboard's six bars:
the server renders more bars than fit (`web::HEARTBEAT_BARS`, over
`web::HEARTBEAT_WINDOW` rows) and `.beat` in `assets/app.css` clips the
overflow from the *left*, so the strip fills any viewport with the newest run
pinned right — the server never needs to know the viewport. Do not "optimise"
the window into a `kind IN ('success','fail')` query: `run_durations` pairs
each finish ping with the `start` before it, so dropping the starts flattens
every bar. Do not put a run count in the caption either; only the browser
knows how many bars are visible.

**Display status** (`src/view.rs::display_status`/`DisplayStatus`): a
display-only status layered on top of the stored `CheckStatus`
(`new`/`up`/`down`/`paused`) — `late` and `running` exist only here, so the
stored status keeps its narrower meaning. Precedence is `Paused > Down >
Running > Late > Up`: `Running` beats `Late` (a long-running job legitimately
drifts past its expected time) but is itself beaten by `Down`/`Paused`, so an
in-flight run never masks an alert. It is derived from `last_start_at >
last_ping_at`, relying on `Option`'s ordering (`Some(_) > None`, not
`None > None`) to cover "started and never finished" with no `is_some()`
check. `view::next_due` derives the header countdown from
`scheduler::due_time`, **not** the stored `checks.next_due_at` — that column
is only ever stamped by `ping::apply`, so it is NULL for a never-pinged check
and for one downed by a `fail` ping, while `due_time` is what `scan_once`
itself evaluates. The deadline includes grace, hence "due" not "expected".

**Notifications** (`src/notify.rs`): a `Notifier` trait with six implementations
(`webhook`, `telegram`, `slack`, `ntfy`, `pushover`, `email`/SMTP). `notifier_for`
builds one from a stored `Channel`; `deliver_event` applies a `RetryPolicy`.
Delivery is fire-and-forget (`tokio::spawn`) so a ping response is never blocked
on notification I/O. Instance SMTP is configured via env (`config::SmtpConfig`).
Check creation auto-binds every channel already configured on the check's
project (`Store::bind_all_project_channels`) — both `web::check_create_core`
and `api::v1::create_check` call it, so a check made through either surface
starts wired up instead of silently alerting nobody. Existing checks are
unaffected. A check that still ends up with zero bound channels gets a "no
channel" chip on the dashboard and project page
(`Store::checks_with_channels`).
Channels are editable on the web, admin and API surfaces under one rule: **a
blank submitted field keeps the stored value**, so a stored secret is never
re-rendered. `validate_channel_update(form, Option<&Channel>)` is the single
validator (`validate_channel` = the `None` case, so create and edit enforce the
same per-kind required fields); the template only sees `ChannelEditView`
(non-secret values + `has_*: bool` flags), which makes non-leakage a type-level
property. Webhook/Slack **URLs count as secrets** (they are capability tokens);
chat ids, ntfy server/topic and email recipients are pre-filled.
`ntfy_token_clear` is the one explicit-clear escape hatch, and **`kind` is
immutable**. See ARCHITECTURE.md's "Editing a channel without leaking its
secrets".
Message content is `notify::event_text`, deliberately capped at **four lines**
— headline, context, reason, link. Everything past the headline comes from
`notify::EventDetail`, whose fields are all `Option` so a failed lookup drops a
line rather than the notification (`EventDetail::default()` renders the bare
one-liner — that is the channel-test path). It is built **at the call site from
the pre-update check snapshot**, never re-read during delivery: an `Up` event
reports the ping *before* the recovery, which a re-read would have already
overwritten. `DownCause` is likewise the caller's to set —
`scan_once` knows `Overdue`/`Overrun`, `ping::apply` knows
`Failed { exit_code }`, and `nag_once` sets none, which is why a reminder says
"Last ping …" instead of claiming "No ping since …" for a check downed by a
`/fail` ping. Timestamps render in the *check's* timezone (`notify::fmt_at`)
with a `duration::fmt_duration` relative suffix — unless the instance-wide
`display_timezone` setting is set on `/admin`, which wins
(`EventDetail::with_display_timezone`). That setting exists because a
notification is the one surface with no browser to localise it; the web UI
renders every absolute time in the *viewer's* zone via the `.localtime[data-ts]`
spans. A check's own `timezone` is otherwise used only to pick the wall clock a
**cron** schedule fires on (period schedules ignore it), and it is validated on
save (`web::validate_timezone`) — an unparseable name used to be stored
verbatim and then silently ignored, leaving the schedule on UTC. Webhook
payload additions are strictly additive — its original
`check`/`event`/`at`/`project_id` keys are untouched. See ARCHITECTURE.md's
"What a notification says".

**Models** (`src/models.rs`): string-backed enums (`CheckStatus`, `PingKind`,
`ScheduleKind`, `ChannelKind`, …) are generated by the `str_enum!` macro, which
also derives `as_str()` / `FromStr` — add variants there.

## Config (env vars, `src/config.rs`)

`DATABASE_URL` (default `sqlite://pingward.sqlite3?mode=rwc`), `PINGWARD_BIND`,
`PINGWARD_BASE_URL` (used to render ping URLs, and the check link in every
notification), `PINGWARD_SCAN_INTERVAL`,
`PINGWARD_PRUNE_INTERVAL_SECS`, `PINGWARD_LOG_FORMAT` (`text` default, or `json`
for line-delimited structured logs — parsed into `config::LogFormat`, applied by
`init_tracing` in `main.rs`), `PINGWARD_FORWARD_AUTH_HEADER` +
`PINGWARD_TRUSTED_PROXIES`, `PINGWARD_FORWARD_AUTH_LOGOUT_URL` (the gateway's
sign-out endpoint; unset, a forward-auth logout lands on the dashboard with a
flash saying only the proxy can end the session, since a local logout would
just be re-authenticated — see ARCHITECTURE.md's "Session layers"),
`PINGWARD_SECRET` (session/CSRF signing key, ≥16 bytes; generated per process
when unset — see above), `PINGWARD_COOKIE_SECURE` (whether the session/flash
cookie carries `Secure`; default derived from whether `PINGWARD_BASE_URL`
starts with `https://`), `PINGWARD_HSTS_MAX_AGE` (off by default since pingward
does not terminate TLS itself), and `PINGWARD_SMTP_*` (host/from required to
enable email; port/TLS defaulted). The scan and prune interval env vars accept
raw seconds or a human-readable duration; an unparseable value falls back to
the default rather than failing at boot. `Config::from_map` is the testable
core — unit-test config parsing through it rather than real env. These env vars
are also surfaced read-only on `/admin`'s "Environment" card, with secrets
shown only as configured/not-set, never their value.
