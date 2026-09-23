# Architecture

The code map for contributors. Install, config and API usage are in
[README.md](README.md); this explains how the pieces fit together.

## Overview

pingward is a single `axum` process serving a server-rendered browser UI
(Askama templates compiled into the binary), machine `/ping/*` endpoints, and
a bearer-authenticated REST API under `/api/v1` with an OpenAPI document and
Scalar UI. All three share one `AppState` and one `sqlx::AnyPool` that talks to
SQLite or Postgres.

## Repository layout

| Path                   | Contents                                                                |
| ---------------------- | ----------------------------------------------------------------------- |
| `src/`                 | Router composition, handlers, domain logic                              |
| `src/api/`             | The `/api/v1` REST surface (DTOs, input parsing, extractors, handlers)  |
| `templates/`           | Askama HTML templates, compiled in at build time                        |
| `assets/`              | CSS, scripts, embedded fonts and icons, served by `src/assets.rs`       |
| `migrations/sqlite/`   | SQLite migrations                                                       |
| `migrations/postgres/` | The same migrations, hand-duplicated for Postgres                       |
| `tests/`               | Integration tests, one file per feature area (`cargo nextest run`)      |
| `e2e/`                 | cucumber + thirtyfour browser tests; its own cargo workspace            |

## Module map

- `src/lib.rs` — module declarations and `app()`, which composes the `Router`.
- `src/main.rs` — entry point: reads `Config`, sets up tracing, connects and
  migrates the DB, spawns the two background loops, runs `axum::serve`, drains
  on SIGTERM/SIGINT (see *Graceful shutdown*). Installs **mimalloc** as the
  `#[global_allocator]` (binary only).
- `src/web.rs` — the browser UI: `routes()`, page/form handlers, the session,
  CSRF and response-header middleware, and owner/admin scoping helpers
  (`owned_project`, `owned_check`, `admin_project`, `admin_check`).
- `src/ping.rs` — `/ping/{uuid}[/fail|/start|/log|/{code}]`.
- `src/api/` — `mod.rs` (router, OpenAPI/Scalar handlers), `v1.rs` (handlers),
  `dto.rs` (response shapes), `input.rs` (request bodies), `extract.rs`
  (`ApiUser` bearer extractor), `error.rs` (JSON error type).
- `src/auth.rs` — session cookie naming, argon2 hashing, the password policy,
  forward-auth and client-IP resolution, and the `CurrentUser`/`OptionalUser`/
  `AdminUser` extractors.
- `src/secret.rs` — the process secret and every HMAC derived from it (session
  cookie, CSRF token, flash cookie).
- `src/elevate.rs` — the in-memory per-session unlock for `/admin`'s
  access-granting actions.
- `src/apikey.rs` — API key generation (`pw_...`) and SHA-256 hashing.
- `src/ratelimit.rs` — `RateLimiter<K>`, the in-memory fixed-window limiter
  behind both `POST /login` limiters, plus the key functions `rate_limit_key`
  (per address) and `account_key` (per username).
- `src/state.rs` — `AppState` (`store`, `config`, `events`, `login_limiter`,
  `account_limiter`, `elevations`) with `FromRef` impls.
- `src/store.rs` — `Store`, the only data-access layer.
- `src/db.rs` — `connect()` (builds the pool, applies SQLite pragmas) and
  `migrate()` (picks the embedded migrator by URL scheme).
- `src/models.rs` — domain structs and the `str_enum!` string-backed enums
  (`CheckStatus`, `PingKind`, `ScheduleKind`, `ChannelKind`, `NotifyStatus`).
- `src/scheduler.rs` — `due_time`/`overrun_time`, `scan_once` (downs
  overdue/overrun checks), `nag_once` (reminders), `run_scan_loop`.
- `src/prune.rs` — `prune_once` (retention-based deletion of pings,
  notifications and audit rows, plus expired sessions; returns `PruneCounts`)
  and `run_prune_loop`. All three retention settings default to **off** — a
  default that started deleting the audit trail on upgrade would destroy the
  record it exists to keep.
- `src/shutdown.rs` — the shutdown flag (`channel()` → `ShutdownTx`/`Shutdown`)
  and `os_signal()`.
- `src/notify.rs` — the `Notifier` trait and its six implementations,
  `notifier_for`, `deliver_event`, and notification text.
- `src/config.rs` — `Config` (`from_env`, testable via `from_map`),
  `SmtpConfig`, and the `effective_scan_interval`/`effective_nag_interval`
  cascades.
- `src/duration.rs` — `parse_duration`/`fmt_duration` (`5m`, `1h30m`, `2d`),
  lossless, used by form fields and duration env vars.
- `src/view.rs` — template helpers: the lossy display formatter `fmt_secs`,
  `fmt_utc`, `next_due`, and `display_status` (below).
- `src/markdown.rs` — rendering of project/check descriptions.
- `src/assets.rs` — serves `app.css`, `app.js`, `theme-init.js`, the fonts and
  the icons, content-addressed by hash (`IMMUTABLE_CACHE`).
- `src/error.rs` — `AppError`, the app-wide `IntoResponse` error.

**Display status** (`view::display_status`) layers `late` and `running` on top
of the stored `CheckStatus`, so the stored status keeps its narrower meaning.
Precedence is `Paused > Down > Running > Late > Up`: a long-running job
legitimately drifts past its deadline, so `Running` beats `Late`, but an
in-flight run must never mask an alert. `view::next_due` derives the header
countdown from `scheduler::due_time`, **not** `checks.next_due_at` — that column
is only stamped by `ping::apply`, so it is `NULL` for a never-pinged check and
for one downed by a `fail` ping, whereas `due_time` is what `scan_once`
evaluates. The deadline includes grace (hence "due"); a paused check shows none.

## Request lifecycle / router composition

`lib.rs::app()` merges sibling routers and attaches `AppState`:

