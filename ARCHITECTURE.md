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
| `e2e/`                 | cucumber + thirtyfour browser tests (`.feature` + Rust steps), its own workspace |

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
- `src/ratelimit.rs` — `RateLimiter`, the in-memory per-client-IP fixed-window
  limiter guarding `POST /login`, and `rate_limit_key`, which resolves the
  bucket key from the trusted-proxy-gated **rightmost** `X-Forwarded-For` hop
  (deliberately the opposite end from `auth::client_ip`'s leftmost).
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
- `src/prune.rs` — `prune_once` (deletes old pings/notifications/audit rows
  per retention setting, plus expired sessions; returns a named
  `PruneCounts`) and `run_prune_loop`. All three retention settings default
  to off; `audit_retention_days` stays off deliberately, since a default that
  began deleting the audit trail on upgrade would destroy exactly the record
  it exists to keep.
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
  never masks an alert. `next_due` renders the check header's countdown to
  the next deadline, and derives it from `scheduler::due_time` rather than
  the stored `checks.next_due_at` column — that column is only ever stamped
  by `ping::apply`, so it is `NULL` for a check that has never pinged and for
  one downed by a `fail` ping, while `due_time` is what `scan_once` itself
  evaluates to decide a check is overdue. The deadline includes grace, hence
  "due" rather than "expected"; a paused check is excluded from monitoring
  and so shows no deadline at all.
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
    .merge(web::routes()
        .layer(csrf_guard)          // innermost
        .layer(anonymous_session)
        .layer(forward_auth_session)
        .layer(no_store)
        .layer(content_security_policy)) // outermost of the `web` layers
    .merge(ping::routes())
    .merge(api::routes())
    .merge(assets::routes())
    .layer(hsts)                        // app-wide, not `web`-only
    .layer(security_headers)            // outermost overall
    .with_state(state)
```

Only `web::routes()` carries those layers. Because the other routers are
merged as *siblings* rather than nested under it, `/ping/*` (machine
endpoints, no session), `/api/v1/*` (bearer-authenticated, never reads the
session cookie), and the static asset/`/healthz` routes are **structurally**
exempt from CSRF — there's no way for a change inside `web::routes()` to
accidentally start covering them. `csrf_guard` itself lets safe methods
(GET/HEAD/OPTIONS) through and otherwise requires a per-session synchronizer
token sent as `X-CSRF-Token` (or hidden form field).

`no_store` sets `Cache-Control: no-store` on every response that doesn't
already carry one, so authenticated pages and the `/login`/`/setup` forms
(which embed a cookie-bound `_csrf`) are never cached by the browser, a
shared computer, or an intermediary proxy. It sits **outermost** of the four
`web` layers — but unlike the other three, that position is not about request
ordering: `no_store` only reads and writes response headers on the way out,
so it never observes or affects the session/CSRF request-handling chain
described below. It runs outermost purely so it wraps every early-return
path too, including `csrf_guard`'s 403s. `/assets/*`, `/ping/*`, and
`/healthz` are sibling routers and stay structurally exempt — see
`src/assets.rs`'s `IMMUTABLE_CACHE`, unchanged by this layer. `/api/*` is
exempt the same structural way, but not uniformly: `api::routes()` layers
`no_store` a second time, scoped to just `/api/docs` and `/api/openapi.json`
— those two additionally accept a logged-in web session (`CurrentUser`)
alongside `/api/v1`'s bearer auth, so they are session-authenticated
responses and need the same protection. `/api/v1` stays exempt on purpose: it
is bearer-authenticated, was never going to carry a browser-cacheable session
in the first place, and adding response headers there would affect API
consumers for no benefit.

`content_security_policy` is `web`-scoped for the same reason `no_store` is:
it describes pages this app renders. Its `script-src` is `'self'` alone — no
`'unsafe-inline'`, no nonce — and that is only true because **every script the
UI runs is a file under `/assets`** (`assets/app.js`, plus the tiny
render-blocking `assets/theme-init.js` that resolves the theme before first
paint) and **no template carries an `onclick=`/`onsubmit=` attribute**. Row
navigation (`data-href`), confirmation prompts (`data-confirm`) and the
non-submitting filter forms (`data-nosubmit`) go through delegated handlers in
`app.js` instead, which also means they keep working on markup inserted by a
fragment swap. `data-href` is a **mouse convenience layered over a real
link**, not the link itself: the name inside each row is an `<a>` to the same
destination, which is what carries keyboard access, the focus ring,
middle-click and — the reason it exists — the row working at all with JS off.
It replaced a `tabindex="0" role="link"` div that simulated the first two and
delivered none of the rest, leaving the dashboard with no route to a check
page for an unscripted browser. `tests/no_js.rs` is the guard.

CSS that hides content a click would reveal has the same problem, and hangs
off a **`js` class on `<html>`** set by the render-blocking `theme-init.js`.
The expandable ping/audit panels are collapsed by `:root.js tr.exp:not(.open)`
alone, so with no script they stay open and a failed job's captured output —
the most useful thing on the page when something has broken — is still
readable rather than sealed behind a caret nothing can open. The carets and
the row's pointer cursor are suppressed in the same state, since both
advertise an affordance that is not there. The class is set in
`theme-init.js` rather than `app.js` for the reason that file exists at all:
`app.js` is deferred, so collapsing from there would show every panel and then
snap them shut after the first paint.

The **theme** is the same story told with `data-theme`. `theme-init.js`
resolves the stored `light | dark | system` preference into that attribute
before the first paint; `base.html` deliberately does **not** carry a default
value, because a hardcoded one is precisely what a scriptless browser would be
stuck with — it was `dark`, so every such visitor got a dark page whatever
their OS asked for, even though `prefers-color-scheme` answers that question
with no script at all. With the attribute absent, `app.css`'s
`@media (prefers-color-scheme: light) { :root:not([data-theme]) { … } }` gets
to apply. `:not([data-theme])` is what keeps it inert once the script has run,
since `theme-init.js` always sets the attribute and never leaves it off, so
the media block can never fight the toggle.

That palette is written **twice** — a selector list cannot span a media query
boundary, and `light-dark()` would mean rewriting all thirty tokens for a
visual-drift risk not worth taking here. `tests/no_js.rs` compares the two
blocks token by token, because a value tuned in one copy only is invisible
until someone opens the app with script off.

Three controls are pure `app.js` and are **removed** rather than left to be
clicked at: the clipboard `.copy` buttons, the live-tail toggle, and the theme
cycle (`:root:not(.js)`, `display: none`). Nothing is lost with them — the ping
URL and API token sit in a `code` element that selects by hand, the live tail
shows rows a reload also shows, and the theme now follows the OS by itself. The
same rule of thumb applies to anything added later: a control whose behaviour
lives entirely in a script should not be drawn when the script is absent. The
one exception is content, not a control — `/admin`'s heartbeat age, which the
handler now renders server-side (`web::relative_setting`) and `app.js` merely
keeps ticking, since a blank line is worse than a slightly stale one.

Two more places used to depend on script and no longer do:

- The check form's period-vs-cron fields and the channel form's six per-kind
  config blocks are switched by `:has()` rules in `app.css` — `:checked`
  follows the select and the rules re-evaluate, so the interaction is identical
  with or without script. It was a `sync()` setting inline `style.display`,
  which meant an unscripted form showed every branch at once. The script half
  is **deleted** rather than kept alongside: an inline style outranks a
  stylesheet rule, so two mechanisms could only disagree.
- Every absolute timestamp's plain-text fallback goes through `view::fmt_utc`
  (`2026-08-13 17:54:27 UTC`). It is what stays on the page when `app.js` is
  not there to localize the `.localtime[data-ts]` span, so it is text a person
  reads: the zone spelled out, no `T` separator, no sub-second digits. The
  history tables always did this; `/account` and `/admin` were falling back to
  chrono's `Display` or to the raw RFC3339 string.

There is deliberately **no `<noscript>` banner**. One would have been worth
having when a scriptless visit hit a dashboard that led nowhere and pager links
that rendered a bare fragment — it would at least have named the cause. With
those fixed there is nothing left for it to explain: what remains is a live
tail that is not offered, times in UTC rather than local, and no clipboard
button, none of which reads as breakage.

`data-confirm` is the same shape of promise, and is likewise **not** what
enforces anything. The attribute only turns into a question if `app.js` is
running, so the server refuses every irreversible action that does not carry
`?confirmed=1` and renders `templates/confirm.html` — the same question as a
page — instead of doing the work. `app.js` appends the flag once its dialog is
accepted, so a scripted browser still asks in place and still posts once, and
neither side trusts the other: the page cannot skip the flag and the server
never assumes a dialog ran. See "Confirming a destructive action" below.

Reintroducing one inline handler means either weakening the
policy for the whole UI or minting a nonce per response, so don't: put the
behaviour in `app.js` behind a `data-` attribute. `style-src` does keep
`'unsafe-inline'` — the heartbeat bars carry a computed
`style="height:Npx"` — which is a deliberate, much narrower concession.

`/api/docs` is deliberately outside this layer: the Scalar reference loads its
bundle from `cdn.jsdelivr.net`, so `script-src 'self'` would render it blank,
and widening the policy app-wide to admit one CDN would cost every other page
the guarantee above. It is still covered by `security_headers` below.

`security_headers` is app-wide (`X-Content-Type-Options: nosniff`,
`X-Frame-Options: DENY`, `Referrer-Policy: same-origin`, an empty
`Permissions-Policy` allowlist). Unlike the CSP these say nothing about
rendered markup, so they belong on the routers the CSP skips too: nosniff
matters most for `/api/v1`'s JSON and a captured ping body served as
`text/plain`, and `X-Frame-Options` is what keeps `/api/docs` unframable
without a CSP. Each header is only filled in when the response does not
already carry it, so a handler can still override.

`hsts` (`web::hsts`, gated by `PINGWARD_HSTS_MAX_AGE`) is layered outside
every `.merge(...)` in the block above, not inside the `web` router the way
`no_store` is. `no_store` is a browser-page-caching concern scoped to
`web::routes()` — the pages that render a cookie-bound `_csrf`. HSTS instead
tells the browser the whole *origin* is HTTPS-only, so it has to cover
`/healthz`, `/ping/*`, `/api/*`, and static assets too, none of which
`no_store` touches. Like `no_store`, it is a no-op response-only layer (it
defaults off — see `web::hsts`'s doc comment for why sending it
unconditionally would be wrong on a plain-HTTP internal deployment), so its
position relative to the session/CSRF request-ordering chain below does not
matter; it is placed outermost purely to cover every response, including
early returns from the layers nested inside `web`.

### Session layers

Both orderings among the session/CSRF layers below are load-bearing.

The two session layers run **before** `csrf_guard` because a cookie minted
during a request has to be visible to the guard on that same request — each
layer therefore rewrites the request's `Cookie` header as well as setting
`Set-Cookie` on the response, so a handler rendering a form can derive the
matching token immediately.

`forward_auth_session` runs **before** `anonymous_session` because when both
would mint, the real session must win; reversed, the anonymous layer's
`Set-Cookie` would be appended last and shadow it.

- **`anonymous_session`** gives every visitor a signed cookie, logged in or
  not — but writes **no `sessions` row**. The CSRF token is derived from a
  session id, not looked up, so an id alone is enough (see below). This is why
  `csrf_guard` needs no path exemptions: `/login` and `/setup` render a real
  token and are CSRF-protected like everything else. `auth::resolve_user`
  needs no special case either — an anonymous id matches no row, so the
  visitor stays anonymous. Logging in rotates to a fresh id, so an
  attacker-planted anonymous cookie cannot survive into an authenticated
  session.
- **`forward_auth_session`** turns a trusted `PINGWARD_FORWARD_AUTH_HEADER`
  identity into a real session row plus cookie. Without it such a user is
  authenticated but session-less, and everything keyed off the session
  degrades silently — forms render an empty `_csrf`, every POST is rejected
  with 403, and the account page lists no session to review or revoke. It
  short-circuits before any database work when forward-auth is unconfigured or
  the request already carries a live session. That liveness check is
  deliberately a lookup rather than a bare signature check: with
  `anonymous_session` in play, a valid signature no longer implies a row
  exists. `create_session` stamps these rows with `sso = true` (a plain
  password/setup login stamps `false`), and `/account` renders an "SSO" badge
  next to any session created this way.

The same layer is why **logout cannot be local-only** under forward auth:
`web::logout` deletes the row, but the next request still carries the gateway's
header and `forward_auth_session` mints a replacement before any handler runs.
Two things follow from that, both in `web.rs`. `logout` redirects to
`PINGWARD_FORWARD_AUTH_LOGOUT_URL` when configured — the gateway's own sign-out
endpoint, the only thing that can end the identity; the target comes from the
operator's environment and never from the request, so it is not an open
redirect. With **no** logout URL configured, `logout` looks at whether the
trusted proxy identity header is present on the logout request itself (the same
`auth::forward_auth_username` gate, fed the socket peer via the `PeerAddr`
extractor): if it is, a local logout is provably a no-op, so instead of bouncing
to `/login` and silently re-authenticating, it lands on the dashboard carrying a
one-shot `forward_auth_logout` flash (`take_flash`/`flash_cookie`, consumed only
by `dashboard`) that tells the visitor only their proxy/SSO provider can end the
session. A password user's logout — no identity header on the request — still
redirects to `/login` as before. `login_page` likewise bounces an
already-authenticated visitor to `/`, because rendering a login form to someone
the layer has just signed back in would be dishonest.

The `/login`- and gateway-URL exits also send `Clear-Site-Data: "cache"`
(`web::CLEAR_SITE_DATA`) so the browser drops this origin's cache on logout.
`"cookies"` is deliberately not included: that directive is scoped to the
whole *registered domain*, including subdomains, not just this origin — on
the SSO layout `PINGWARD_FORWARD_AUTH_LOGOUT_URL` is meant for (pingward and
its gateway as sibling subdomains of the same parent domain), sending it would
clear the gateway's own session cookie before the browser even follows the
redirect, breaking the logout handoff and signing the user out of every other
app on the domain too. The session cookie itself is already ended by the
removal `Set-Cookie`, which *is* origin- and path-scoped, so nothing is lost
by leaving "cookies" out. The flash exit omits the header entirely: it is not
a credential teardown at all (the gateway re-mints the session on the very
next request regardless), and its whole purpose is to carry the flash cookie
(`web::flash_cookie_name`) to `/`. `"storage"` is never sent on any exit — it
would wipe the `pw-theme` localStorage preference (`templates/base.html`) for
no security benefit, since pingward keeps nothing secret in localStorage.
Browsers only honour `Clear-Site-Data` on a trustworthy origin, so on a
plain-HTTP deployment the header is sent but silently ignored — a no-op, not
a gap.

`/api/v1` data endpoints authenticate independently via the `ApiUser` bearer
extractor; `/api/docs` and `/api/openapi.json` additionally accept a logged-in
web session (`CurrentUser`) but are read-only `GET`s, so they add no
CSRF-relevant ambient authority.

### Session and CSRF secret

Both browser credentials are keyed off one process secret (`src/secret.rs`,
`PINGWARD_SECRET`):

```
session cookie = <session_id>.<HMAC-SHA256(secret, "session:" ++ session_id)>
CSRF token     =                HMAC-SHA256(secret, "csrf:"    ++ session_id)
flash cookie   = <payload>.<HMAC-SHA256(secret, "flash:"   ++ payload)>
```

The domain-separation prefixes are load-bearing: without them the two values
are identical, and every rendered form embeds the CSRF token — which would
print the cookie's signature into the page body.

Two consequences follow from deriving rather than storing:

- **`sessions` has no `csrf_token` column.** Rendering a form and checking a
  submitted token are both pure computation, so neither costs a query — and a
  session id needs no row behind it to carry a working token, which is what
  makes the row-free `anonymous_session` layer possible.
- **The cookie is verified before any database work.** A forged, stale, or
  DB-leaked `sessions.id` fails the signature check in `secret::verify_session`
  and never reaches a lookup. The raw cookie value is therefore *not* the
  session id — every consumer must go through `secret::session_id_from_jar`.

Rotating the secret invalidates every signature at once, ending all browser
sessions while leaving the rows intact (the prune loop reaps them on expiry).
When `PINGWARD_SECRET` is unset a random secret is generated per process, so
**every restart signs everyone out**; `main::warn_on_ephemeral_secret` logs a
warning saying so at startup. API keys are unaffected either way — they are
random bearer tokens matched by SHA-256 digest (`src/apikey.rs`) and never
touch this secret.

### Cookie attributes

Every session and flash cookie is built by exactly one pair of functions
(`web::session_cookie`/`session_removal_cookie`, `web::flash_cookie`/
`flash_removal_cookie`), so their attributes cannot drift between the
mint and removal paths:

- `HttpOnly` and `SameSite=Lax` always.
- `Path=/`, so every route sees (and can clear) the cookie.
- `Secure` follows `Config::cookie_secure` (`config::parse_cookie_secure`):
  an explicit `PINGWARD_COOKIE_SECURE` wins, otherwise it is derived from
  whether `PINGWARD_BASE_URL` starts with `https://`. It is not hardcoded —
  a browser silently drops a `Secure` cookie sent over plain HTTP, which
  would otherwise break login on a plaintext self-hosted LAN deployment.
- **No `Max-Age`/`Expires`.** This is deliberate, not an oversight: OWASP's
  session-management guidance prefers a non-persistent session cookie, so
  expiry is enforced only server-side (`sessions.expires_at`), not by the
  browser. Do not add one when touching this code.
- A removal cookie's attributes must match the corresponding mint function
  exactly (`session_cookie`/`session_removal_cookie`, `flash_cookie`/
  `flash_removal_cookie`) — RFC 6265bis §5.5 ("Leave Secure Cookies Alone")
  means a mismatched removal cookie can fail to clear the original in some
  browsers.
- **The session cookie's name is itself conditional on `Secure`**
  (`auth::session_cookie_name`): `__Host-pingward_session` when
  `cookie_secure` is true, the plain `pingward_session` otherwise. The
  `__Host-` prefix is a browser-enforced guarantee, layered on top of the
  server-side attributes above — the browser itself refuses to store or send
  such a cookie unless it also carries `Secure`, `Path=/`, and no `Domain`,
  so it cannot be overwritten by a sibling subdomain or by a response
  downgraded to plain HTTP. It is applied conditionally, never
  unconditionally: on a plaintext deployment a browser would refuse a
  `__Host-` cookie outright, turning login into a silent failure. The flash
  cookie follows the same conditional pairing (`web::flash_cookie_name`:
  `__Host-pingward_flash` / `pingward_flash`) **and** carries an HMAC over
  its payload (`secret::sign_flash`, verified by `web::flash_payload`). The
  cookie holds no authority — its value is either a fixed key mapped to a
  fixed message, or (for `password_reset_keys:<revoked>:<keys>`) a pair of
  `u64`-parsed counts that Askama escapes on render, so a forged value can
  neither elevate nor inject markup — but authority is not the only thing
  worth protecting: without these two, a response from a sibling subdomain
  could plant a flash this origin never set, and the reader would see a
  message the server never sent (a fabricated "N API keys still work" count
  on an admin's own page). The prefix stops a sibling writing the cookie at
  all under HTTPS; the signature covers the plain-HTTP deployment, where no
  prefix is available. The removal cookie's value is left empty and unsigned
  on purpose — removal rides on the attributes, and an unsigned value fails
  verification on read anyway. Every read
  of the session cookie goes through
  `secret::session_id_from_jar(jar, secret, cookie_name)`, which now takes
  the resolved name as a parameter rather than a hardcoded constant, so the
  read and write sides cannot drift apart.

  **Flipping `PINGWARD_COOKIE_SECURE` or the scheme of `PINGWARD_BASE_URL`
  changes the cookie name and therefore signs everyone out once** — the
  browser's existing cookie is under the old name and is simply no longer
  read. There is precedent for this: `0012_session_secret.sql` already does
  the equivalent (`DELETE FROM sessions`, "Everyone signs in again once, on
  upgrade"). No read-side fallback to the other name is implemented on
  purpose — it would add a permanent branch and a "when do we remove this?"
  question for a one-time, self-healing inconvenience.

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
`list_checks_for_project`, `list_recent_ping_summaries_for_checks` beside
`list_recent_ping_summaries` — each building an `IN ($1,…,$N)` list and
returning a `HashMap` keyed by the parent id (parents with no children are
simply absent).

The heartbeat pair is also **narrowed**, not just batched: it selects
`id, check_id, kind, created_at` into a `models::PingSummary` instead of whole
`Ping` rows. `view::heartbeat` and `view::run_durations` read nothing else, so
the wide form spent the dashboard's time decoding captured POST bodies (up to
`ping::MAX_BODY`, 10 KiB per row, 40 rows per check) that were dropped on the
next line. #116 has the measurement that motivated it — `GET /` was 24-74%
slower with the wide form across check counts and body sizes, and the
per-row decode alone (four columns rather than seven, no `String` for
body/source_ip) accounts for the low end, so the win is not confined to
instances that capture large bodies.
The two callers want different amounts of it. The dashboard's strip is six
bars in a narrow column, so it reads a 40-row window. The check page's strip
is the width of the card, and **how many bars fit is a question only the
browser can answer** — so the server renders past the widest possible strip
(`web::HEARTBEAT_BARS`, 120, over a `web::HEARTBEAT_WINDOW` of 300 rows) and
`assets/app.css`'s `.beat` clips the overflow from the *left*
(`justify-content: flex-end` + `overflow: hidden`), keeping the newest run
pinned to the right edge. A phone shows ~34 bars and a desktop ~100, with no
JS, no media query and no second round trip; the ceiling is finite because
`.wrap` caps the page at 1080px, so the strip can never exceed ~100 bars.
Two consequences worth knowing before changing either number: the window is
**not** filtered to `kind IN ('success','fail')`, because `run_durations`
pairs each finish ping with the `start` before it and dropping the starts
would flatten every bar; and no caption may name a bar count, since the server
does not know how many are visible.

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

Sessions are a `session_cookie_name(cookie_secure)`-named cookie — plain
`pingward_session`, or `__Host-pingward_session` when `PINGWARD_COOKIE_SECURE`
is on — plus an argon2 password hash (`src/auth.rs`). An optional trusted
forward-auth header
(`PINGWARD_FORWARD_AUTH_HEADER` + `PINGWARD_TRUSTED_PROXIES`) can
auto-provision a passwordless, non-admin user on first sight, but only when
the request's peer IP is a configured trusted proxy.

Session expiry is two independent layers, not one:

- **Idle timeout** — `sessions.expires_at` means "last activity +
  `SESSION_IDLE_TTL_HOURS`" (72h) and slides forward on use
  (`auth::refreshed_expiry`, applied in `Store::find_session_user`). It is
  checked in SQL (`WHERE s.expires_at > $2`).
- **Absolute cap** — `SESSION_ABSOLUTE_MAX_DAYS` (30, unchanged from the old
  single-layer TTL) measured from `created_at`; no amount of activity extends
  it. This is enforced in Rust (`auth::is_past_absolute_cap`), not SQL,
  because `created_at` can be `''` on a pre-`0010` row (`parse_ts` yields
  `None` for that), and `''` sorts below every RFC3339 string — a SQL
  predicate over it would misjudge those rows as infinitely old. `Store`
  applies the same check in `list_sessions_for_user` (filtered after mapping)
  so a session already refused by `find_session_user` never appears on
  `/account`, and in `delete_expired_sessions` (an extra `OR` clause,
  excluding `created_at = ''` explicitly) so prune reclaims rows that die from
  either layer. Hiding such a row is not the same as removing it, though — an
  owner could see neither it nor a revoke button for it until the next prune
  pass — so `render_account` first calls
  `Store::delete_capped_sessions_for_user` (scoped to the caller, same
  `created_at <> ''` exclusion): opening `/account` reaps the caller's own
  past-cap rows, which is what makes "not listed" mean "gone" rather than
  "withheld". The Rust filter stays as a belt-and-braces guard for the other
  callers. `expires_at`'s write is throttled to firing only once past the
  half-life of the idle window (`refreshed_expiry`'s guard), so a hot session
  costs roughly one write per 36 hours rather than one per request;
  `last_seen_at` keeps its separate 60-second throttle (see below) since
  `/account`'s display wants finer granularity than the slide does. Upgrade
  compatibility: `refreshed_expiry` also carries a downward clamp — whenever
  the stored `expires_at` already exceeds what the idle policy would ever
  grant (`min(now + idle, cap)`), it is pulled *down* to that value on the
  very next request, bypassing the write throttle. That clamp was originally
  written to handle a pre-branch row, whose old single-layer `open_session`
  (`git show aa17ca9:src/web.rs`) produced an `expires_at == created_at + 30d`
  that would otherwise have sailed on its fixed-length expiry for weeks with
  no idle enforcement — migration `0015_invalidate_legacy_sessions.sql` now
  deletes every session predating the idle window outright, so that row shape
  cannot occur **in a single-instance deployment**: `db::migrate` runs before
  `TcpListener::bind` (`src/main.rs`), so this process cannot mint a legacy
  row ahead of its own `DELETE`. The migration cannot order *other* processes
  against the same database, though — a rolling deploy where a pre-`0015`
  binary is still serving after the new binary's migration commits, or two
  instances sharing one `DATABASE_URL` at different versions, can each still
  write such a row after the database has already been migrated. The retained
  clamp is what covers that multi-instance case, consistent with this
  document's framing elsewhere that multi-instance pingward is only
  semi-supported (see the SSE bus, which is in-process only). The clamp is
  not scoped to that scenario alone, either: it is also what protects a
  future build that *lowers* `SESSION_IDLE_TTL_HOURS` — every session minted
  under the old, longer window carries an `expires_at` the new window would
  never grant on its own, and the clamp pulls each one down to the new policy
  the first time it's read rather than letting it ride out its old expiry.
  `is_past_absolute_cap`'s independent check in `find_session_user` remains
  the backstop that still bounds the clamp itself at `cap`.

`auth::is_trusted_proxy` is the single gate for that decision, shared by
forward-auth and by `auth::client_ip` (the address stamped on a session row
and on `pings.source_ip`). A `PINGWARD_TRUSTED_PROXIES` entry is a bare
address or a **CIDR block** — the container case needs the block, since a
reverse proxy on a bridge network draws its address from a pool and a pinned
literal silently stops matching after the network is recreated. Addresses are
compared (and stored) canonically, so an IPv4-mapped IPv6 peer matches an
IPv4 entry; an unparseable entry matches nothing and DNS is never consulted.

`POST /login` is additionally guarded by **two** `ratelimit::RateLimiter`
instances (`src/ratelimit.rs`), checked in that order and both reserved
*before* the argon2 verification, so a refused attempt never pays for one.
`RateLimiter` is generic over its key precisely so the two share one
implementation: the window roll-over, the single-lock check-and-record, the
tracked-key cap and the overflow bucket each fixed a defect (below), and a
second hand-written copy would be a second chance to reintroduce one.

`AppState::login_limiter` (`RateLimiter<IpAddr>`) is 5 attempts per client IP
per 60-second window, released back on a successful login only, so a
legitimate user signing in repeatedly never exhausts the window. Its key comes
from `ratelimit::rate_limit_key`, gated by
the same `is_trusted_proxy` check but reading the **rightmost**
`X-Forwarded-For` hop rather than `client_ip`'s leftmost — under a stock
appending proxy (nginx's `$proxy_add_x_forwarded_for`, Caddy's
`reverse_proxy`) the leftmost entry is client-controlled, and keying a
security control on it would let an attacker mint a fresh bucket per request.
Like `AppState::events` (see "Live-tail signal bus" below), the limiter's
state is in-process only: a multi-replica deployment counts each replica
separately (effective budget is `5 × replicas`), and a restart resets every
counter to zero.

`AppState::account_limiter` (`RateLimiter<String>`) is the second one:
`ACCOUNT_MAX_ATTEMPTS` (10) per `ACCOUNT_WINDOW_SECS` (15 minutes) against a
single **account**, however many addresses the attempts arrive from. The
per-address limiter cannot substitute for it and cannot even see the attack it
answers — an attacker holding N addresses simply gets `5 × N` guesses at one
account, and each of them looks unremarkable to a counter keyed on origin.
OWASP's Authentication Cheat Sheet asks for the counter to be associated with
the account for exactly this reason.

Three things about it are load-bearing:

- **It is keyed on the submitted username** (`ratelimit::account_key`), not on
  a resolved `users.id`, and is charged *before* the account is looked up. An
  invented username has to consume and exhaust a budget exactly as a real one
  does; a limiter that only engaged for accounts that exist would make "this
  request was throttled" a username oracle, giving back at this layer what
  `verify_password_or_dummy` protects at the next. Only length is normalised
  (bounded, since the field is attacker-chosen and becomes a map key) — never
  case, because `find_user_by_username` compares exactly on both backends, so
  `Alice` and `alice` are different accounts and must be different buckets.
- **A success clears the bucket outright** (`RateLimiter::clear`) rather than
  refunding the one attempt it cost (`release`, which is what the per-address
  limiter still does). Without that, an owner who mistyped nine times and then
  signed in correctly would sit one failure away from a 15-minute lockout with
  the credential already proven. The per-address limiter must *not* clear: a
  success there says nothing about the other attempts from that address, which
  may be a shared NAT carrying an attacker too. It does hand an attacker a
  reset whenever the owner signs in, which is bounded by how rarely people
  actually log in (sessions idle out after 72 hours).
- **An account lockout is a denial-of-service primitive**, handed to anyone who
  knows a username, and that is accepted rather than solved — the Cheat Sheet
  names the trade-off and every account-lockout design has it. Once the budget
  is spent the *correct* password is refused too
  (`tests/login_rate_limit.rs::a_locked_account_refuses_even_the_correct_password`
  pins this as a decision, not a surprise). What keeps it proportionate: the
  budget is a rolling window rather than a latch, so nothing needs unlocking;
  it is per-account, so a sprayed username never denies the instance; the state
  is per-process, so a restart clears it; and a forward-auth deployment does
  not use password login at all.

Both limiters answer with the same 429 body (`web::throttled_login`), differing
only in `Retry-After`. Which bucket ran out is not disclosed, and the wording
avoids implying that the submitted username names a real account. The refusals
are distinguishable in the log, though — `reason = "rate_limited"` versus
`"account_locked"` (see "Rejected authentication attempts" above) — because for
an operator they mean quite different things.

The tracked-key map is capped (`MAX_ENTRIES`, 10 000). On reaching it the
limiter first prunes windows that have already elapsed; if every entry is
still live — a spray from more distinct sources than the cap — an address with
no bucket of its own is charged to a single **shared overflow bucket**
(`max_attempts × OVERFLOW_FACTOR`, i.e. 50 attempts per window) rather than
admitted unmetered. That closes a bypass in which the cap itself was the
attack: hold 10 000 live windows and every further address guessed without
limit. It stays deliberately fail-open-ish — refusing outright would turn the
same spray into a global login lockout, and an address that still owns a
bucket with room left is unaffected either way. Existing counters are never
cleared to make room (that would let anyone already throttled reset their own
budget), and a successful login refunds whichever bucket paid for it.

`ping::ClientIp` and `ratelimit::rate_limit_key` resolve the client address
differently on purpose, and the two must never be merged. `ClientIp` wraps
`auth::client_ip`'s **leftmost** hop and exists for *attribution* —
`pings.source_ip`, the IP a session row stamps — which is why `/ping/*` and
the login/setup handlers share it for "whose request was this." It is not
used for the limiter: `rate_limit_key` reads the **rightmost** hop instead,
because the limiter is a *security control*, and under a stock appending
proxy the leftmost hop is client-controlled — keying the limiter on it (via
`ClientIp` or otherwise) would let an attacker mint a fresh bucket per
request, reopening the exact bypass `rate_limit_key` exists to close.

### The password policy, and why `/login` is exempt from it

`auth::validate_password` is the one validator, called by every surface that
**sets** a password: `setup_submit`, `web::account_password`,
`web::users_create` and `web::users_set_password`. Length is the only rule —
at least `MIN_PASSWORD_CHARS` (15) and at most `MAX_PASSWORD_CHARS` (128),
counted in *characters* so a non-ASCII passphrase is not penalised for its
UTF-8 width. There are deliberately no composition rules and no excluded
characters (whitespace and unicode included), and the password is never
trimmed: NIST SP800-63B and OWASP's Authentication Cheat Sheet both treat
composition requirements as counterproductive. 15 rather than 8 because that
is the figure that applies **without MFA**, which is pingward's situation; if
a second factor is ever added, that constant is what changes.

Over the maximum is a *rejection*, never a truncation — a silently truncated
password would authenticate a shorter prefix than the user believes they set.
The maximum is not a denial-of-service defence: unlike bcrypt, argon2's cost
comes from its memory/time parameters and barely moves with input length.

`POST /login` does **not** validate, and must not start: the length of a
submitted password is not evidence of anything, and enforcing a floor at
sign-in would lock out every account whose credential predates the policy.
`web::users_set_password` renders its rejection rather than redirecting to
`/admin`, which is what it used to do for an empty password — a bare redirect
back to an unchanged page is indistinguishable from success, and an admin who
believes they rotated a credential they did not is the wrong failure for this
control.

The Cheat Sheet's breached-password blocklist (Pwned Passwords) is the one
control in that section **not** implemented, and the omission is deliberate: at
a 15-character floor almost nothing in a top-100k list is still eligible, so
the marginal gain is small against a new SHA-1 dependency plus either an
outbound request on the password-set path or a checked-in list that goes
stale. `validate_password` is the single seam to add it at if that changes.

### Equal cost for an unknown username

`auth::verify_password_or_dummy` is what `login_submit` calls, never a bare
`verify_password` behind an `is_some_and`. When there is no stored hash — an
unknown username, or a forward-auth account with no local password — it still
runs one argon2 verification, against a throwaway PHC string minted once per
process from a random secret (`dummy_password_hash`), and discards the result
through `black_box`.

Skipping that work is the "quick exit" pattern the Cheat Sheet names as a
user-enumeration hole. The generic `invalid username or password` message is
worthless on its own if the *response time* still separates the two cases, and
argon2 is deliberately slow enough that the difference is trivially
measurable: `tests/auth_web.rs`'s
`an_unknown_username_costs_the_same_as_a_wrong_password` measured ~1.9 ms
against ~590 ms with the equalisation removed. It does not equalise
everything — the preceding `find_user_by_username` is still a hit-versus-miss
— but that difference is orders of magnitude below one verification.

### Re-authentication for sensitive actions

`web::reauthenticate` is the shared gate: it asks for the signed-in user's own
password again before an action, because a session cookie proves who *opened*
the browser, not who is at it now. Three surfaces use it —
`POST /account/password`, `POST /account/api-keys` and `POST /admin/unlock`.

Note what this is and is not defending. The Cheat Sheet motivates
re-authentication with CSRF, XSS and session hijacking; the first two are
already closed here (a derived CSRF token with no path exemptions, and a CSP
admitting no inline script), so the residual threat it answers is specifically a
**borrowed or exported session cookie**. That narrows how much it is worth, and
is why the gate is on two endpoints rather than every mutation.

`POST /account/api-keys` is the case that justifies it on its own. An API key is
a bearer credential that *escapes every session control*: it is bound by neither
`SESSION_IDLE_TTL_HOURS` nor `SESSION_ABSOLUTE_MAX_DAYS`, and
`users_set_password` deliberately leaves keys alone (the flash it raises exists
to tell the operator so). A borrowed browser therefore converts one session's
access into permanent access, and it is the one gated action that signing out
cannot undo afterwards. The check runs *before* the name and expiry are
validated, so a wrong password never reaches the rest of the handler.

Two properties are load-bearing:

- **A passwordless forward-auth account passes unchallenged**, and is not shown
  a field it could never fill in (`has_password` gates both that field and the
  "Change password" card). There is no stored credential to verify against, the
  account's authority lives at the gateway, and pingward has no protocol for
  asking the gateway to re-assert it. So a borrowed *forward-auth* session can
  still mint a key — a real asymmetry, recorded here rather than papered over.
  Refusing instead would be worse: those users legitimately need API keys and
  would have no way to obtain one. This is the opposite outcome to
  `account_password`'s 403 for the same kind of account, and deliberately so —
  that one refuses because setting a *local* password on a gateway account
  would create a second way in that the gateway's sign-out could not end.
- **Attempts are charged to the account limiter**, the same bucket keyed the
  same way as `login_submit`'s, because this is the same activity as guessing at
  the login form — just from an authenticated seat. Before this existed,
  `/account/password` was an unmetered password oracle: a stolen session could
  guess its owner's password as often as it liked. A success clears the bucket,
  exactly as a successful login does.

### Creating a user, and the order of the checks

`Store::create_user` returns `CreateUserError`, not `sqlx::Error`.
`users.username` is `UNIQUE` in both migration sets, so a duplicate is an
ordinary outcome of a form submission — but as a bare `sqlx::Error` it reached
`AppError::Db` and rendered a blank `500 internal error`, leaving an admin with
no message and no form to correct. The distinct `UsernameTaken` variant makes
that impossible to route into the 500 path by accident, and it is classified
from the backend's own unique-violation code rather than a message, so it holds
on both drivers (`tests/pg_store.rs` covers the Postgres side, since SQLite's
2067 and Postgres's 23505 are different codes from different drivers).

`web::users_create` runs its checks in a deliberate order: **username → password
policy → duplicate → elevation gate → hash → insert.** Everything before the
gate is read-only, and a submission that could never succeed should say so
rather than send the admin through a confirmation for nothing — which is exactly
the flow that produced a confirmation reading as success. A locked admin is
still an admin, so learning that a username is taken discloses nothing they
cannot read off the user list. **The gate must stay immediately above the first
side effect**; `tests/admin_elevation.rs` pins both that a doomed submission
never asks for a password and that a valid one from a locked admin still writes
nothing.

The pre-check does not replace the error mapping. It is a read followed by a
write, so two admins submitting the same name concurrently can both pass it; the
constraint is the real arbiter and its refusal lands on the same message.
`setup_submit` handles `UsernameTaken` too — normally unreachable, since it
returns early unless the table is empty, but two visitors racing the very first
`/setup` both pass that check and the loser must not meet a blank 500 on the
app's first screen.

Usernames are compared **exactly**, matching the constraint and
`find_user_by_username`: `Admin` and `admin` are different accounts. Rejecting a
name the database would accept would be its own bug; making them collide is a
migration, not a validator change.

### Elevation for `/admin`'s access-granting actions

`/account` can carry a password field on the form itself. `/admin` cannot: its
controls are single-button inline forms in a table row, and
`users_toggle_admin` posts no body at all. So re-authentication is **decoupled
from the action** (`src/elevate.rs`): a refused action redirects to
`GET /admin/unlock`, an interstitial that explains the requirement and takes the
password; `POST /admin/unlock` runs the same `reauthenticate` gate, and
`web::elevation` then checks that the confirmation is still fresh
(`ELEVATION_TTL_SECS`, 15 minutes).

With JavaScript, the bounce is pre-empted: `app.js` intercepts a marked
submission and asks in a native `<dialog>` instead, then submits the original
form once confirmed. That is not decoration — it is what stops the admin's
filled-in form being discarded. Bouncing to a page loses whatever was typed, so
an admin who confirmed came back to an empty form (and, for a while, to a
message that read as though the action had succeeded).

The layering is deliberate in both directions. `data-reauth` is rendered only
while locked (`elevation_locked`), naming the action so the dialog can say what
is about to happen; once confirmed nothing is marked and forms submit straight
through. The dialog talks to `POST /admin/unlock` with this app's existing
`X-Requested-With: fetch` signal, which returns 204/403/429 instead of HTML —
the *decision* is identical either way, so a scripted caller is never a weaker
door. Anything unexpected sends the browser to the page rather than leaving it
stuck in a dialog. And the server re-checks regardless of what the page
rendered: if `data-reauth` ever drifted from the handlers, the cost is a
needless dialog or a needless bounce, never an ungated action. The password
never leaves the browser except in that one unlock request — which is why the
form is not preserved server-side across the bounce instead: that would mean
stashing a plaintext password across a redirect.

No inline handlers, since the CSP is `script-src 'self'` with no nonce; the
dialog is built and wired from `app.js` by delegation, like `data-confirm` and
`data-href` before it. `<dialog>`/`showModal()` supplies focus trapping, Esc and
the backdrop, so none of that is script here.

The interstitial page remains, and remains the target of the server-side bounce
— it is what a browser without JavaScript gets, and `/admin` links to it so the
requirement is visible before anything is refused. It is a **page, not a field**,
because the requirement needs explaining. An admin who is already signed in and gets asked for their password
again will reasonably wonder whether something is wrong, so the page says why,
which three actions it covers, which it deliberately does not, how long
confirming lasts (rendered from the constant, so copy and code cannot drift),
and — importantly — that this is the same password rather than a second factor.
None of that fits beside a button in a table row. `/admin` keeps only a one-line
state note linking to it, so the requirement is discoverable before an action is
refused rather than only after.

The refused action is **not replayed** afterwards: the admin lands back on
`/admin` and clicks again. Replaying would mean stashing a POST body across a
redirect, and one extra click is cheaper than that machinery.

The line is **granting versus removing access**, not "dangerous versus safe":

| Gated | Not gated |
| --- | --- |
| `POST /admin/users` (creates an account + password) | `POST /admin/users/{id}/delete` |
| `POST /admin/users/{id}/password` (replaces a credential) | `POST /admin/users/{id}/disabled` |
| `POST /admin/users/{id}/admin` **when promoting** | the same route when demoting |

Each gated action hands out access that outlives the browser session that
performed it — the same property that made API-key creation worth gating, which
is why `users_toggle_admin` is gated in one direction only. The ungated column
is not an oversight: disabling, demoting and deleting *take* access away, and an
operator who believes an account is compromised must be able to do them without
first finding their password. Ungating them is the same instinct that leaves
`/account`'s session-revoke controls alone.

Deliberately outside the gate too: `settings_save` (no credential; shortening
`audit_retention_days` is already recorded by its own `settings.update` audit
entry) and `POST /admin/checks/{id}/ping-url` (a disclosure of one check's
capability token, already audited, and recoverable by regenerating it).

Three properties:

- **State is in-memory and per-process**, like `crate::ratelimit` and
  `AppState::events`, and this is the one place that costs nothing. Elevation is
  short-lived by design, so persisting it would buy at most the tail of one
  window; a restart or a second replica just means entering the password again,
  which is the safe direction for a privilege gate. That is why this needed no
  migration.
- **Keyed per session, by the SHA-256 handle**, never the raw session id — the
  rule `auth::session_log_handle` and `/account`'s rows already follow. Per
  session rather than per user, so unlocking one browser does not unlock
  another signed in as the same admin, and `logout` revokes it.
- **A passwordless forward-auth admin is never gated** (`Elevation::not_applicable`),
  and the card is hidden rather than shown with a field they could not fill —
  the same asymmetry, for the same reason, as the API-key gate above.

A refusal is a redirect to the interstitial, not a 403: the controls stay live
in the table (hiding them would make the page depend on a timer), so the honest
answer to clicking one while locked is to explain the requirement and offer the
way through it. The check is server-side regardless of what the page rendered.

### Confirming a destructive action

Every irreversible control is a one-button inline form carrying a
`data-confirm` message that `app.js` turns into a native `confirm()`. That
attribute is inert without script, so for a browser running none the only thing
between a misclick and a deleted project was a feature it was not running.

The gate is therefore server-side, in the same shape as the elevation gate
above: a destructive handler does its work only when the request carries
`?confirmed=1`, and otherwise renders `templates/confirm.html`, which asks the
same question as a page and offers a form that re-posts the identical action
with the flag. `app.js` appends the flag once its dialog is accepted, so a
scripted browser still asks in place and still posts once. Neither side trusts
the other: the page cannot skip the flag, and the server never assumes a dialog
ran.

Three details are load-bearing:

- **The flag rides in the query string, not the body.** Several of these forms
  legitimately post nothing at all (`users_toggle_admin` sends no fields), so a
  body extractor would reject them with a 415 *before* authorization ran,
  turning `owned_check`'s 404 into a content-type error. `Query<ConfirmQuery>`
  is infallible — no query string at all deserializes to "not confirmed", which
  is exactly the unscripted first click.
- **The gate sits below authorization and below every refusal.** A stranger's
  check is a 404, not an invitation to confirm deleting something they cannot
  see; and a delete that the self-guard or last-enabled-admin guard will block
  says so rather than asking for a confirmation it would then ignore. Same
  ordering, for the same reason, as `users_create`'s validation-before-elevation.
- **The two `/admin` toggles are gated in one direction only**, matching what
  the template renders `data-confirm` for. Demoting and disabling ask; promoting
  and re-enabling do not — promotion is already behind the elevation password,
  and making an operator confirm an *undo* is friction pointing the wrong way.

The page's copy is deliberately not the same string as the dialog's: a
`confirm()` can show one terse line, while the page has room to name what the
action takes with it. Same reason `/admin/unlock` is a page rather than a
field.

### Rejected authentication attempts

Failures are logged as `tracing` events under the `pingward::auth` target,
separate from `pingward::session` so an operator can filter the two apart.
Nothing else in pingward observes a rejected attempt: the `audit_log` table
records what succeeded, and the rate limiter keeps its counters in memory and
tells nobody. This log is therefore the only signal that the login page is
being sprayed.

- `login.failed` (`web::log_login_failure`) — `username`, `ip`, `bucket`,
  `reason`. One event name for all three rejection paths, discriminated by
  `reason` (`bad_credentials`, `account_disabled`, `rate_limited`,
  `account_locked` — the last two being the per-address and per-account
  limiters respectively, kept apart because only the second one means somebody
  is working on one specific account), so a single
  query catches them; splitting them across event names is how a lockout stops
  being noticed. Two addresses because they can legitimately differ: `ip` is
  the attribution address (`ping::ClientIp`, what a session row stamps) and
  `bucket` is the key the limiter counted against (`rate_limit_key`) — see
  above for why those resolve differently.
- `reauth.failed` (`web::log_reauth_failure`) — `username`, `user_id`,
  `surface`, `reason`. One event for every re-authentication gate (see below),
  discriminated by `surface` (`password_change`, `api_key_create`,
  `admin_unlock`) rather than
  by event name: "somebody is guessing this account's password" is one thing to
  alert on, and an operator should not have to enumerate the forms to catch it.
  `reason` is `bad_current_password` or `rate_limited`. Kept separate from
  `login.failed` because that one is unauthenticated and carries an address
  instead of a `user_id`.
- `csrf.rejected` (`web::log_csrf_rejection`) — `reason`, `handle`. One event
  for all five of `csrf_guard`'s refusal paths: `no_session`,
  `header_mismatch`, `body_unreadable`, `token_missing` and `token_mismatch`.
  It is the **only** one of these three events emitted at two different levels,
  and the split is about volume rather than severity. `csrf_guard` is layered
  outside every handler, so it answers before `login_submit` ever consults
  `login_limiter`: an unauthenticated bot is refused here with nothing
  throttling it, and since a bot never carries a `_csrf` field, all of that
  traffic lands on `token_missing`. That one reason is therefore `debug!` and
  the rest are `warn!` — every other path means a token was actually presented
  and still failed to verify, which is what a token drifting out of step with
  its session looks like from the server side, and the event this whole
  vocabulary exists to make visible. `no_session` is unreachable while the
  layer ordering holds (`anonymous_session` runs outside `csrf_guard` and
  guarantees a signed cookie downstream), so it warns precisely because only
  that ordering makes it so. `tests/csrf_logging.rs` pins the levels; without
  it, promoting `token_missing` to `warn!` is a one-word change that reads as
  a tidy-up and silently costs the signal.

The 403 stays bodyless throughout — the gap this event closed was a refusal
leaving no trace for the *operator*, not one that failed to explain itself to
the caller, and naming the missing field would only tell a scanner what to send
next.

The submitted password is never logged. `username` is attacker-chosen input,
so it goes through `auth::log_username` (truncated, so a megabyte of form data
cannot become a megabyte of log) and every call site renders it with `Debug`
(`username = ?…`) — that quoting is what stops an embedded newline forging a
second entry in `text` log format. `tests/auth_logging.rs` pins both halves.

Session creation, renewal and destruction are logged as `tracing` events under
the `pingward::session` target (not the `audit_log` table — that models "an
admin acted on a target" via `actor_*`/`target_*` columns with no `ip`/
`user_agent`, whereas these are per-request, higher-volume, and already have a
JSON log pipeline aimed at a log aggregator; see `PINGWARD_LOG_FORMAT` below).
The target lets an operator silence them independently
(`RUST_LOG=info,pingward::session=warn`) without touching the rest of the
`info` filter. Where an event does carry a per-session identifier it is
always `handle` — `auth::session_log_handle`, the same (truncated to 16 hex
characters) SHA-256 handle `/account` uses to identify a row — **never the
raw session id**, which is the bearer secret the cookie signature is attached
to. Not every event has one to carry: the bulk `session.destroyed` reasons
below (`revoke_others`, `password_change`, `password_reset`, `user_disabled`,
`user_deleted`, `expired`) act on many rows via a single query, so they log
`count` instead.
Fields:

- `session.created` (`web::open_session`, the single mint point for all three
  creation paths — `setup_submit`, `login_submit`, and `forward_auth_session`,
  distinguished by `sso`) — `handle`, `user_id`, `sso`, `ip`, `user_agent`,
  `expires_at`.
- `session.renewed` (`Store::find_session_user`, the slide branch) — `handle`,
  `user_id`, `ip`, `user_agent`, `expires_at`, `renewal`. That last field is
  `auth::RenewalKind`: `slid` for the ordinary forward move (including one
  truncated by the absolute cap) and `clamped` when the stored window was
  *longer* than the current policy grants and was pulled back. The two mean
  different things operationally — a clamp is only produced by a stale writer
  (a rolling deploy, a second instance on one `DATABASE_URL`) or by a build
  that lowered `SESSION_IDLE_TTL_HOURS`, so a burst of them is a deployment
  signal rather than user activity.
- `session.destroyed` — one per teardown path, each tagged with a `reason`:
  `logout`, `revoked` (self-service, `handle`/`user_id`/`is_current`),
  `revoke_others` (`user_id`/`count`), `password_change` (`user_id`/`count` —
  the owner changing their own password on `/account`, so there is no separate
  actor), `password_reset` (`user_id`/`count`/`actor_user_id`, the
  admin-driven one), `user_disabled` (same fields, only on
  the disabling direction), `user_deleted` (`user_id`/`actor_user_id` — no
  `count`, since those rows go via `ON DELETE CASCADE` rather than a query
  this handler issues), and `expired` (`prune::prune_once`, one aggregate
  `count` per prune pass rather than one event per row, since
  `delete_expired_sessions` returns no ids to build a `handle` from).

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

### Reading vs. disclosing under `/admin`

An admin browsing another user's data reads names, schedules and history —
none of it a credential — and auditing every page open buried the entries
that mattered under browsing noise. `web::audits_as_mutation` therefore gates
the three cross-user resolvers on the request method. That gate lives in the
resolvers rather than at each call site for a specific reason: they are the
choke point for reads *and* writes, so removing the read audit anywhere else
would have silently taken every admin pause/resume/delete/regenerate with it.

The exception is the one read that hands over a credential. A check's ping URL
is a bearer token — anyone holding it can mark that check up or down — so the
admin check page withholds it behind `POST /admin/checks/{id}/ping-url`, which
records `admin.ping_url_reveal`. It is a POST rather than a `?reveal=1` so the
disclosure cannot happen without passing through the handler that writes it
down, and the usage help (which prints the URL five more times) is withheld
with it. `web::CheckPageViewer` carries the decision; it replaced a separate
`admin: bool` parameter so the action-URL prefix and the credential's
visibility cannot be passed contradicting each other. An admin viewing a check
they own themselves is not gated — `viewer_id == owner_id` — since nothing is
disclosed to anyone.

The REST API needs no equivalent: `CheckDto` carries `ping_uuid`, so a
cross-user read there *is* a disclosure, and `admin.api.access` already
records it.

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
  1h), deletes pings/notifications/audit rows past their retention window and
  any already-expired session rows. Each retention window is an independent
  global setting and each defaults to off.

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

### What a notification says

Every text-oriented channel renders the same `notify::event_text`, capped at
**four short lines** — headline, context, reason, link:

```
🔴 DOWN — nightly-backup
Project: infra · every 5m (grace 1m)
No ping since 2026-07-29 17:03 CST (1h5m ago)
https://pingward.example.com/checks/42
```

The cap is the design: anything past those four lines lives on the linked
page, which is what the link is for. `event_title` (ntfy/Pushover title, email
subject) stays one line: `pingward: infra/nightly-backup is DOWN`.

Everything but the headline comes from `notify::EventDetail`, and every field
on it is `Option` — a failed lookup or an unset `PINGWARD_BASE_URL` drops a
line instead of the notification. `EventDetail::default()` renders the original
bare one-liner, which is what the channel-test path uses.

Two things decide where that struct is built:

- **It is built at the call site, not during delivery.** For an `Up` event
  `last_ping_at` must be the ping *before* the recovery; `ping::apply` has that
  snapshot in hand (`resolve` loaded it before `mark_ping` overwrote the row),
  and a re-read inside `deliver_event` would report the recovery ping itself.
- **Only the caller knows why a check went down.** `DownCause` is set by
  `scan_once` (`Overdue`, or `Overrun` when an in-flight run blew
  `max_runtime_secs` — reported in preference to overdue, being the more
  specific story) and by `ping::apply` (`Failed { exit_code }`). `nag_once`
  sets none: a reminder fires long after the transition, so it reports "Last
  ping …" rather than claiming "No ping since …" for a check that ended up down
  by pinging `/fail`.

Timestamps render via `fmt_at` (`%Y-%m-%d %H:%M %Z`) with a relative suffix
from `duration::fmt_duration` — the same rendering the edit forms use, so `300`
reads as `5m` in both places. Which zone is a two-step fallback: the
instance-wide `display_timezone` setting if an admin set one
(`EventDetail::with_display_timezone`, read through `Store::display_timezone`),
otherwise the **check's** own zone, otherwise UTC.

That setting exists because of an asymmetry worth naming: the web UI renders
every absolute time in the *viewer's* zone (the `.localtime[data-ts]` spans
localized by `pw.localize` in `base.html`), and a notification has no browser to
do that. Without the setting the same instant reads as one time in the UI and
another in ops chat. The check's own zone is the fallback rather than the
default-winner because it is written for the **cron** schedule — the wall clock
the expression fires on — not for whoever reads the alert.

A settings read failure degrades to `None` rather than propagating:
`Store::display_timezone` swallows the error, because a settings query going
wrong must not stop a down alert from going out.

Per-channel affordances hang off the same struct: ntfy gets a `Click` header
(guarded on the URL being header-safe, since an invalid `HeaderValue` would
abort the whole send), Pushover an `url`/`url_title` pair, and the webhook
payload gains `check_id`/`project`/`url`/`schedule`/`timezone`/`last_ping_at`/
`cause`/`exit_code`/`text` **additively** — its original four keys (`check`,
`event`, `at`, `project_id`) are unchanged, so an existing consumer keeps
parsing what it parsed.

The project name costs one query per notification: batched as
`Store::all_project_names()` once per scan/nag pass, and inside the spawned
delivery task in `ping::apply` so it never lands on the ping response path.

### Editing a channel without leaking its secrets

`channels.config_json` is a single plaintext JSON blob holding delivery
credentials, and until channel editing existed nothing ever read it back out to
a user-visible surface (`ChannelDto` omits it outright — see `src/api/dto.rs`).
The edit form is the first surface that could break that, so the whole design is
built around **never re-rendering a stored secret**:

- One merge rule, in `web::validate_channel_update(form, Option<&Channel>)`: a
  blank submitted field keeps the stored value. `validate_channel` is now just
  that function with `None`, so create and edit share one set of per-kind
  required-field checks — blanking a *required* credential that isn't stored is
  still an error.
- Secrets render as empty inputs with `placeholder="unchanged"` plus a
  `configured` / `not set` pill (the same treatment as `/admin`'s Environment
  card). The template only ever sees `web::ChannelEditView`, which carries the
  non-secret values plus `has_*: bool` flags — non-leakage is a property of the
  type, not of template discipline.
- What counts as secret is a judgement call: a webhook or Slack **URL** is the
  capability to post, so it is hidden; a telegram chat id, ntfy server/topic,
  and email recipient are identifiers and are pre-filled.
- `ntfy_token_clear` is the one escape hatch — blank-means-unchanged would
  otherwise make the single *optional* secret impossible to remove.
- **`kind` is immutable** (rendered as static text, and a submitted `kind` is
  ignored): a stored config only has meaning for the kind that wrote it, so
  there is no right answer for carrying it across. `Store::update_channel`
  takes no `kind` parameter at all.
- `PATCH /api/v1/channels/{id}` shares the validator and is therefore a *merge*,
  not a replacement like `PATCH` on projects/checks — a client cannot re-send
  credentials it was never given.
- The same projection rule applies to every other surface that lists channels:
  the project page renders through `web::ProjectChannelRow` (id/name/kind), not
  a whole `Channel`. Handing a template the model would put `config_json` in the
  render context of a page that has no use for it — nothing prints it today, but
  the guarantee should hold by construction rather than by review.

At-rest encryption of `config_json` was considered and deliberately deferred.

## Swappable history sections

Three tables page and filter the same way: the check page's recent pings and
recent notifications, and `/admin`'s audit trail. Each is one template
(`check_pings.html`, `check_notifs.html`, `admin_audit.html`) holding filter
controls + table + a `Newer`/`Older` keyset pager, rendered on two surfaces —
inlined into the full page by the handler and served standalone by a fragment
endpoint (`GET /checks/{id}/pings`, `…/notifications`, `GET /admin/audit`) —
so the markup has one source and a partial refresh cannot drift from a full
load.

A fragment endpoint answers a *navigation* with a redirect rather than a
partial (`web::wants_fragment` / `web::fragment_page_redirect`). The pager and
Clear controls are real `<a href>`s aimed at those endpoints, because with JS
on `wireSection` intercepts the click and swaps the response in place; followed
as ordinary links they used to render the partial as the whole document — no
`<head>`, so no stylesheet, no nav, no way back, a page that reads as a broken
site rather than as a missing feature. Absent `X-Requested-With: fetch` the
endpoint now redirects to the page that embeds the section, carrying the query
string (the full page parses the very same query struct, so the cursor and
filters survive) and anchoring on the section. Ownership/admin resolution runs
**before** the redirect decision, so the fallback never becomes a cheap way to
confirm that someone else's check exists.

Each section's filter is a **real GET form** submitting to the page that
embeds it (`/checks/{id}`, `/admin`), with named controls and a `type="submit"`
button, so it filters with no script at all — the page parses the very same
query struct the fragment endpoint does. It had none of that: no `method`, no
`action`, no `name` on any control and a `type="button"` Filter, four separate
reasons a scriptless click did nothing. With script, `wireSection`'s
`data-apply` handler cancels the submit and swaps the fragment in place
instead, and `data-nosubmit` stops a stray Enter navigating away.

Two consequences worth knowing:

- A GET submission replaces the **whole** query string, so the check page's two
  sections would clear each other's filter. Each form re-sends the other's
  half as hidden inputs (`web::carry_fields`), and each Clear link keeps them
  (`web::clear_href`). Clear still points at the *fragment* endpoint, not the
  page: with script `wireSection` intercepts it and expects a partial back, and
  without script that endpoint redirects to the page carrying the query.
- The `datetime-local` controls hold the viewer's **local** wall clock.
  `app.js` converts to UTC before fetching; submitted raw they are read as UTC
  (`web::parse_date_bound` already accepted the naive form). The two modes
  disagree by design, and each agrees with what it *shows*: timestamps are
  localized by `app.js` and rendered as UTC when it is not running.

The client half is `pw.wireSection` in `base.html`: it delegates clicks inside
the section, turning a pager/Clear link or the Filter button into a `fetch` of
the fragment endpoint whose response replaces the section body, then re-runs
timestamp localization, the expandable-row bindings and the `datetime-local`
fill against the swapped-in markup. It lives in `base.html` (a head script, so
it is defined before the per-page scripts inside the `body` block that call
it) rather than in any one page, which is what lets the admin audit card reuse
the check page's behaviour instead of copying it.

The server half is `store::keyset_page`: one query builder shared by all three
tables, paging by `id` (monotonic and index-backed, so it does not drift under
concurrent inserts the way an offset would) with a limit+1 fetch to decide
`has_newer`/`has_older`. `pings`/`notifications` pass a scope of
`("check_id", id)`; `audit_log` belongs to the instance rather than to any
row, so it passes `None` and the `WHERE` clause is assembled conditionally —
an unscoped, unfiltered page has no predicates at all. Filter values are
always bound; only self-generated literals (table, column, operator,
placeholder) are interpolated into the SQL text.

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

`e2e/` is a cucumber + thirtyfour harness: `.feature` files paired with Rust
step definitions under `tests/e2e/steps/`, run via
`cd e2e && cargo test --test e2e`. It is a **cargo workspace of its own**, so
a `--workspace` build or coverage run at the root never compiles it or drives
a browser. Each scenario spawns its own fresh `pingward` binary against a
temporary SQLite database on a random port, so scenarios don't share state —
which is not merely tidiness here: `POST /setup` creates the first admin once
and almost every scenario walks through it.

`no_js.feature` carries an **`@nojs` tag**, and a `before` hook reads it to
open that scenario's session with `Emulation.setScriptExecutionDisabled` —
which is what Playwright's `javaScriptEnabled: false` did underneath. It
applies to the *next* document, so sessions are per-scenario rather than
shared. Scripting is a browser capability rather than something a page can be
asked to give up, so it needs its own session — and
without one the whole suite is structurally blind to anything the UI has
quietly started depending on `app.js` for, which is how a dashboard with no
route to a check page, pager links rendering a bare fragment, and captured
output nothing could open all shipped. Assertions that need both sides (a
panel that is open without script and collapsed with it) are paired across
`no_js.feature` and the JS-on suite deliberately: a fix that simply left the
panel always open would satisfy the no-JS half on its own.

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
