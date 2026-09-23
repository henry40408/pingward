# CLAUDE.md

Guidance for Claude Code in this repository. `ARCHITECTURE.md` holds the full
design rationale; this file lists what breaks things if you get it wrong.

## What this is

pingward: self-hosted, healthchecks-style uptime/cron monitor. Jobs ping a
per-check URL; a background loop marks overdue checks **down** and notifies via
per-check channels. One axum process serves a server-rendered UI (Askama) and
the machine `/ping/*` endpoints.

## Commands

- `cargo build` — **required after any template or route change** (templates
  compile into the binary; the E2E harness runs `target/debug/pingward`).
- `cargo run` — SQLite `pingward.sqlite3`, bind `127.0.0.1:8080`.
- CI lint: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo deny check`.
- Tests: `cargo nextest run` (never `cargo test`). One test:
  `cargo nextest run -E 'test(success_ping_marks_up)'` or a substring.
- `tests/pg_store.rs` **silently skips** without `TEST_DATABASE_URL=postgres://…`;
  `tests/smtp_e2e.rs` skips without `PINGWARD_TEST_SMTP_HOST`. `docker compose up -d`
  (Postgres 5432, mailpit 1025/8025), then export `TEST_DATABASE_URL`,
  `PINGWARD_TEST_SMTP_HOST=localhost`, `PINGWARD_TEST_SMTP_PORT=1025`,
  `PINGWARD_TEST_MAILPIT_API=http://localhost:8025`.

Version stamp: `build.rs` sets `GIT_VERSION` from `git describe --tags --always
--dirty` (footer). An explicit non-empty, non-`dev` `GIT_VERSION` env wins —
`docker.yml` passes it as a build arg since `.dockerignore` excludes `.git`.
Releases are `gh release create`; **never bump `Cargo.toml`'s `version`**.

### Browser E2E (`e2e/`, cucumber + thirtyfour)

- Its **own cargo workspace**: root `fmt`/`clippy`/`--workspace` never reach it —
  run `cargo fmt --all --check` and clippy inside `e2e/` too (CI does).
- `cd e2e && cargo test --test e2e` — whole suite (`harness = false`; scenarios run
  concurrently, one per core, max 4). One feature: `-- -i features/ping_kinds.feature`;
  one scenario: `--name "POST body"`; `--tags` works.
- Each scenario gets a fresh binary + temp SQLite DB on a random port (almost all
  walk through `POST /setup`). Selectors use `data-testid`.
- Needs a **local Chrome/Chromium**; only the driver is downloaded
  (`brew install --cask ungoogled-chromium`).
- Tags pick the environment (`e2e/tests/e2e/main.rs`, `server::Options::from_tags`):
  `@nojs` (only in `no_js.feature`) disables page scripts; `@fast-scan` sets
  `PINGWARD_SCAN_INTERVAL=1`; `@smtp-env`, `@trusted-proxy` add env. Anything
  `app.js` must not be the sole provider of needs a `@nojs` scenario; pair it
  with a JS-on one when both behaviours matter.
- cucumber-rs matches on the **Gherkin keyword**: a step used under `Given` and
  `When` needs both attributes. `{string}` args keep backslash escapes — pass
  through `pingward_e2e::unescape` when expecting a quote.

### README assets (outputs are committed)