```rust
Router::new()
    .route("/healthz", get(|| async { "ok" }))
    .merge(web::routes()
        .layer(csrf_guard)          // innermost
        .layer(anonymous_session)
        .layer(forward_auth_session)
        .layer(no_store)
        .layer(content_security_policy)) // outermost of the `web` layers
    .merge(ping::routes())
    .merge(api::routes())
    .merge(assets::routes())
    .layer(hsts)                        // app-wide
    .layer(security_headers)            // outermost overall
    .with_state(state)
```

Only `web::routes()` carries the session/CSRF layers. Because the other routers
are merged as *siblings*, `/ping/*`, `/api/*`, assets and `/healthz` are
**structurally** exempt from CSRF — no change inside `web::routes()` can start
covering them. `csrf_guard` passes GET/HEAD/OPTIONS and otherwise requires the
token in `X-CSRF-Token` or the `_csrf` form field.

**`no_store`** sets `Cache-Control: no-store` unless a response already has one,
so authenticated pages and the `/login`/`/setup` forms (which embed a
cookie-bound `_csrf`) are never cached. It is response-only; it sits outside
the session layers purely to wrap their early returns (e.g. `csrf_guard`'s
403s). `api::routes()` layers it a second time on just `/api/docs` and
`/api/openapi.json`, which also accept a web session. `/api/v1` is
bearer-authenticated and deliberately left alone.

**`content_security_policy`** is `web`-scoped because it describes pages this
app renders. `script-src 'self'` with no `'unsafe-inline'` and no nonce holds
only because every script is a file under `/assets` (`app.js` and the
render-blocking `theme-init.js`) and **no template carries an inline
`onclick=`/`onsubmit=`**. Row navigation (`data-href`), confirmation
(`data-confirm`) and non-submitting filter forms (`data-nosubmit`) are
delegated handlers in `app.js`, which also survive a fragment swap. Adding an
inline handler means weakening the policy for the whole UI or minting a nonce
per response — put the behaviour in `app.js` behind a `data-` attribute.
`style-src` keeps `'unsafe-inline'` for the heartbeat bars' computed
`style="height:Npx"`. `/api/docs` is outside the CSP: Scalar loads from
`cdn.jsdelivr.net`, and admitting a CDN app-wide would cost every other page.

**`security_headers`** is app-wide — `nosniff`, `X-Frame-Options: DENY`,
`Referrer-Policy: same-origin`, and a `Permissions-Policy` denying
geolocation/camera/microphone/payment/usb. These are not about markup, so they
also cover what the CSP skips: nosniff matters for `/api/v1` JSON and captured
ping bodies served as `text/plain`; `X-Frame-Options` keeps `/api/docs`
unframable. Each is set only if absent, so a handler can override.

**`hsts`** (`PINGWARD_HSTS_MAX_AGE`, default off — pingward does not terminate
TLS) is app-wide because HSTS describes the whole origin. Response-only, so its
position relative to the request chain is irrelevant.

### Working without JavaScript

The UI must stay usable with scripts off; `tests/no_js.rs` and
`e2e/features/no_js.feature` guard it.

- **`data-href` is a mouse convenience over a real link.** Each row's name is an
  `<a>` to the same destination, which carries keyboard access, middle-click,
  and navigation without JS.
- **CSS that hides what a click reveals hangs off the `js` class on `<html>`**,
  set by `theme-init.js` before first paint (not the deferred `app.js`, which
  would flash every panel open then snap them shut). `:root.js
  tr.exp:not(.open)` alone collapses the ping/audit panels, so without script a
  failed job's captured output stays readable; carets and pointer cursors are
  hidden in the same state.
