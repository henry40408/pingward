# Architecture

This is the code map for contributors. It assumes you've read
[README.md](README.md) for install/config/API usage — this document does not
repeat that, it explains how the pieces fit together.

## Overview

pingward is a single `axum` process. It serves a server-rendered browser UI
(Askama templates, compiled into the binary), a set of machine `/ping/*`
endpoints that jobs call to report in, and a bearer-authenticated REST API
under `/api/v1` with an OpenAPI document and a Scalar reference UI. All three
surfaces share one `AppState` (a `Store` plus the parsed `Config`) and one
`sqlx::AnyPool` that talks to either SQLite or Postgres.

## Repository layout

| Path                  | Contents                                                          |
| ---------------------- | ------------------------------------------------------------------ |
| `src/`                 | The application: router composition, handlers, domain logic       |
| `src/api/`             | The `/api/v1` REST surface (DTOs, input parsing, extractors, v1 handlers) |
| `templates/`           | Askama HTML templates, compiled into the binary at build time     |
| `assets/`              | Static CSS, embedded fonts and the app icons, served by `src/assets.rs` |
| `migrations/sqlite/`   | SQLite schema migrations                                          |
| `migrations/postgres/` | The same migrations, hand-duplicated for Postgres syntax          |
| `tests/`                | Rust integration tests (one file per feature area), run with `cargo nextest run` |
| `e2e/`                 | Playwright + playwright-bdd browser tests (`.feature` + `.steps.js`) |

## Module map

- `src/lib.rs` — declares the crate's modules and `app()`, which composes the
  final `Router`.
- `src/main.rs` — the binary entry point: reads `Config`, sets up tracing,
  connects/migrates the database, spawns the two background loops, starts
  `axum::serve`, drains everything on SIGTERM/SIGINT (see *Graceful shutdown*),
  and installs **mimalloc** as the process-wide `#[global_allocator]`
  (binary only, not the library).
- `src/web.rs` — the browser-facing UI: `routes()`, every page/form handler,
  the `csrf_guard` middleware, and owner/admin scoping helpers
  (`owned_project`, `owned_check`, `admin_project`, `admin_check`).
- `src/ping.rs` — the machine `/ping/{uuid}[...]` endpoints (success, fail,
  start, log, exit-code) that jobs call to report in.
- `src/api/` — the REST API surface:
  - `mod.rs` — router (`routes()`) and the `OpenApi`/Scalar docs handlers.
  - `v1.rs` — the actual `/api/v1` handlers.
  - `dto.rs` — response shapes (`utoipa::ToSchema`).
  - `input.rs` — request bodies for create/update endpoints.
  - `extract.rs` — the `ApiUser` bearer-auth extractor.
  - `error.rs` — the API's JSON error type.
- `src/auth.rs` — session cookie constants, argon2 password hashing,
  forward-auth header resolution, client-IP resolution, and the
  `CurrentUser`/`OptionalUser`/`AdminUser` request extractors.
- `src/apikey.rs` — API key generation (`pw_...`) and SHA-256 hashing for the
  REST API's bearer tokens.
- `src/state.rs` — `AppState { store, config }`, `Clone` + `FromRef` so
  handlers can extract either piece independently.
- `src/store.rs` — `Store`, the single data-access layer; every query in the
  app goes through it.
- `src/db.rs` — `connect()` (builds the `AnyPool`, applies SQLite pragmas per
  connection) and `migrate()` (picks the embedded migration set by URL scheme).
- `src/models.rs` — domain structs (`Check`, `User`, `Project`, `Channel`,
  ...) and the `str_enum!`-generated string-backed enums
  (`CheckStatus`, `PingKind`, `ScheduleKind`, `ChannelKind`, `NotifyStatus`).
- `src/scheduler.rs` — `due_time`/`overrun_time` computation, `scan_once`
  (marks overdue/overrun checks down and emits events), `nag_once` (repeat
  reminders), and `run_scan_loop`, the background task `main.rs` spawns.
- `src/prune.rs` — `prune_once` (deletes old pings/notifications per
  retention setting, plus expired sessions) and `run_prune_loop`.
- `src/shutdown.rs` — the cooperative shutdown flag (`channel()` →
  `ShutdownTx`/`Shutdown`) and `os_signal()`, the SIGTERM/SIGINT listener that
  raises it.