- `cd e2e && cargo run --bin screenshots` → `docs/screenshots/*.png`.
  `e2e/src/seed.rs` must keep seeded timestamps inside each schedule's budget or
  the boot `scan_once` rewrites the status. Cron `dow` is **Sunday = 1** (the
  `cron` crate's numbering).
- **After any change to a template, `assets/app.css` or rendered copy, say
  unprompted whether the screenshots are now stale** — nothing fails when they
  drift. Compare the affected PNG, then regenerate or state it's out of frame.
  Build from a clean tree (or set `GIT_VERSION`): the footer stamp is in frame and
  `build.rs` can reuse a stale `-dirty` stamp.
- `cd e2e && cargo run --bin icons` → `assets/apple-touch-icon.png` from
  `assets/favicon.svg` (no browser needed).

## Invariants

### Routing & CSP
- `src/lib.rs::app` merges `web::routes()`, `ping::routes()`, `api` routes,
  `assets::routes()` + `/healthz`. `web` layer order is load-bearing:
  `forward_auth_session` → `anonymous_session` → `csrf_guard` → handler.
  `/ping/*` is a sibling, so structurally outside CSRF.
- CSP (`web::content_security_policy`, `web` router only) is `script-src 'self'`
  with **no `'unsafe-inline'`, no nonce**. So: **no inline scripts, no
  `onclick=`/`onsubmit=`** — add behaviour to `assets/app.js` via delegation
  (`data-href`, `data-confirm`, `data-nosubmit`, `data-reauth`). `style-src` keeps
  `'unsafe-inline'` only for heartbeat bar heights. `/api/docs` is deliberately
  outside the CSP (Scalar from CDN).

### Works without JavaScript (`tests/no_js.rs`)
- `data-href` rows must keep a real `<a>`. Fragment endpoints redirect a plain
  navigation to the embedding page (`wants_fragment`/`fragment_page_redirect`).
- `data-confirm` enforces nothing: destructive handlers run only with
  `?confirmed=1` (query, since some forms post no body), else render
  `templates/confirm.html`. The gate sits below authz and every refusal guard.
  See ARCHITECTURE.md "Confirming a destructive action".
- CSS hiding click-revealed content must hang off `:root.js` (set by
  `theme-init.js` before paint), e.g. `:root.js tr.exp:not(.open)`. JS-only
  controls are hidden via `:root:not(.js)`.
- `base.html` has **no default `data-theme`**. The light palette exists twice in
  `app.css` (`[data-theme]` + `prefers-color-scheme`) — keep in sync; the test
  compares them.
- History filters are real GET forms; each carries the sibling section's filter
  as hidden inputs (`carry_fields`/`clear_href`).
- Per-kind form fields switch via `:has()` in CSS, not JS. Absolute timestamps
  fall back to `view::fmt_utc` inside `.localtime[data-ts]` spans.

### Session, CSRF, secret (`src/secret.rs`)
- One `PINGWARD_SECRET` HMACs cookie, CSRF token and flash cookie with
  domain prefixes `session:`/`csrf:`/`flash:` — **keep the prefixes** (otherwise
  the CSRF token equals the cookie signature).
- CSRF is derived, not stored; `csrf_guard` has **no path exemptions**.
- **The cookie value is not the session id** — use
  `secret::session_id_from_jar`, never `cookie.value()`.
- Session rows are shown/addressed by a SHA-256 handle
  (`apikey::hash_api_key`), never the raw id.

### Auth (`src/auth.rs`, `src/web.rs`)
- `auth::validate_password` (15–128 *characters*, no trimming/composition) must be
  called by every surface that **sets** a password. **`/login` must never
  validate.**
- `login_submit` uses `verify_password_or_dummy` (constant cost for unknown users).
- `POST /login`: `login_limiter` (5/IP/60s) + `account_limiter` (10/account/15min,
  keyed on the submitted username before lookup; success `clear`s).
  `ratelimit::rate_limit_key` uses the **rightmost** XFF hop; `auth::client_ip`
  (attribution) uses the leftmost — **do not unify them**.
- `Store::create_user` returns `CreateUserError`; callers must handle
  `UsernameTaken`.
- `web::reauthenticate` guards `/account/password`, `/account/api-keys`,
  `/admin/unlock`; passwordless forward-auth accounts pass unchallenged.
- `src/elevate.rs`: in-memory 15-min unlock per session handle. Gate actions that
  **grant** access (`users_create`, `users_set_password`, promote); never gate
  delete/disable/demote. See ARCHITECTURE.md "Elevation for `/admin`'s
  access-granting actions".
- An admin cannot delete/disable/demote themselves.
- Owner scoping: `owned_project`/`owned_check` return **404, not 403**.
- Trusted proxies: `auth::is_trusted_proxy` (address or CIDR, canonicalised, no
  DNS). `ping::ClientIp` is the one extractor for pings and sessions. Under
  `axum-test` there is no `ConnectInfo`; see `tests/ping_source_ip.rs`.
- Auth logging (`pingward::auth`): `login.failed`, `reauth.failed`
  (`surface` = `password_change`/`api_key_create`/`admin_unlock`),
  `csrf.rejected`. Never log passwords; log usernames via `auth::log_username`
  with `?`. `csrf.rejected` `token_missing` is `debug!`, others `warn!` — pinned
  by `tests/csrf_logging.rs`.

### Admin & audit
- Cross-user admin handlers reuse owner templates. `web::audits_as_mutation`
  audits non-GET only; the ping URL of another user's check is a disclosure,
  revealed via audited `POST /admin/checks/{id}/ping-url` (`web::CheckPageViewer`).
  The REST API audits every admin cross-user access. See ARCHITECTURE.md
  "Reading vs. disclosing under `/admin`".
- Retention (`pings_/notifications_/audit_retention_days`) defaults to **off**;
  `settings_save` audits changed keys as `settings.update`.

### Persistence (`src/db.rs`, `src/store.rs`)
- One sqlx `AnyPool`, SQLite or Postgres by URL scheme. All SQL via `Store`, must
  run on both: **`$N` placeholders** (`?` is not translated), `RETURNING id`, no
  `ILIKE` (filter case-insensitively in Rust).
- Schema changes go in **both** `migrations/sqlite/` and `migrations/postgres/`;
  they're embedded with `sqlx::migrate!` — never read from disk at runtime.

### Background loops & shutdown
- `main.rs` spawns `scheduler::run_scan_loop` and `prune::run_prune_loop`.
  Changing either loop's params means updating `main.rs` and `tests/scheduler.rs`.
- `src/shutdown.rs` handles SIGTERM/SIGINT — **mandatory** (PID 1 in the image
  ignores default-disposition signals). Drain: server (≤ `HTTP_DRAIN_TIMEOUT`; SSE never ends
  by itself) → loops → join →
  `store.pool.close()` within `POOL_CLOSE_TIMEOUT` (checkpoints SQLite WAL).
- Live tail: `AppState::events` broadcasts a `check_id` (from `ping::apply` and
  `run_scan_loop`, only when `receiver_count() > 0`); `web::sse_for_check` sends
  a data-less `"changed"` and the browser re-fetches the fragment. In-process only.

### Scheduling & display
- `due_time` = last success (else creation) + period/cron + grace. Cron is 6-field
  (`sec min hour dom mon dow`) in the check's timezone (`web::validate_timezone`).
- Scan interval cascades check → project → global → env; nag interval check →
  project → global (no env default, off when unset). Non-positive falls through.
- Duration fields accept seconds or `5m`/`1h30m`/`2d` (`duration::parse_duration`),
  stored as seconds, re-rendered with `duration::fmt_duration` (lossless);
  `view::fmt_secs` is lossy display only. Retention fields are plain integers.
- Dashboard ordering and `q`/status filtering happen in Rust; `Store` lists stay
  in id order (shared by other pages and the API). Loads are batched — keep the
  query count fixed; heartbeat reads the narrow `models::PingSummary`, not `body`.
- Check-page heartbeat: never narrow the window to `kind IN ('success','fail')` —
  `run_durations` needs the `start` pings. No run count in the caption.
- `view::display_status` adds display-only `late`/`running`; precedence
  `Paused > Down > Running > Late > Up`. `view::next_due` uses
  `scheduler::due_time`, not `checks.next_due_at` (often NULL).

### Notifications (`src/notify.rs`)
- Six `Notifier`s; delivery is fire-and-forget (`tokio::spawn`) with `RetryPolicy`.
- Check creation (web and API) calls `Store::bind_all_project_channels`.
- Channel edit: **blank field keeps the stored value; never re-render a secret**
  (webhook/Slack URLs count). Single validator `validate_channel_update`;
  templates see only `ChannelEditView`. `kind` is immutable. See ARCHITECTURE.md
  "Editing a channel without leaking its secrets".
- Message = `notify::event_text`, max **four lines**; `EventDetail` is built at the
  call site from the **pre-update** snapshot, never re-read during delivery.
  Webhook payload changes are additive only. See ARCHITECTURE.md "What a
  notification says".

### Models
- String-backed enums come from `str_enum!` in `src/models.rs` — add variants there.

## Config (`src/config.rs`)

Test parsing via `Config::from_map`, not real env. Unparseable duration env vars
fall back to defaults. The README's table is the user-facing reference; `/admin`'s
Environment card shows these read-only, secrets only as configured/not-set.

`DATABASE_URL` (`sqlite://pingward.sqlite3?mode=rwc`), `PINGWARD_BIND`
(`127.0.0.1:8080`), `PINGWARD_BASE_URL` (`http://localhost:8080`; ping URLs and
notification links), `PINGWARD_SCAN_INTERVAL` (30s), `PINGWARD_PRUNE_INTERVAL_SECS`
(3600), `PINGWARD_LOG_FORMAT` (`full` default; `compact`/`pretty`/`json`),
`RUST_LOG` (default `error,pingward=info`), `PINGWARD_TRUSTED_PROXIES`,
`PINGWARD_FORWARD_AUTH_HEADER`, `PINGWARD_FORWARD_AUTH_LOGOUT_URL`,
`PINGWARD_SECRET` (≥16 bytes, else random per process → every restart signs
everyone out), `PINGWARD_COOKIE_SECURE` (default from `PINGWARD_BASE_URL`
scheme; also switches the cookie name to `__Host-pingward_session`),
`PINGWARD_HSTS_MAX_AGE` (0 = off), `PINGWARD_SMTP_{HOST,FROM,PORT,USERNAME,PASSWORD,TLS}`
(host + from enable email).