- **`base.html` carries no default `data-theme`.** `theme-init.js` always sets
  it, so `app.css`'s `@media (prefers-color-scheme: light) {
  :root:not([data-theme]) }` only ever applies to a scriptless browser and
  cannot fight the toggle. That light palette is therefore written **twice** (a
  selector list cannot span a media query); `tests/no_js.rs` compares the
  copies token by token.
- **Script-only controls are not drawn** without script (`:root:not(.js)` hides
  `.copy`, `.live-toggle`, the theme toggle). Content is the opposite: `/admin`'s
  heartbeat age is rendered server-side (`web::relative_setting`) and merely
  re-ticked by `app.js`.
- The check form's period/cron fields and the channel form's per-kind blocks
  are switched by `:has()` rules in `app.css`, not JS — an inline
  `style.display` would outrank the stylesheet, so there must be only one
  mechanism.
- Every absolute timestamp's fallback text is `view::fmt_utc`
  (`2026-08-13 17:54:27 UTC`), what remains when `app.js` does not localise the
  `.localtime[data-ts]` span.
- There is deliberately no `<noscript>` banner: what remains missing (live tail,
  local times, clipboard buttons) does not read as breakage.
- `data-confirm` enforces nothing; see "Confirming a destructive action".

### Session layers

Both orderings are load-bearing:

- The two session layers run **before** `csrf_guard` so a cookie minted during a
  request is visible to the guard on that same request. Each layer therefore
  rewrites the request's `Cookie` header as well as setting `Set-Cookie`, so a
  handler rendering a form can derive the matching token immediately.
- `forward_auth_session` runs **before** `anonymous_session` so that when both
  would mint, the real session wins; reversed, the anonymous `Set-Cookie` would
  be appended last and shadow it.

**`anonymous_session`** gives every visitor a signed cookie but writes **no
`sessions` row** — the CSRF token is derived from the id, so an id alone
suffices. Hence `csrf_guard` needs no path exemptions (`/login` and `/setup`
are protected like everything else), and `auth::resolve_user` needs no special
case (an anonymous id matches no row). Login rotates to a fresh id, so a
planted anonymous cookie cannot become an authenticated session.

**`forward_auth_session`** turns a trusted `PINGWARD_FORWARD_AUTH_HEADER`
identity into a real session row plus cookie; without it such a user would have
no `_csrf` and every POST would 403. It short-circuits when forward auth is
unconfigured or the request already has a live session — checked by lookup,
not signature, since a valid signature no longer implies a row. These rows are
stamped `sso = true` and badged "SSO" on `/account`.

**Logout cannot be local-only under forward auth**: the next request still
carries the gateway header and gets a new session. So `web::logout`:

- redirects to `PINGWARD_FORWARD_AUTH_LOGOUT_URL` if set (from the environment,
  never the request, so not an open redirect);
- otherwise, if the logout request itself carries the trusted identity header
  (`auth::forward_auth_username`, peer via `PeerAddr`), lands on `/` with a
  one-shot `forward_auth_logout` flash saying only the proxy can end the
  session;
- otherwise (password user) redirects to `/login`.

`login_page` bounces an already-authenticated visitor to `/`.

The `/login` and gateway exits send `Clear-Site-Data: "cache"`
(`web::CLEAR_SITE_DATA`). Not `"cookies"`: that directive covers the whole
registered domain, so on the sibling-subdomain SSO layout it would delete the
gateway's own cookie before the redirect and sign the user out of every app on
the domain; the session cookie is already removed by its origin-scoped removal
cookie. Not `"storage"`: it would wipe the `pw-theme` preference for no
benefit. The flash exit sends no header — it is not a teardown and must carry
the flash cookie. Browsers ignore the header on non-HTTPS origins.

`/api/v1` authenticates via `ApiUser` only; `/api/docs` and `/api/openapi.json`
also accept `CurrentUser` but are read-only GETs, so they add no CSRF-relevant
ambient authority.

### Session and CSRF secret

Every browser credential is keyed off one process secret (`src/secret.rs`,
`PINGWARD_SECRET`):

```
session cookie = <session_id>.<HMAC-SHA256(secret, "session:" ++ session_id)>
CSRF token     =                HMAC-SHA256(secret, "csrf:"    ++ session_id)
flash cookie   = <payload>.<HMAC-SHA256(secret, "flash:"   ++ payload)>
```

The prefixes are load-bearing: without them the CSRF token equals the cookie
signature, and every rendered form would print it.

- **`sessions` has no `csrf_token` column.** Rendering and checking a token are
  pure computation, and a session id needs no row to carry one — which is what
  makes the row-free `anonymous_session` possible.
- **The cookie is verified before any DB work** (`secret::verify_session`), so
  a forged or DB-leaked id never reaches a lookup. The cookie value is *not*
  the session id; always go through `secret::session_id_from_jar`.

Rotating the secret ends every browser session at once (rows remain until
pruned). Unset, a random secret is generated per process, so **every restart
signs everyone out**; `main::warn_on_ephemeral_secret` says so at startup. API
keys (SHA-256 digests, `src/apikey.rs`) never touch this secret.

### Cookie attributes

Session and flash cookies are each built by one mint/removal pair
(`web::session_cookie`/`session_removal_cookie`,
`web::flash_cookie`/`flash_removal_cookie`) so attributes cannot drift — a
removal cookie must match its mint exactly or some browsers will not clear the
original (RFC 6265bis §5.5).

- `HttpOnly`, `SameSite=Lax`, `Path=/`.
- `Secure` follows `Config::cookie_secure` (`config::parse_cookie_secure`):
  `PINGWARD_COOKIE_SECURE` if set, else whether `PINGWARD_BASE_URL` is
  `https://`. Not hardcoded, because browsers drop `Secure` cookies over plain
  HTTP and a LAN deployment could never log in.
- **No `Max-Age`/`Expires`** — deliberately non-persistent (OWASP); expiry is
  server-side only. Do not add one.
- **The name depends on `Secure`** (`auth::session_cookie_name`,
  `web::flash_cookie_name`): `__Host-pingward_session`/`__Host-pingward_flash`
  when secure, else the unprefixed names. The browser enforces that a `__Host-`
  cookie is `Secure`, `Path=/` and domainless, so a sibling subdomain or
  downgraded response cannot overwrite it; on plain HTTP the browser would
  refuse it outright, hence conditional. The read side takes the resolved name
  (`session_id_from_jar(jar, secret, cookie_name)`).
- **The flash cookie is signed** (`secret::sign_flash`, verified in
  `web::flash_payload`). It carries no authority — a fixed key, or
  `password_reset_keys:<revoked>:<keys>` parsed as integers and escaped — but
  unsigned, a sibling subdomain could make the page show a message the server
  never sent (e.g. a fake "N API keys still work"). The prefix blocks that on
  HTTPS, the signature on plain HTTP. Removal cookies are empty and unsigned.

**Flipping `PINGWARD_COOKIE_SECURE` or the base URL's scheme renames the cookie
and signs everyone out once.** There is no fallback read of the other name, on
purpose: a permanent branch for a one-time, self-healing inconvenience
(`0012_session_secret.sql` did the same with `DELETE FROM sessions`).

## Persistence

One `sqlx::AnyPool` (`db::connect`) dispatches on the `DATABASE_URL` scheme.
All queries go through `Store`. The `Any` driver does **not** translate `?`, so
use `$N` placeholders and `RETURNING id`.

Migrations are hand-duplicated in `migrations/sqlite/` and
`migrations/postgres/`; a schema change means writing both. Both sets are
embedded with `sqlx::migrate!`, because the release image ships only the binary
and runs from `/data` — a migrator reading `migrations/` from disk would panic.

`db::connect` applies SQLite-only pragmas per connection: `foreign_keys` (so
`ON DELETE CASCADE` works; Postgres does this natively), `busy_timeout` 5000ms,
and, for file DBs, WAL with `synchronous = NORMAL`. In-memory SQLite is capped
at one connection since `:memory:` is per-connection.

**List-of-lists pages batch their child loads.** `Store` has a batched sibling
beside each per-parent query (`list_checks_for_projects`,
`list_recent_ping_summaries_for_checks`), building `IN ($1,…,$N)` and returning
a `HashMap` keyed by parent id (childless parents absent);
`checks_with_channels` returns a `HashSet<i64>` of checks with a bound channel.
The dashboard's query count is therefore fixed regardless of project/check
count.

**The heartbeat window is narrowed**, not just batched: it selects
`id, check_id, kind, created_at` into `models::PingSummary`. `view::heartbeat`
and `view::run_durations` read nothing else, and decoding whole rows meant
decoding captured bodies (up to `ping::MAX_BODY`, 10 KiB, 40 rows per check)
only to drop them. #116 measured `GET /` 24–74% slower with the wide form, and
the win holds even without large bodies.

