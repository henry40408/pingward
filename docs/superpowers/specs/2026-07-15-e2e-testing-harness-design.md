# E2E Testing Harness (Playwright + BDD) — Design

**Date:** 2026-07-15
**Status:** Approved (design)
**Scope of first cut:** Authentication flows only (setup / login / logout)

## 1. Goal

Introduce browser-level end-to-end (E2E) tests to pingward, modeled on the E2E
harness in the sibling `rdrs` project. The harness drives a **real compiled
`pingward` binary** with a real browser (Chromium via Playwright), asserting
behavior the way a user experiences it. The first cut covers the authentication
surface only; the harness is built so later scopes (projects, checks, ping →
status changes, notification channels) slot in without rework.

Non-goals for this cut: Project/Check/Channel/Ping/notification scenarios, direct
database seeding, and any change to pingward application code.

## 2. Reference: how `rdrs` does it

`rdrs` keeps its E2E suite in a self-contained Node project at `e2e/`:

- **Playwright + `playwright-bdd`** — Gherkin `.feature` files compiled to
  Playwright specs; step definitions in `steps/*.js`.
- **`global-setup.js`** runs `cargo build` once to produce the debug binary.
- **`support/server.js`** spawns the real binary against a **temp SQLite DB** on a
  **random port**, then polls a health endpoint until ready.
- **`support/api.js`** exercises real HTTP auth (register/login) to establish
  preconditions; **`support/seed.js`** writes directly to SQLite (via
  `better-sqlite3`) for fast fixture setup.
- **`support/fixtures.js`** wires it together with `playwright-bdd`'s
  `test.extend`; **worker-scoped** server reused across scenarios, **test-scoped**
  unique user per scenario for isolation. `try/finally` cleanup.
- CI runs the suite **sharded** in a dedicated job.

We mirror this shape. The one deliberate divergence is the isolation model
(§4), forced by pingward's different authentication model.

## 3. pingward facts the design relies on

Verified against `src/web.rs`, `src/lib.rs`, `src/config.rs`, `templates/`:

- **Health endpoint:** `GET /healthz` → `200 "ok"` (CSRF-exempt sibling router).
  Used for readiness polling.
- **Startup env:** binary reads `DATABASE_URL`
  (default `sqlite://pingward.sqlite3?mode=rwc`), `PINGWARD_BIND`
  (default `127.0.0.1:8080`), `RUST_LOG`. Migrations run on startup, so a fresh
  temp DB is schema-ready. **Migrator uses relative paths → the binary must be
  spawned with `cwd` = repo root.**
- **No self-service registration.** Users are created only via the one-time
  `POST /setup` (first admin) or admin-only `/users`. This is the crux of the
  isolation decision.
- **Auth flow (all form-encoded, fields `username` + `password`):**
  - `GET /` with zero users → redirect `/setup`; with users but no session →
    handled by dashboard/login redirects.
  - `GET /setup` with users present → redirect `/login`.
  - `POST /setup` (first admin): creates admin, starts session, redirect `/`
    (**you are logged in after setup**). Empty field → re-render with
    "username and password are required".
  - `GET /login` with zero users → redirect `/setup`.
  - `POST /login` valid → session + redirect `/`; invalid → re-render with
    "invalid username or password"; disabled → "account is disabled".
  - `POST /logout` → delete session, redirect `/login`.
- **CSRF:** synchronizer token per session, validated by `csrf_guard` on
  state-changing browser requests. **`POST /login` and `POST /setup` are
  explicitly CSRF-exempt** (pre-session paths). `POST /logout` is **not** exempt,
  but its rendered form embeds the hidden `_csrf` field, so a browser submit
  carries the token automatically.
- **Selectors:** setup/login templates expose `#username`, `#password`, and
  submit buttons labeled "Create admin" / "Log in". The nav logout control is
  `<button>Log out</button>`. **No `data-testid` attributes exist**, so E2E uses
  CSS id + role/name selectors — no template changes required.

## 4. Isolation model (the one architectural decision)

**Chosen: Approach A — per-scenario fresh server + fresh temp SQLite.**

Each scenario spawns its own instance of the prebuilt binary pointed at its own
temp SQLite file, and tears it down afterward. Rationale:

- pingward's `/setup` is a one-time bootstrap that **closes once any user
  exists**, and there is no open registration. rdrs's model (one worker-scoped
  server, isolate by unique username) therefore cannot work: the "fresh setup"
  scenario needs an empty DB, while login scenarios need a pre-existing user, and
  parallel/unordered execution would leak the setup user across scenarios.
- The auth suite is small (~5 scenarios). `cargo build` runs once in
  global-setup; per-scenario cost is only process spawn + startup migrations +
  health poll, which is cheap for a self-contained binary.

Rejected alternatives:

- **B — worker-scoped server + `better-sqlite3` DB reset between scenarios.**
  Faster reuse, but requires mutating a running SQLite (WAL) DB from a second
  connection and a reset hook; more moving parts than the tiny suite justifies.
  Revisit if the suite grows large enough that spawn cost dominates.
- **C — rdrs's worker-scoped + unique-username model.** Does not fit: no open
  registration and one-time setup.

## 5. Directory layout