- `src/notify.rs` — the `Notifier` trait, its six implementations (webhook,
  Telegram, Slack, ntfy, Pushover, email/SMTP), `notifier_for` (builds one
  from a stored `Channel`), and `deliver_event` (fans an event out to a
  check's bound channels under a `RetryPolicy`).
- `src/config.rs` — `Config` (parsed once from env via `Config::from_env`,
  testable through `Config::from_map`), `SmtpConfig`, and the
  `effective_scan_interval`/`effective_nag_interval` cascade resolvers.
- `src/duration.rs` — `parse_duration`/`fmt_duration`, the human-readable
  (`5m`, `1h30m`, `2d`) duration parser/formatter used by form fields and
  duration env vars.
- `src/view.rs` — presentation helpers shared by templates, including the
  lossy `fmt_secs` display formatter (distinct from `duration::fmt_duration`,
  which round-trips), and `display_status`/`DisplayStatus`, the display-only
  status derived from a `Check` (`new`/`up`/`running`/`late`/`down`/`paused`)
  — `late` and `running` have no `CheckStatus` counterpart, so the stored
  status keeps its narrower up/down/new/paused meaning. Precedence is
  `Paused > Down > Running > Late > Up`: `Running` (a stored `up` or `new`
  check with `last_start_at` newer than `last_ping_at`, i.e. a `start` ping
  not yet followed by a finish) beats `Late` because a long-running job
  naturally drifts past its expected time while legitimately still
  executing, and is itself beaten by `Down`/`Paused` so an in-flight run
  never masks an alert.
- `src/assets.rs` — serves `assets/app.css`, the embedded webfonts, and the
  app icons (`/favicon.svg`, `/apple-touch-icon.png`), each content-addressed
  by a hash of what it serves.
- `src/error.rs` — `AppError`, the app-wide error type implementing
  `IntoResponse`.

## Request lifecycle / router composition

`lib.rs::app()` builds one `Router` by merging four sibling routers, then
attaches `AppState`:

```rust
Router::new()
    .route("/healthz", get(|| async { "ok" }))
    .merge(web::routes().layer(csrf_guard middleware))
    .merge(ping::routes())
    .merge(api::routes())
    .merge(assets::routes())
    .with_state(state)
```

Only `web::routes()` is wrapped in `web::csrf_guard`. Because the other
routers are merged as *siblings* rather than nested under it, `/ping/*`
(machine endpoints, no session), `/api/v1/*` (bearer-authenticated, never
reads the session cookie), and the static asset/`/healthz` routes are
**structurally** exempt from CSRF — there's no way for a change inside
`web::routes()` to accidentally start covering them. `csrf_guard` itself
lets safe methods (GET/HEAD/OPTIONS) and the pre-session `/login`/`/setup`
paths through, and otherwise requires a per-session synchronizer token sent
as `X-CSRF-Token` (or hidden form field) matching the one stored on the
session.

`/api/v1` data endpoints authenticate independently via the `ApiUser` bearer
extractor; `/api/docs` and `/api/openapi.json` additionally accept a logged-in
web session (`CurrentUser`) but are read-only `GET`s, so they add no
CSRF-relevant ambient authority.

## Persistence

One `sqlx::AnyPool` (`src/db.rs::connect`) dispatches to SQLite or Postgres
based on the `DATABASE_URL` scheme. Every query in the app goes through
`Store` (`src/store.rs`) — there's no direct `sqlx` access from handlers.

Because the `Any` driver does **not** translate `?` placeholders, every query
must use `$N` placeholders and `RETURNING id` (not `?` + `last_insert_id`).
This applies uniformly across both backends when going through `Any`.

Migrations live in `migrations/sqlite/` and `migrations/postgres/` and are
**hand-duplicated** — `db::migrate` just picks the migrator matching the
URL scheme and runs it. A schema change means writing the SQL twice, once
per dialect.

Both directories are embedded into the binary at compile time via
`sqlx::migrate!` (one `static Migrator` each), so nothing is read from disk at
startup. That is what makes the release image work: it ships the binary alone,
with no source tree, and runs from the mounted `/data` volume — a migrator
that resolved `migrations/` relative to the working directory would panic
there.

A page that renders a list of lists must **batch its child loads** rather than
querying once per parent. `Store` exposes a batched sibling next to the
per-parent query for each such case — `list_checks_for_projects` beside
`list_checks_for_project`, `list_recent_pings_for_checks` beside
`list_recent_pings` — each building an `IN ($1,…,$N)` list and returning a
`HashMap` keyed by the parent id (parents with no children are simply absent).
The dashboard uses both, plus `checks_with_channels` (same `IN ($1,…,$N)`
shape, but returning a flat `HashSet<i64>` of the check ids that have at least
one bound channel rather than a per-parent map, since the caller only needs
membership) to decide which rows get the "no channel" chip — so its query
count is fixed no matter how many projects or checks a user owns; without
these batched queries it would issue one query per project group and one (or
more) per check row.

`db::connect` applies SQLite-only pragmas per new connection: `foreign_keys`
(so `ON DELETE CASCADE` is enforced — Postgres does this natively), a
`busy_timeout` of 5s, and, for on-disk (non-`:memory:`) databases, WAL
journaling with `synchronous = NORMAL`. In-memory SQLite is capped to a
single pool connection since `:memory:` is scoped to one physical connection.

## Auth & authorization

Sessions are a `pingward_session` cookie plus an argon2 password hash
(`src/auth.rs`). An optional trusted forward-auth header
(`PINGWARD_FORWARD_AUTH_HEADER` + `PINGWARD_TRUSTED_PROXIES`) can
auto-provision a passwordless, non-admin user on first sight, but only when
the request's peer IP is a configured trusted proxy.

Three request extractors resolve the caller:

- `CurrentUser` — 401/redirects to `/login` if no session/forward-auth user.
- `OptionalUser` — same resolution, but yields `None` instead of redirecting
  (used where a handler needs to branch on "no user" itself).
- `AdminUser` — wraps `CurrentUser`, additionally requiring `is_admin`;
  otherwise 403s.

Owner scoping for the per-user browser routes goes through `owned_project`
and `owned_check` in `web.rs`, which return `AppError::NotFound` (**404, not
403**) when the resource belongs to a different user — this hides whether
the resource exists at all from a caller who doesn't own it.

`/admin*` routes have **no router-level guard layer** — every entry
registered in `web::routes()` is individually guarded by extracting
`AdminUser` as one of its parameters (before `Form`/`HtmlForm`, so the guard
rejects before the request body is even parsed), with **no exceptions**.
That makes it possible, in principle, for a newly added `/admin` route to
forget the guard. `tests/admin.rs::non_admin_forbidden_on_every_admin_route`
closes that gap by parsing `web::routes()`'s own source at test time to
derive the exact list of `/admin*` (method, path) pairs it registers, then
asserting every one of them returns 403 for a signed-in non-admin — so a
`/admin` route that forgets its `AdminUser` guard fails the suite, with no
table to silence it.

`/api/v1` has the identical shape: `api::routes()` has **no router-level
auth layer** either — every handler individually extracts `ApiUser`, the
bearer-token extractor (`src/api/extract.rs`).
`tests/api_v1.rs::every_api_v1_route_requires_a_bearer_key` enforces the
invariant the same source-parsing way, reusing
`tests/common::routes_in_router_source` against `src/api/mod.rs` instead of
`src/web.rs`. `/api/openapi.json` and `/api/docs` are session-gated
(`CurrentUser`) rather than bearer-gated and so sit outside this invariant —
the `/api/v1` prefix filter excludes them automatically.

Once past that auth check, owner scoping for `/api/v1` goes through
`resolve_project`/`resolve_check`/`resolve_channel` in `src/api/v1.rs`: owner
first, else an audited admin cross-user access, else `404` (not `403`) — the
same existence-hiding behaviour as the web UI's `owned_project`/`owned_check`.
`tests/api_v1.rs::member_cannot_reach_another_users_resource_on_any_api_route`
enforces this across every parameterised `/api/v1` route, derived the same
source-parsing way, by substituting another user's resource id and asserting
a non-admin caller gets `404`. Each route is checked both ways: the non-owner
gets `404`, and the owner, hitting the same route against the same id, gets
something other than `404` — a nonexistent id also 404s, so without that
owner half the test could pass vacuously even if ownership scoping were
broken.

The web UI's `owned_project`/`owned_check` (see above) are covered the same
exhaustive, two-sided way by
`tests/web_ownership.rs::member_cannot_reach_another_users_resource_on_any_web_route`,
derived from `web::routes()` instead of `api::routes()` and excluding
`/admin*` (its own exhaustive test, and admins are allowed cross-user access)
and `/account/*` (owner-scoped by a different mechanism entirely).

## Background loops

`main.rs` spawns two `tokio` tasks against the shared `Store`:

- `scheduler::run_scan_loop` — every `PINGWARD_SCAN_INTERVAL` (default 30s),
  scans active checks, transitions any overdue-or-overrun check to `down`,
  and fans out `NotificationEvent`s via `notify::deliver_event`.
- `prune::run_prune_loop` — every `PINGWARD_PRUNE_INTERVAL_SECS` (default
  1h), deletes pings/notifications past their retention window and any
  already-expired session rows.

Scan and nag (repeat-reminder) intervals resolve through a
check → project → global-setting → env-default cascade
(`config::effective_scan_interval` / `effective_nag_interval`): the most
specific non-positive-or-unset level falls through to the next. Nag has no
env default — it's off unless a level opts in.

## Graceful shutdown

`src/shutdown.rs` holds one `tokio::sync::watch<bool>` flag behind a
`(ShutdownTx, Shutdown)` pair. `main` hands a `Shutdown` clone to all three
long-lived tasks; a spawned listener raises the flag on the first
SIGTERM/SIGINT (`shutdown::os_signal`). Dropping the `ShutdownTx` also counts
as a request — a lost controller must not leave the loops running.

**Why a handler is mandatory, not polite.** The container image's
`ENTRYPOINT ["/pingward"]` is exec-form with no init shim, so pingward is
**PID 1**, and Linux discards any signal whose disposition is still the default
for PID 1. With no handler installed, SIGTERM is silently ignored: `docker
stop` / `docker compose down` waits out its full 10s grace period and then
SIGKILLs.

The drain runs in a fixed order, because each step depends on the previous one:

1. `axum::serve(...).with_graceful_shutdown(...)` stops accepting connections
   and lets in-flight requests finish. An open SSE stream
   (`web::sse_for_check`) only ends when the client disconnects, so step 4's
   timeout — not this step — bounds the wait.
2. Both loops return from their `tokio::select!` at the sleep, so a scan or
   prune pass already in flight completes instead of being abandoned.
3. `main` **joins** those two `JoinHandle`s. Returning rather than being
   aborted is the point: it guarantees no loop query is outstanding when the
   pool closes, which would otherwise fail with `PoolClosed`.
4. `store.pool.close()`, bounded by `POOL_CLOSE_TIMEOUT` (5s, well inside
   Docker's 10s grace). `close()` waits for every connection to be returned,
   including ones held by fire-and-forget `deliver_event` tasks; the timeout
   keeps a stuck notification retry from turning a graceful stop into a hang.

Step 4 is what matters for SQLite: a clean close of the **last** connection
checkpoints the WAL into the main database file and removes the `-wal`/`-shm`
sidecars (asserted by `db::tests::closing_the_pool_checkpoints_and_removes_wal_sidecars`).
Under SIGKILL that never happened, so the sidecars survived and every start
had to replay the WAL.

## Live-tail signal bus (SSE)

`AppState::events` is a `tokio::sync::broadcast::Sender<i64>` (capacity 256,
built in `AppState::new` and shared via `FromRef`, alongside `Store` and
`Arc<Config>`) that carries a `check_id` whenever that check changes. Two
producers publish to it:

- `ping::apply` — after every successful `store.insert_ping(...)` (all five
  ping kinds, including `Log`, and regardless of the check's status —
  paused checks still record pings and still publish), before the
  paused-check early return.
- `scheduler::run_scan_loop` — for every `NotificationEvent` a scan pass
  produces (i.e. `Down` transitions), publishing `ev.check_id` alongside
  delivering the notification. `main.rs` builds `AppState` before spawning the
  background loops specifically so the scan loop and the HTTP server can share
  one sender (`state.events.clone()`).

Both producers gate on `events.receiver_count() > 0` first, so publishing
costs nothing when no browser tab has the check page open, and a `send` with
no subscribers is not treated as an error.

`GET /checks/{id}/events` (owner-scoped) and `GET /admin/checks/{id}/events`
(admin twin) subscribe and turn the broadcast into an SSE stream
(`web::sse_for_check`). The payload is deliberately just the string
`"changed"`, never ping data: on receipt, the browser re-fetches the
existing `/checks/{id}/pings` HTML fragment, so rendering, filtering, and
authorization stay in that one already-tested code path instead of being
duplicated over the wire. Ownership is checked (via `owned_check`/
`admin_check`) *before* the stream is constructed, so a non-owner gets the
usual 404 immediately rather than a stream that never resolves to anything.

On the check page, this is wired up behind an opt-in LIVE toggle on the
"Recent pings" card (`templates/check.html`) rather than an always-open
connection: a browser caps HTTP/1.1 connections per origin at roughly six, so
one EventSource per open check tab would starve the rest of the app. Clicking
LIVE opens the EventSource; each `"changed"` event debounces ~500ms (coalescing
bursts) before re-fetching the pings fragment with no query string — live mode
is defined as "newest page, unfiltered," so the pager and filter form are
hidden for the duration (`assets/app.css`, `.card.live-on`).

A lagged subscriber (its receiver fell behind the channel's 256-slot buffer)
is coalesced into one more `"changed"` event rather than dropped. This is a
deliberate divergence from the usual "skip what you missed" idiom for a log
tail: a dropped *signal* here would leave the page stale forever (there's no
later signal that says "you're behind, catch up"), whereas a spurious extra
refresh is harmless and self-corrects on the next fragment fetch.

**Known limitation:** the channel is in-process only. Run multiple pingward
replicas against a shared Postgres and a browser tab connected to replica A
never sees a ping delivered to replica B — SQLite has no `LISTEN/NOTIFY`
equivalent, so there's no backend-portable fix, and none is attempted; a
stale tab still catches up on its next manual reload or fragment poll.

## Notifications

`notify::Notifier` is a trait with six implementations: webhook, Telegram,
Slack, ntfy, Pushover, and email (SMTP). `notifier_for` builds the right one
from a stored `Channel`'s `kind` and `config_json`, logging and returning
`None` on invalid/missing config rather than failing the caller.
`deliver_event` resolves a check's bound channels and retries each delivery
under a `RetryPolicy` (3 attempts, exponential backoff from 500ms by
default). Delivery is fire-and-forget: `run_scan_loop` calls it inside
`tokio::spawn`, so a slow or failing notification never blocks the scan loop
or a ping response.

A check that ends up with no bound channel at all silently drops its alerts
(`deliver_event` returns early with only a `tracing::debug!`), so check
creation auto-binds every channel already configured on the project —
`Store::bind_all_project_channels` (one `INSERT … SELECT ... ON CONFLICT DO
NOTHING`, not a loop over `bind_channel`), called from both
`web::check_create_core` and `api::v1::create_check` right after
`store.create_check`. Existing checks are untouched. For the checks that still
end up unbound, `Store::checks_with_channels` (batched, same `$N`-placeholder
generation as `list_checks_for_projects`) tells the dashboard and project page
which rows to mark with a "no channel" chip, and the project page's
empty-channels state is a warning naming the consequence instead of a neutral
note.

## Templates & assets

Askama compiles `templates/*.html` into the binary at build time — **`cargo
build` is required after any template or route change** for the change to
take effect, including in the E2E harness (its `global-setup.js` only
rebuilds if `target/debug/pingward` doesn't already exist). Interactive
elements carry `data-testid` attributes, which both the Rust integration
tests and the Playwright E2E steps select on.

## Testing

Rust integration tests live in `tests/`, one file per feature area (e.g.
`admin.rs`, `csrf.rs`, `ping_api.rs`, `scheduler.rs`). Run them with `cargo
nextest run` — **not** `cargo test`, which the CI pipeline doesn't use either.
SQLite-backed tests run unconditionally against an in-memory database.
`tests/pg_store.rs` silently skips unless `TEST_DATABASE_URL=postgres://...`
is set, and `tests/smtp_e2e.rs` skips unless `PINGWARD_TEST_SMTP_HOST` is
set (with `PINGWARD_TEST_SMTP_PORT` and `PINGWARD_TEST_MAILPIT_API` for a
local mailpit relay). `docker compose up -d` starts both backends.

`e2e/` is a Playwright + playwright-bdd harness: `.feature` files paired
with `.steps.js` step definitions, run via `cd e2e && npm test` (which runs
`bddgen` to generate specs, then `playwright test`). A `global-setup`
ensures the binary is built; each scenario then spawns its own fresh
`pingward` binary against a temporary SQLite database on a random port, so
scenarios don't share state.

## How to make common changes

- **Add a DB column/table**: write the migration SQL in **both**
  `migrations/sqlite/` and `migrations/postgres/`, then add the field to the
  relevant struct in `models.rs` and thread it through the matching
  `Store` methods (using `$N` placeholders, not `?`).
- **Add an enum variant**: extend the corresponding `str_enum!` invocation
  in `models.rs` — it generates `as_str()` and `FromStr` for you.
- **Add a notifier**: implement `Notifier` in `notify.rs`, add a
  `ChannelKind` variant in `models.rs`, and wire it into `notifier_for`.
- **Add a route**: register it in the appropriate `routes()`
  (`web::routes()`, `ping::routes()`, or `api::routes()`). If it's under
  `/admin*`, extract `AdminUser` in the handler (before any `Form`/`HtmlForm`
  extractor) — `tests/admin.rs::non_admin_forbidden_on_every_admin_route`
  picks the route up automatically and will fail if the guard is missing. If
  it's under `/api/v1`, extract `ApiUser` in the handler (before any body
  extractor, e.g. `ApiJson`) —
  `tests/api_v1.rs::every_api_v1_route_requires_a_bearer_key` picks it up
  the same way.