The dashboard reads a 40-row window for its six bars. The check page's strip is
card-width, and only the browser knows how many bars fit, so the server renders
`web::HEARTBEAT_BARS` (120) from `web::HEARTBEAT_WINDOW` (300 rows) and `.beat`
in `app.css` clips from the *left* (`justify-content: flex-end`,
`overflow: hidden`), keeping the newest run pinned right. `.wrap`'s 1080px cap
bounds the visible count at ~100. Do **not** filter the window to
`kind IN ('success','fail')` — `run_durations` pairs each finish with the
preceding `start`, and dropping starts flattens every bar. No caption may state
a bar count.

## Auth & authorization

Sessions are a `session_cookie_name(cookie_secure)` cookie plus argon2 password
hashes (`src/auth.rs`). An optional forward-auth header
(`PINGWARD_FORWARD_AUTH_HEADER` + `PINGWARD_TRUSTED_PROXIES`) auto-provisions a
passwordless non-admin user, only when the peer is a trusted proxy.

**Session expiry has two independent layers:**

- **Idle** — `sessions.expires_at` = last activity + `SESSION_IDLE_TTL_HOURS`
  (72h), checked in SQL, slid forward by `auth::refreshed_expiry` in
  `Store::find_session_user` only once past the window's half-life (~one write
  per 36h). `last_seen_at` has its own 60s throttle for `/account`'s display.
- **Absolute** — `SESSION_ABSOLUTE_MAX_DAYS` (30) from `created_at`, never
  extended. Enforced in Rust (`auth::is_past_absolute_cap`), not SQL, because
  pre-`0010` rows can have `created_at = ''`, which sorts below every RFC3339
  string and would read as infinitely old. `list_sessions_for_user` filters on
  it; `delete_expired_sessions` deletes on either layer (excluding
  `created_at = ''`); and `render_account` first calls
  `delete_capped_sessions_for_user`, so "not listed" means "gone" rather than
  an unrevokable hidden row.

`refreshed_expiry` also **clamps downward**: a stored `expires_at` beyond
`min(now + idle, cap)` is pulled down on the next request, bypassing the
throttle. `0015_invalidate_legacy_sessions.sql` deleted the old single-layer
rows, and migration runs before `bind`, but the clamp still covers a stale
binary writing to a shared DB (rolling deploy, mixed-version instances) and a
future build that lowers `SESSION_IDLE_TTL_HOURS`. `is_past_absolute_cap`
remains the backstop.

**Trusted proxies.** `auth::is_trusted_proxy` is the single gate for
forward auth and `auth::client_ip`. An entry is an address or **CIDR block** —
a containerised proxy's bridge-network address changes when the network is
recreated. Comparison and storage are canonical (an IPv4-mapped IPv6 peer
matches an IPv4 entry); unparseable entries match nothing; no DNS.

Two address resolvers exist and **must never be merged**:

- `ping::ClientIp` wraps `auth::client_ip`, the **leftmost** `X-Forwarded-For`
  hop, for *attribution* (`pings.source_ip`, session rows).
- `ratelimit::rate_limit_key` uses the **rightmost** hop, for the limiter.
  Under an appending proxy (nginx `$proxy_add_x_forwarded_for`, Caddy) the
  leftmost hop is client-controlled, so keying a security control on it would
  let an attacker mint a bucket per request.

### Login rate limiting

`POST /login` has **two** `RateLimiter`s, both reserved *before* argon2 so a
refused attempt costs nothing. They share one generic implementation so the
window roll-over, atomic check-and-record, key cap and overflow bucket exist
once.

- **`login_limiter`** (`RateLimiter<IpAddr>`): `MAX_ATTEMPTS` 5 per
  `WINDOW_SECS` 60, keyed by `rate_limit_key`. A success `release`s the one
  attempt; it must not clear, since a shared NAT may also carry an attacker.
- **`account_limiter`** (`RateLimiter<String>`): `ACCOUNT_MAX_ATTEMPTS` 10 per
  `ACCOUNT_WINDOW_SECS` 900, per account. A per-address counter cannot see a
  distributed attack (N addresses buy `5 × N` guesses).
  - Keyed on the **submitted** username (`ratelimit::account_key`) and charged
    *before* lookup, so an invented name throttles identically — otherwise
    throttling is a username oracle. Only length is bounded, never case:
    usernames compare exactly on both backends.
  - A success **`clear`s** the bucket, so an owner who mistyped nine times is
    not left one failure from lockout.
  - A lockout is a **DoS primitive** against any known username; accepted, not
    solved. The correct password is refused too
    (`tests/login_rate_limit.rs::a_locked_account_refuses_even_the_correct_password`).
    It is kept proportionate by being a rolling window, per account,
    per process, and irrelevant under forward auth.

Both answer with the same 429 (`web::throttled_login`), differing only in
`Retry-After`, and never imply the username exists; the log distinguishes them
(`rate_limited` vs `account_locked`, see "Rejected authentication attempts").

The tracked-key map is capped (`MAX_ENTRIES`, 10 000). At the cap, elapsed
windows are pruned first; if all are live, a new key is charged to one shared
**overflow bucket** (`max_attempts × OVERFLOW_FACTOR`, 50 per window for the
IP limiter) instead of being admitted unmetered — otherwise holding 10 000 live
windows would be the bypass. Refusing outright would turn a spray into a global
lockout, and existing counters are never evicted (that would let a throttled
client reset itself).

State is in-process, like `AppState::events`: replicas count separately and a
restart resets everything.

### The password policy, and why `/login` is exempt from it

`auth::validate_password` is the only validator, called by every surface that
**sets** a password: `setup_submit`, `account_password`, `users_create`,
`users_set_password`. Length only: `MIN_PASSWORD_CHARS` (15) to
`MAX_PASSWORD_CHARS` (128), counted in characters. No composition rules, no
excluded characters, no trimming (NIST SP800-63B / OWASP). 15 is the no-MFA
figure; adding a second factor is what would change it. Over-long is a
**rejection**, never a truncation, which would authenticate a prefix the user
did not choose. The maximum is not a DoS defence — argon2's cost barely moves
with input length.