```
e2e/
  features/
    auth.feature              # Gherkin scenarios (setup / login / logout)
  steps/
    auth.steps.js             # step definitions binding phrases → Playwright
  support/
    server.js                 # spawnPingward(): temp SQLite, random port, /healthz poll
    api.js                    # bootstrapAdmin(): POST /setup (CSRF-exempt) to pre-create the admin
    fixtures.js               # playwright-bdd test.extend — test-scoped server/page/api
  global-setup.js             # cargo build once (skip if binary present)
  playwright.config.js
  package.json
  .gitignore                  # node_modules/, .features-gen/, test-results/, playwright-report/
```

## 6. Harness components

### `support/server.js` — `spawnPingward()`
1. `mkdtempSync` a temp dir; DB path = `<dir>/test.sqlite3`.
2. Find a free ephemeral port (open a `net` server on port 0, read the assigned
   port, close).
3. `spawn` the debug binary (`target/debug/pingward`) with **`cwd` = repo root**
   (relative migration paths) and env:
   - `DATABASE_URL=sqlite://<dbPath>?mode=rwc`
   - `PINGWARD_BIND=127.0.0.1:<port>`
   - `RUST_LOG=warn`
4. `waitForServer`: poll `GET /healthz` until `200` (timeout ~30s), fail loud.
5. Return `{ url, dbPath, cleanup }`; `cleanup` sends `SIGTERM` and removes the
   temp dir.

### `support/api.js` — `bootstrapAdmin(url, username, password)`
- A single `fetch('POST /setup', form-urlencoded {username, password})`. **No CSRF
  token needed** (setup is exempt). Used as a precondition for login/logout
  scenarios so the browser starts from a clean, logged-out state. The Set-Cookie
  it returns is discarded — the browser logs in itself.
- Mirrors rdrs's `api.register`, adapted to pingward's setup-only bootstrap.

### `support/fixtures.js` — `playwright-bdd` `test.extend`
- **`pingwardServer`** (test-scoped): calls `spawnPingward()`, `use()`s it inside
  `try/finally` that calls `cleanup()`.
- **`serverUrl`** (test-scoped): `pingwardServer.url`.
- **`api`** (test-scoped): bound to `serverUrl`, exposes `bootstrapAdmin`.
- `page` is Playwright's built-in fixture (navigated to `serverUrl` in steps).
- Standard admin credentials constant: `admin` / `password123`.

### `global-setup.js`
- Run `cargo build` (debug) from repo root once. Skip the build if
  `target/debug/pingward` already exists (matches rdrs; document that a rebuild is
  needed after app changes, since pingward embeds templates/assets at compile
  time via askama/`include_*`).

## 7. First-cut scenarios (`features/auth.feature`)

Tagged `@parallel`. No shared `Background` that creates a user (isolation is
per-server), but login/logout scenarios use a `Given an admin account exists`
step (calls `api.bootstrapAdmin`).

1. **Fresh instance redirects to setup** — visiting `/` on an empty instance lands
   on `/setup` showing the "Create admin" form.
2. **Creating the first admin logs you in** — fill `/setup` and submit; land on
   `/` (dashboard) in a logged-in state.
3. **Setup rejects empty credentials** — submit empty fields; stay on `/setup`
   with "username and password are required".
4. **Login with valid credentials** — given an admin exists, visit `/login`, sign
   in, land on `/` logged in.
5. **Login with wrong password fails** — given an admin exists, submit a bad
   password; stay on `/login` with "invalid username or password".
6. **Logout ends the session** — given signed in, click "Log out"; land on
   `/login`; visiting `/` no longer shows a logged-in dashboard.

(Scenarios 3 and 5 are cheap negative-path additions that share the same
harness; included because they exercise the error-render branches with no extra
infrastructure. Drop either if considered out of scope during planning.)

## 8. Running the suite

- **Local:** `cd e2e && npm ci && npx playwright test`. Convenience scripts
  (`test:ui`, `test:headed`) mirror rdrs.
- **CI:** add an `e2e-tests` job to `.github/workflows/ci.yml`:
  - checkout, `cargo build` (debug), setup Node 22, `npm ci`, cache Playwright
    browsers, `npx playwright install --with-deps chromium`,
    `npx playwright test` (sharded, e.g. matrix `--shard=n/2`).
  - upload the HTML report as an artifact.
  - Runs alongside the existing fmt/clippy/nextest jobs; independent of the
    Postgres/Mailpit services (auth scope uses SQLite only).

## 9. Dependencies (pinned, honoring the 7-day cooldown)

- `@playwright/test`, `playwright-bdd` — versions selected at install time,
  choosing the latest release **at least 7 days old**.
- No `better-sqlite3` in this cut (no direct seeding). Add it when a later scope
  needs DB-level fixtures.

## 10. Optional follow-ups (out of scope)

- A `PINGWARD_FAST_HASH`-style env flag to weaken Argon2 cost for tests (rdrs has
  `RDRS_FAST_HASH`). Not needed for ~5 auth scenarios; would require an app-code
  change.
- `better-sqlite3` seed helper + a `seed.js` module for future
  project/check/ping scenarios.
- Worker-scoped server + DB-reset isolation (Approach B) if the suite grows large
  enough that per-scenario spawn cost dominates.
- Mock upstream servers (SMTP/Mailpit) for notification-channel E2E.