`POST /login` does **not** validate and must not: a floor at sign-in would lock
out every credential predating the policy. `users_set_password` renders its
rejection rather than redirecting, since a bare redirect to an unchanged page
looks like success.

A breached-password blocklist is deliberately absent: at a 15-character floor
it gains little against a SHA-1 dependency plus an outbound request or a stale
checked-in list. `validate_password` is the seam to add it at.

### Equal cost for an unknown username

`login_submit` calls `auth::verify_password_or_dummy`, never a bare
`verify_password`. With no stored hash (unknown user, passwordless forward-auth
account) it still verifies against a per-process throwaway hash
(`dummy_password_hash`) and discards the result via `black_box`. Otherwise
response time reveals which usernames exist
(`tests/auth_web.rs::an_unknown_username_costs_the_same_as_a_wrong_password`
measured ~1.9ms vs ~590ms without it). The preceding user lookup still differs,
but by orders of magnitude less.

### Re-authentication for sensitive actions

`web::reauthenticate` asks for the signed-in user's own password again, because
a cookie proves who opened the browser, not who is at it. It gates
`POST /account/password`, `POST /account/api-keys` and `POST /admin/unlock`.
CSRF and XSS are already closed (derived tokens, strict CSP), so the residual
threat is a **borrowed or exported session cookie** — which is why it guards
these three rather than every mutation.

API-key creation is the case that justifies it alone: a key is bound by neither
session cap and survives `users_set_password`, so a borrowed browser would
otherwise become permanent access that signing out cannot undo. The check runs
before name/expiry validation.

- **A passwordless forward-auth account passes unchallenged** and is not shown
  the field (`has_password` gates it and the password card): there is nothing
  to verify and no way to ask the gateway to re-assert. So a borrowed
  forward-auth session can still mint a key — an accepted asymmetry, since
  refusing would leave those users with no way to get one. Contrast
  `account_password`'s 403 for the same accounts: a local password there would
  be a second way in the gateway's sign-out could not end.
- **Attempts go through `account_limiter`**, keyed as for login, so a stolen
  session is not an unmetered password oracle. Success clears the bucket.

### Creating a user, and the order of the checks

`Store::create_user` returns `CreateUserError`, not `sqlx::Error`, so a
duplicate username cannot fall into `AppError::Db`'s blank 500. `UsernameTaken`
is classified from the backend's unique-violation code (SQLite 2067, Postgres
23505; `tests/pg_store.rs` covers the latter).

`users_create` runs **username → password policy → duplicate → elevation gate →
hash → insert**. Everything before the gate is read-only, so a doomed
submission says why instead of demanding a confirmation first (a locked admin
learns nothing the user list does not show). **The gate must stay immediately
above the first side effect**; `tests/admin_elevation.rs` pins both halves.

The pre-check does not replace the error mapping — two admins can race it.
`setup_submit` also handles `UsernameTaken`, for two visitors racing the first
`/setup`.

Usernames compare **exactly** (`Admin` ≠ `admin`), matching the constraint;
changing that is a migration, not a validator tweak.

### Elevation for `/admin`'s access-granting actions

`/admin`'s controls are single-button inline forms (`users_toggle_admin` posts
no body), so re-auth is decoupled from the action (`src/elevate.rs`): a refused
action redirects to `GET /admin/unlock`; `POST /admin/unlock` runs
`reauthenticate`; `web::elevation` checks freshness (`ELEVATION_TTL_SECS`,
15 minutes).

With JS, `app.js` pre-empts the bounce so the filled-in form is not lost: forms
carrying `data-reauth` (rendered only while `elevation_locked`, naming the
action) open a native `<dialog>` that posts to `/admin/unlock` with
`X-Requested-With: fetch` — getting 204/403/429 instead of HTML, same decision
either way — then submits the original form. Anything unexpected falls back to
the page. The server re-checks regardless, so marker/handler drift costs a
needless prompt, never an ungated action. The dialog is wired by delegation
(CSP). The form is not preserved server-side across the bounce because that
would mean stashing a password across a redirect; likewise a refused action is
**not replayed** after unlocking.

The interstitial is a **page, not a field**: an already signed-in admin asked
again needs to be told why, which actions it covers and which it does not, how
long it lasts (rendered from the constant), and that it is the same password,
**not** a second factor. `/admin` keeps a one-line note linking to it.

The line is **granting versus removing access**:

| Gated | Not gated |
| --- | --- |
| `POST /admin/users` | `POST /admin/users/{id}/delete` |
| `POST /admin/users/{id}/password` | `POST /admin/users/{id}/disabled` |
| `POST /admin/users/{id}/admin` **when promoting** | the same route when demoting |

Gated actions hand out access that outlives the session. Removing access must
not require finding a password during a suspected compromise. Also ungated:
`settings_save` (already audited as `settings.update`) and
`POST /admin/checks/{id}/ping-url` (audited, and recoverable by regenerating).

- **In-memory, per process** — elevation is short-lived, so a restart or second
  replica just asks again (the safe direction). No migration.
- **Keyed per session by SHA-256 handle**, never the raw id; unlocking one
  browser does not unlock another. Every handler that ends a session (logout,
  revocation, password change/reset, disable, delete) drops its elevation;
  expiry leaves it to the far shorter TTL.
- **A passwordless forward-auth admin is never gated**
  (`Elevation::not_applicable`), same reasoning as above.

A refusal is a redirect, not a 403: the controls stay live (hiding them would
make the page depend on a timer), so the honest response is to explain.

### Confirming a destructive action

Irreversible controls carry `data-confirm`, which `app.js` turns into a
`confirm()`, but that is inert without script. So the gate is server-side: the
handler acts only with `?confirmed=1`, otherwise rendering
`templates/confirm.html` — the same question as a page, with a form re-posting
the action plus the flag. `app.js` appends the flag after its dialog, so a
scripted browser still posts once. Neither side trusts the other.

- **The flag is a query param.** Several forms post no body
  (`users_toggle_admin`), and a body extractor would 415 *before*
  authorization, turning `owned_check`'s 404 into a content-type error.
  `Query<ConfirmQuery>` is infallible; absent means unconfirmed.
- **The gate sits below authorization and every refusal guard.** A stranger's
  check is a 404, not a confirmation prompt; a delete the self-guard or
  last-admin guard will block says so instead of asking first.
- **The `/admin` toggles confirm in one direction only**, matching the
  template: demote and disable ask; promote (already elevation-gated) and
  re-enable do not.

The page copy is longer than the dialog's single line.

### Rejected authentication attempts

Failures are `tracing` events under `pingward::auth` (separate from
`pingward::session`). Nothing else records a rejected attempt — `audit_log`
records successes and the limiters tell nobody — so this is the only spray
signal. One event per layer, discriminated by a field, so one alert rule
catches all of it:

- `login.failed` (`web::log_login_failure`) — `username`, `ip`, `bucket`,
  `reason` ∈ `bad_credentials`, `account_disabled`, `rate_limited` (per
  address), `account_locked` (per account — someone is working on one
  account). `ip` is the attribution address (`ClientIp`), `bucket` the limiter
  key (`rate_limit_key`); they can differ, see above.
- `reauth.failed` (`web::log_reauth_failure`) — `username`, `user_id`,
  `surface` ∈ `password_change`, `api_key_create`, `admin_unlock`; `reason` ∈
  `bad_current_password`, `rate_limited`.
- `csrf.rejected` (`web::log_csrf_rejection`) — `reason`, `handle`, for
  `no_session`, `header_mismatch`, `body_unreadable`, `token_missing`,
  `token_mismatch`. **`token_missing` is `debug!`, the rest `warn!`**: the guard
  answers before `login_limiter`, and bots never send `_csrf`, so that reason
  carries all the unthrottled noise, while the others mean a token was
  presented and failed. `no_session` is unreachable while the layer order holds,
  so it warns. `tests/csrf_logging.rs` pins the levels — do not flatten them.

The 403 stays bodyless; naming the missing field would only coach a scanner.

The password is never logged. `username` goes through `auth::log_username`
(truncated) and is rendered with `Debug` (`username = ?…`), which stops an
embedded newline forging a log entry. `tests/auth_logging.rs` pins both.

### Session events

Session lifecycle is logged under `pingward::session` rather than `audit_log`
(which models "an admin acted on a target" and has no ip/user agent; these are
high-volume, per-request), so it can be silenced independently
(`RUST_LOG=info,pingward::session=warn`). A session is identified only by
`handle` — `auth::session_log_handle`, the `/account` SHA-256 handle cut to 16
hex chars — **never the raw id**. Bulk teardowns log `count` instead.

- `session.created` (`web::open_session`, the single mint point for setup,
  login and forward auth) — `handle`, `user_id`, `sso`, `ip`, `user_agent`,
  `expires_at`.
- `session.renewed` (`Store::find_session_user`) — the same plus `renewal`
  (`auth::RenewalKind`): `slid`, or `clamped` when a too-long window was pulled
  back. A burst of `clamped` is a deployment signal (stale writer, lowered TTL),
  not user activity.
- `session.destroyed`, by `reason`: `logout`, `revoked`
  (`handle`/`user_id`/`is_current`), `revoke_others` (`user_id`/`count`),
  `password_change` (`user_id`/`count`), `password_reset` and `user_disabled`
  (`user_id`/`count`/`actor_user_id`), `user_deleted`
  (`user_id`/`actor_user_id`; rows go by `ON DELETE CASCADE`, so no count), and
  `expired` (one aggregate `count` per prune pass).

### Extractors and scoping

- `CurrentUser` — 401/redirect to `/login` without a user.
- `OptionalUser` — `None` instead.
- `AdminUser` — also requires `is_admin`, else 403.

`owned_project`/`owned_check` return **404, not 403**, for another user's
resource, hiding its existence.

Neither `/admin*` nor `/api/v1` has a router-level guard: every handler
extracts `AdminUser`/`ApiUser` itself, before any body extractor, so the guard
rejects before the body is parsed. Tests derive the route list from the router
source (`tests/common::routes_in_router_source`), so a new route that forgets
its guard fails with no table to update:
`tests/admin.rs::non_admin_forbidden_on_every_admin_route` and
`tests/api_v1.rs::every_api_v1_route_requires_a_bearer_key` (`/api/docs` and
`/api/openapi.json` are session-gated and fall outside the `/api/v1` prefix).

`/api/v1` scoping goes through `resolve_project`/`resolve_check`/
`resolve_channel` in `src/api/v1.rs`: owner, else audited admin access
(`admin.api.access`), else 404. Ownership is tested exhaustively and
**two-sided** — non-owner gets 404 *and* owner gets non-404 on the same id,
since a nonexistent id also 404s and a one-sided test would pass vacuously:
`tests/api_v1.rs::member_cannot_reach_another_users_resource_on_any_api_route`
and `tests/web_ownership.rs::member_cannot_reach_another_users_resource_on_any_web_route`
(which excludes `/admin*` and `/account/*`).

### Reading vs. disclosing under `/admin`

Auditing every admin page view buried the entries that mattered, so
`web::audits_as_mutation` gates the three cross-user resolvers
(`admin_project`/`admin_check`/`admin_channel`) on request method. The gate is
in the resolvers because they are the choke point for reads *and* writes;
dropping the read audit anywhere else would drop pause/resume/delete/regenerate
audits with it.

The exception is the ping URL, a bearer credential. The admin check page
withholds it behind `POST /admin/checks/{id}/ping-url`, which records
`admin.ping_url_reveal` — a POST, so the disclosure cannot bypass the handler
that records it — and withholds the usage help (which repeats the URL) with it.
`web::CheckPageViewer` carries the decision so route prefix and URL visibility
cannot contradict each other. An admin viewing their own check is not gated.
The REST API needs no equivalent: `CheckDto` includes `ping_uuid`, and
`admin.api.access` already records cross-user reads.

## Background loops

`main.rs` spawns two tasks, after building `AppState` so both loops and the
server share `state.events`:

- `scheduler::run_scan_loop` — every `PINGWARD_SCAN_INTERVAL` (default 30s),
  downs overdue/overrun checks, then runs `nag_once`; notifications are
  delivered in spawned tasks.
- `prune::run_prune_loop` — every `PINGWARD_PRUNE_INTERVAL_SECS` (default 1h),
  applies the retention settings and deletes expired sessions.

Scan and nag intervals cascade check → project → global setting → env
(`effective_scan_interval`/`effective_nag_interval`); non-positive or unset
falls through. Nag has no env default — off unless a level opts in.

## Graceful shutdown

`src/shutdown.rs` is one `watch<bool>` flag behind `(ShutdownTx, Shutdown)`,
shared by the server and both loops and raised on the first SIGTERM/SIGINT
(`os_signal`). Dropping `ShutdownTx` also counts as a request.

**The handler is mandatory.** The image's exec-form `ENTRYPOINT ["/pingward"]`
has no init shim, so pingward is PID 1, and Linux discards default-disposition
signals to PID 1 — without a handler `docker compose down` waits 10s and
SIGKILLs.

The drain order matters:

1. `with_graceful_shutdown` stops accepting and finishes in-flight requests,
   bounded by `HTTP_DRAIN_TIMEOUT` (3s): an open SSE stream only ends on client
   disconnect, and axum's drain has no timeout of its own.
2. Each loop returns from its `select!` at the sleep; an in-flight pass
   completes.
3. `main` **joins** both handles, so no loop query is outstanding when the pool
   closes (`PoolClosed` otherwise).
4. `store.pool.close()` bounded by `POOL_CLOSE_TIMEOUT` (5s; with the drain,
   inside Docker's 10s), since fire-and-forget `deliver_event` tasks may still hold
   connections.

For SQLite, closing the last connection checkpoints the WAL and removes the
`-wal`/`-shm` sidecars
(`db::tests::closing_the_pool_checkpoints_and_removes_wal_sidecars`); SIGKILL
left them to be replayed on every start.

## Live-tail signal bus (SSE)

`AppState::events` is a `broadcast::Sender<i64>` (capacity 256) carrying a
`check_id` whenever that check changes. Producers, each gated on
`receiver_count() > 0` so it is free when unwatched:

- `ping::apply` — after every `insert_ping` (all kinds, including `Log`), before
  the paused-check early return, so paused checks still publish.
- `scheduler::run_scan_loop` — for each `Down` event from `scan_once`.

`GET /checks/{id}/events` and `GET /admin/checks/{id}/events`
(`web::sse_for_check`) stream a bare `"changed"`, **never data**: the browser
re-fetches the existing pings fragment, keeping rendering, filtering and
authorization in one tested path. Ownership is resolved before the stream is
built, so a non-owner gets an immediate 404.

On the check page it is **opt-in** behind a LIVE toggle: browsers allow ~6
HTTP/1.1 connections per origin, and one EventSource per tab would starve the
app. Each event debounces ~500ms, then fetches the newest unfiltered page; the
pager and filter are hidden while live (`.card.live-on`).

A lagged subscriber gets one more `"changed"` rather than a drop — a dropped
signal would leave the page stale indefinitely, while an extra refresh is
harmless.

**Known limitation:** in-process only. With multiple replicas on shared
Postgres, a tab on replica A never hears about a ping on B. SQLite has no
`LISTEN/NOTIFY`, so there is no backend-portable fix; a reload catches up.

## Notifications

`notify::Notifier` has six implementations: webhook, Telegram, Slack, ntfy,
Pushover, email (SMTP). `notifier_for` builds one from a `Channel`, logging and
returning `None` on bad config. `deliver_event` delivers to a check's bound
channels under a `RetryPolicy` (default 3 attempts, backoff from 500ms). Both
`run_scan_loop` and `ping::apply` spawn delivery, so it never blocks a scan or
a ping response.

A check with no bound channel drops its alerts (only a `debug!`), so check
creation auto-binds every channel on the project
(`Store::bind_all_project_channels`, one `INSERT … SELECT … ON CONFLICT DO
NOTHING`), from both `web::check_create_core` and `api::v1::create_check`.
Existing checks are untouched. Checks still unbound get a "no channel" chip
(`Store::checks_with_channels`), and a project with no channels shows a
warning.

### What a notification says

Every text channel renders `notify::event_text`, capped at **four lines** —
headline, context, reason, link:

```
🔴 DOWN — nightly-backup
Project: infra · every 5m (grace 1m)
No ping since 2026-07-29 17:03 CST (1h5m ago)
https://pingward.example.com/checks/42
```

Anything more belongs on the linked page. `event_title` (ntfy/Pushover title,
email subject) is one line: `pingward: infra/nightly-backup is DOWN`.

Lines after the headline come from `notify::EventDetail`, whose fields are all
`Option`: a failed lookup or unset `PINGWARD_BASE_URL` drops a line, not the
notification. `EventDetail::default()` gives the bare one-liner (channel test).

- **Built at the call site, not during delivery.** An `Up` event must report
  the ping *before* the recovery; `ping::apply` holds that pre-update snapshot,
  and a re-read in `deliver_event` would see the recovery ping.
- **Only the caller knows the cause.** `scan_once` sets `DownCause::Overdue`,
  or `Overrun` when a run exceeded `max_runtime_secs` (preferred as more
  specific); `ping::apply` sets `Failed { exit_code }`; `nag_once` sets none, so
  a reminder says "Last ping …" rather than wrongly "No ping since …" for a
  check downed by `/fail`.

Timestamps use `fmt_at` (`%Y-%m-%d %H:%M %Z`) plus a `duration::fmt_duration`
relative suffix. Zone: the instance `display_timezone` setting if set
(`EventDetail::with_display_timezone`), else the check's zone, else UTC. The
setting exists because the web UI localises every time to the *viewer* (via
`app.js`), but a notification has no browser; the check's own zone describes
when the **cron** schedule fires, not who reads the alert.
`Store::display_timezone` swallows read errors so a settings failure never
blocks a down alert.

Per-channel extras: ntfy `Click` (only if header-safe — an invalid
`HeaderValue` aborts the send), Pushover `url`/`url_title`, and webhook keys
`check_id`/`project`/`url`/`schedule`/`timezone`/`last_ping_at`/`cause`/
`exit_code`/`text`, added **strictly additively** to the original `check`,
`event`, `at`, `project_id`.

Project names cost one query: `Store::all_project_names()` once per scan/nag
pass, and inside the spawned delivery task in `ping::apply` so it stays off the
response path.

### Editing a channel without leaking its secrets

`channels.config_json` is plaintext JSON holding delivery credentials
(`ChannelDto` omits it). The rule is **never re-render a stored secret**:

- One merge rule, `web::validate_channel_update(form, Option<&Channel>)`: a
  blank field keeps the stored value. `validate_channel` is the `None` case, so
  create and edit share the per-kind required-field checks.
- Secrets render empty with `placeholder="unchanged"` and a
  `configured`/`not set` pill. The template sees only `web::ChannelEditView`
  (non-secret values + `has_*: bool`), so non-leakage is a type property.
- Webhook/Slack **URLs are secrets** (capability to post); Telegram chat id,
  ntfy server/topic and email recipients are identifiers and are pre-filled.
- `ntfy_token_clear` is the one explicit clear, since blank-means-unchanged
  could never remove the only optional secret.
- **`kind` is immutable**: a config only means something for the kind that
  wrote it. A submitted `kind` is ignored; `Store::update_channel` takes none.
- `PATCH /api/v1/channels/{id}` shares the validator, so it is a *merge* (unlike
  PATCH on projects/checks) — a client cannot re-send secrets it never got.
- Other channel lists use projections too (`web::ProjectChannelRow`), so
  `config_json` never enters a render context by construction.

At-rest encryption of `config_json` is deliberately deferred.

## Swappable history sections

The check page's pings and notifications and `/admin`'s audit trail each use one
template (`check_pings.html`, `check_notifs.html`, `admin_audit.html`) — filter
form, table, `Newer`/`Older` keyset pager — rendered both inline in the page and
standalone by a fragment endpoint (`GET /checks/{id}/pings`,
`…/notifications`, `GET /admin/audit`), so a partial refresh cannot drift from
a full load.

- **A navigation to a fragment endpoint redirects** (`web::wants_fragment`,
  `web::fragment_page_redirect`): without `X-Requested-With: fetch` it
  redirects to the embedding page with the same query and an anchor, instead of
  serving an unstyled partial. Ownership is resolved **before** that decision,
  so the redirect cannot confirm someone else's check exists.
- **Filters are real GET forms** to the embedding page (`/checks/{id}`,
  `/admin`) with named controls and a submit button; the page parses the same
  query struct. With JS, `data-apply` swaps the fragment instead and
  `data-nosubmit` stops a stray Enter.
- A GET submit replaces the whole query, so each form re-sends the sibling
  section's filter as hidden inputs (`web::carry_fields`) and each Clear link
  keeps it (`web::clear_href`). Clear targets the fragment endpoint: JS expects
  a partial, and without JS it redirects.
- `datetime-local` values are local time with JS (converted to UTC before
  fetch) and read as UTC without it (`web::parse_date_bound`) — each matching
  what that mode displays.

Client side: `pw.wireSection` in `assets/app.js` delegates clicks in the
section, fetches the fragment, swaps it in, and re-runs localisation, row
toggles and date filling; the admin audit card reuses it.

Server side: `store::keyset_page`, shared by all three, pages by `id`
(monotonic, indexed, stable under concurrent inserts, unlike offsets) with a
limit+1 fetch for `has_newer`/`has_older`. `pings`/`notifications` scope by
`("check_id", id)`; `audit_log` passes `None`. Filter values are always bound;
only self-generated literals are interpolated.

## Templates & assets

Askama compiles `templates/*.html` into the binary — **run `cargo build` after
any template or route change**. The E2E harness runs `target/debug/pingward`
and only builds it when missing (`e2e/src/server.rs::ensure_binary`), so a
stale binary is silently reused. Interactive elements carry `data-testid`,
used by the integration tests and the E2E steps.

## Testing

Integration tests live in `tests/`, one file per area; run with
`cargo nextest run` (CI does not use `cargo test`). SQLite tests always run.
`tests/pg_store.rs` skips without `TEST_DATABASE_URL=postgres://…`;
`tests/smtp_e2e.rs` skips without `PINGWARD_TEST_SMTP_HOST` (plus
`PINGWARD_TEST_SMTP_PORT`, `PINGWARD_TEST_MAILPIT_API`). `docker compose up -d`
starts both backends.

`e2e/` is a cucumber + thirtyfour harness — `.feature` files plus steps in
`e2e/tests/e2e/steps/`, run with `cd e2e && cargo test --test e2e`. It is its
own workspace, so root `--workspace` runs never drive a browser. Each scenario
gets a fresh binary and temp SQLite DB on a random port, because `POST /setup`
creates the first admin once and nearly every scenario walks through it.

`@nojs` scenarios (`no_js.feature`) run in a session opened with
`Emulation.setScriptExecutionDisabled`, which applies to the next document, so
sessions are per-scenario. Without it the suite cannot see anything the UI has
quietly come to depend on `app.js` for. Checks that matter in both directions
(open without script, collapsed with it) are paired across `no_js.feature` and
the JS-on suite, so an always-open "fix" cannot pass alone.

## How to make common changes

- **DB column/table**: migration SQL in **both** `migrations/sqlite/` and
  `migrations/postgres/`, then the `models.rs` struct and `Store` methods
  (`$N` placeholders).
- **Enum variant**: extend the `str_enum!` invocation in `models.rs`.
- **Notifier**: implement `Notifier` in `notify.rs`, add a `ChannelKind`
  variant, wire it into `notifier_for`.
- **Route**: register it in `web::routes()`, `ping::routes()` or
  `api::routes()`. Under `/admin*`, extract `AdminUser`; under `/api/v1`,
  `ApiUser` — before any body extractor. The route-derived tests pick it up
  automatically.
