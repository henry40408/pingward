# CLAUDE.md

Guidance for Claude Code in this repository. `ARCHITECTURE.md` holds the full
design rationale; this file lists what breaks things if you get it wrong.

## What this is

pingward: self-hosted, healthchecks-style uptime/cron monitor, written in Go
(standard library `net/http`). Jobs ping a per-check URL; a background loop
marks overdue checks **down** and notifies via per-check channels. One process
serves a server-rendered UI and the machine `/ping/*` endpoints. It replaced an
earlier Rust implementation; existing databases, cookies, API keys and the
HTTP surface carry over unchanged.

## Commands

Run from the repository root. The Go version comes from `go.mod` alone (`go` +
`toolchain` lines); CI and the Dockerfile install nothing version-specific.

- Build: `go build -o bin/pingward ./cmd/pingward` (views and assets are
  embedded, so rebuild after any view, route or asset change; the E2E harness
  runs `bin/pingward`).
- Run: `go run ./cmd/pingward` (SQLite `pingward.sqlite3`, bind `127.0.0.1:8080`).
- Tests: `go test -race ./...`; one test: `go test ./internal/web -run TestName`.
- Lint (CI-enforced): `golangci-lint fmt --diff ./...` (formatting; must print
  nothing) and `golangci-lint run ./...`, each inside `e2e/` too. Config is in
  `.golangci.yml`; suppress a finding with `//nolint:<linter> // reason`. `run`
  does not check formatting, so `fmt --diff` is a separate step. The linter
  release comes from the CI action (dependabot tracks only the action).
- Mutation testing (CI-enforced, the `mutation` job): `go clean -testcache`, then
  `go run github.com/go-gremlins/gremlins/cmd/gremlins@v0.6.0 unleash .`. The
  efficacy threshold and the `e2e/` and `vendor/` exclusions live in
  `.gremlins.yaml`, not on the command line (gremlins ignores the flag).
  The job is sharded (`mutation-shard` matrix in `ci.yml`, plus an aggregate
  `mutation` job that is the required check): each shard runs `unleash .` with
  `--exclude-files` listing the complement of its packages (RE2 has no
  lookahead), `GOMAXPROCS=2`, `--workers 2` and `ulimit -v 4194304`. A mutant
  can make a loop run forever and allocate until the runner is SIGTERMed (exit
  143), so packages with loops that could do this have a `TestMain` watchdog
  (`main_test.go`) that aborts a hung or ballooning test binary. **A new
  package must be added to the shards by hand**: put it in one shard and in
  the `exclude` list of every other shard except `rest` (which excludes only
  the other shards' packages, so an unlisted package still runs there). Never
  pass `--test-cpu` (gremlins v0.6.0 builds an invalid `-cpu 2` argument and
  every mutant counts as killed).
- Dependency checks (CI-enforced, the `deps` job): `go mod tidy -diff`,
  `go mod vendor` then `git status --porcelain -- vendor go.mod go.sum` must
  print nothing, `go run golang.org/x/vuln/cmd/govulncheck@v1.9.0 ./...`, and
  go-licenses (`GOROOT=$(go env GOROOT) go run github.com/google/go-licenses/v2@v2.0.1 check ./... --include_tests --ignore github.com/henry40408/pingward --allowed_licenses=<list>`),
  at the root and in `e2e/` (with `--ignore github.com/henry40408/pingward/e2e`).
  The allow-list lives only in the `deps` job's `ALLOWED_LICENSES`; `e2e/` allows
  one more entry than the root. If go-licenses reports an unknown or disallowed
  licence, do not widen the list or swap the module without asking.
- Format before committing: `gofmt -w` on the files you touched.

Third-party modules are **vendored and committed** (`vendor/`, and
`e2e/vendor/`). To add or update a dependency: check that the release is at
least 7 days old, `go get <module>@<version>`, `go mod tidy`, `go mod vendor`,
and commit `go.mod`, `go.sum` and `vendor/` together. A `go get` without
`go mod vendor` breaks the build.

Version stamp: the footer shows `internal/version`, set at link time with
`-ldflags "-X github.com/henry40408/pingward/internal/version.version=..."`;
without it Go's embedded VCS info is used, else `dev`. `docker.yml` passes it as
the `GIT_VERSION` build arg since `.dockerignore` excludes `.git`. Releases are
`gh release create --generate-notes`; there is no version field to bump.
GitHub Actions are pinned by commit SHA with the tag as a trailing comment.

Tests that need services skip without them: `TEST_DATABASE_URL=postgres://…`
(PostgreSQL store tests) and `PINGWARD_TEST_SMTP_HOST` (SMTP). `docker compose up -d`
starts Postgres (5432) and mailpit (1025/8025); then export `TEST_DATABASE_URL`,
`PINGWARD_TEST_SMTP_HOST=localhost`, `PINGWARD_TEST_SMTP_PORT=1025`,
`PINGWARD_TEST_MAILPIT_API=http://localhost:8025`. Tests must never touch the
real `$HOME`: use `t.TempDir()` and set `HOME`.

### Browser E2E (`e2e/`, godog + chromedp)

- A **separate Go module** with its own `go.mod` and `vendor/`; `go test ./...`
  at the root does not run it. `e2e/go.mod` mirrors the root `go`/`toolchain`
  lines (CI checks it).
- Run from `e2e/`: `go test -count=1 .` (`-count=1`: the test cache cannot see a
  rebuilt `bin/pingward`). The harness builds `bin/pingward` only when it is
  missing, so rebuild it after changing code. Features: `E2E_FEATURES=features/ping_kinds.feature`
  (comma-separated). CI splits the suite with `E2E_SHARD=<n>/3` (scenarios are
  assigned by hash; a shard that selects none fails).
- Each scenario gets its own server binary, temp SQLite DB and random port
  (almost all walk through `POST /setup`). Selectors use `data-testid`.
- Needs a local Chrome or Chromium (chromedp starts it headless); `E2E_CHROME`
  pins the binary.
- Tags pick the environment (`harness.OptionsFromTags`): `@nojs` (only in
  `no_js.feature`) disables page scripts before the first navigation;
  `@fast-scan` sets `PINGWARD_SCAN_INTERVAL=1`; `@smtp-env`, `@trusted-proxy`
  add env. Anything `app.js` must not be the sole provider of needs a `@nojs`
  scenario; pair it with a JS-on one when both behaviours matter.
- Step files must not end in `_js.go` or another GOOS/GOARCH suffix: Go treats
  it as a build constraint and silently drops the file.

### README assets (outputs are committed)

- `cd e2e && go run ./cmd/screenshots` → `docs/screenshots/*.png`.
  `e2e/internal/seed` must keep seeded timestamps inside each schedule's budget
  or the boot scan rewrites the status. Cron `dow` is **Sunday = 1**.
- **Visual changes (CSS, view layout, front-end JS) regenerate the screenshots
  in the same change**; copy-only or logic-only edits do not need it. Nothing
  fails when they drift, so compare the affected PNG when unsure. Build from a
  clean tree: the footer stamp is in frame.
- `cd e2e && go run ./cmd/icons` → `internal/assets/static/apple-touch-icon.png`
  from `favicon.svg` (no browser needed).

### Golden files

`internal/*/testdata` holds the large captured fixtures (cron and timezone
tables, markdown, the OpenAPI document). Many were captured from the former
Rust server; each `testdata/README.txt` says how. Small Rust vectors (password
hashes, session cookies, notifier request bodies, timezone parse probes, API
error responses, `_sqlx_migrations` checksums and rows) are inline in the tests
that use them (`rust*` variables, `*MigrationGolden`). `openRustCreated`
(`internal/db/rust_created_test.go`) rebuilds the Rust-created SQLite database
from the embedded migrations plus the captured `_sqlx_migrations` statement and
rows.

View markup is pinned by `goldenDigests` (`internal/views/golden_digests_test.go`):
the sha256 of each page the Rust server rendered, compared with the normalised
Go output. A mismatch writes the output to a temp file and names the
`git show` command that recovers the Rust golden from history. A deliberate
markup change means replacing the affected digests (compare the output with the
recovered golden first) and regenerating the screenshots together. Never change
a digest to make a test pass without understanding why it differs.
`testdata/test_map.txt` maps the original Rust tests to Go tests; keep it
consistent when renaming a test (`internal/testutil/testmap_test.go`).

## Invariants

### Routing & CSP
- `server.New` composes `web` routes (behind their own middleware), the ping
  router, the API, assets and `/healthz`. The web middleware order is
  load-bearing: forward-auth session → anonymous session → CSRF guard →
  handler. `/ping/*` and `/api/v1` are siblings, so structurally outside CSRF.
- CSP (web routes only) is `script-src 'self'` with **no `'unsafe-inline'`, no
  nonce**. So: **no inline scripts, no `onclick=`/`onsubmit=`**: add behaviour
  to `internal/assets/static/app.js` via delegation (`data-href`, `data-confirm`,
  `data-nosubmit`, `data-reauth`). `style-src` keeps `'unsafe-inline'` only for
  heartbeat bar heights. `/api/docs` is deliberately outside the CSP (Scalar
  from CDN).

### Works without JavaScript (`internal/server/nojs_test.go`)
- `data-href` rows must keep a real `<a>`. Fragment endpoints redirect a plain
  navigation to the embedding page (`wantsFragment`).
- `data-confirm` enforces nothing: destructive handlers run only with
  `?confirmed=1` (query, since some forms post no body), else render the confirm
  page. The gate sits below authz and every refusal guard. See ARCHITECTURE.md
  "Confirming a destructive action".
- CSS hiding click-revealed content must hang off `:root.js` (set by
  `theme-init.js` before paint). JS-only controls are hidden via `:root:not(.js)`.
- The layout has **no default `data-theme`**. The light palette exists twice in
  `app.css` (`[data-theme]` + `prefers-color-scheme`): keep in sync; a test
  compares them.
- History filters are real GET forms; each carries the sibling section's filter
  as hidden inputs.
- Per-kind form fields switch via `:has()` in CSS, not JS. Absolute timestamps
  fall back to UTC text inside `.localtime[data-ts]` spans.

### Session, CSRF, secret (`internal/secret`)
- One `PINGWARD_SECRET` HMACs cookie, CSRF token and flash cookie with domain
  prefixes `session:`/`csrf:`/`flash:`: **keep the prefixes** (otherwise the
  CSRF token equals the cookie signature). Changing the format logs everyone
  out; the Rust-era cookies are still valid today.
- CSRF is derived, not stored; the guard has **no path exemptions**.
- **The cookie value is not the session id**: verify it through `internal/secret`.
- Session rows are shown/addressed by a SHA-256 handle (`apikey`), never the
  raw id.

### Auth (`internal/auth`, `internal/web`)
- Password validation (15–128 *characters*, no trimming/composition) must run
  on every surface that **sets** a password. **`/login` must never validate.**
- Login does constant-cost verification for unknown users (dummy hash).
- `POST /login`: IP limiter (5/IP/60s) + account limiter (10/account/15min,
  keyed on the submitted username before lookup; success clears).
  `auth.RateLimitKey` uses the **rightmost** XFF hop; `auth.ClientIP`
  (attribution) uses the leftmost: **do not unify them**.
- Callers of user creation must handle the username-taken error.
- Reauthentication guards `/account/password`, `/account/api-keys`,
  `/admin/unlock`; passwordless forward-auth accounts pass unchallenged.
- `internal/elevate`: in-memory 15-min unlock per session handle. Gate actions
  that **grant** access (create user, set password, promote); never gate
  delete/disable/demote. See ARCHITECTURE.md "Elevation for `/admin`'s
  access-granting actions".
- An admin cannot delete/disable/demote themselves.
- Owner scoping: not-yours is **404, not 403** (`projectFor`/`checkFor`/`channelFor`).
- Trusted proxies: `auth.IsTrustedProxy` (address or CIDR, canonicalised, no
  DNS). Forward-auth and `X-Forwarded-For` are honoured only from a trusted peer.
- Auth logging: `login.failed`, `reauth.failed`, `csrf.rejected`. Never log
  passwords. `csrf.rejected` `token_missing` is debug, others warn.
- Request handlers that write after authentication use a context detached from
  the client (`context.WithoutCancel`), so a disconnect cannot leave a
  half-applied change.

### Admin & audit
- Cross-user admin handlers reuse owner templates. Only non-GET admin access is
  audited as `admin.access`; the ping URL of another user's check is a
  disclosure, revealed via audited `POST /admin/checks/{id}/ping-url`. The REST
  API audits every admin cross-user access. See ARCHITECTURE.md "Reading vs.
  disclosing under `/admin`".
- Retention (`pings_/notifications_/audit_retention_days`) defaults to **off**;
  saving settings audits changed keys as `settings.update`.

### Persistence (`internal/db`, `internal/store`)
- One `database/sql` pool, SQLite (modernc, no cgo) or Postgres (pgx) by URL
  scheme. All SQL lives in `internal/store` and must run on both: `$N`
  placeholders, `RETURNING id`, no `ILIKE` (filter case-insensitively in Go).
- Times are written as RFC 3339 with `+00:00` and compared as **strings** in
  SQL: never change the write format. Reads also accept `Z` and
  `YYYY-MM-DD HH:MM:SS`.
- Schema changes go in **both** `internal/db/migrations/sqlite/` and
  `internal/db/migrations/postgres/`, numbered on from `0016`; they are embedded
  and never read from disk at runtime. `_sqlx_migrations` keeps its sqlx layout
  and SHA-384 checksums, so the Rust build can still open the database.
- Existing migration files are immutable (the checksum is verified at startup).

### Background loops & shutdown
- `cmd/pingward/serve.go` starts the scan loop (`internal/scheduler`) and the
  prune loop (`internal/prune`). Changing their parameters means updating
  `serve.go` and the scheduler tests.
- SIGTERM/SIGINT handling is **mandatory** (PID 1 in the image ignores
  default-disposition signals). Drain order: HTTP server (≤ 3 s; SSE never
  ends by itself) → loops → join → WAL checkpoint → DB close within 5 s.
- Live tail: `internal/events.Broker` publishes a `check_id` (from ping handling
  and the scan loop, only when there are subscribers); the SSE handler sends a
  data-less `changed` and the browser re-fetches the fragment. In-process only.

### Scheduling & display
- `scheduler.DueTime` = last success (else creation) + period/cron + grace.
  Cron is 6-field (`sec min hour dom mon dow`) in the check's timezone.
- Scan interval cascades check → project → global → env; nag interval check →
  project → global (no env default, off when unset). Non-positive falls through.
- Duration fields accept seconds or `5m`/`1h30m`/`2d` (`internal/duration`),
  stored as seconds, re-rendered losslessly; `view` formatting is display only.
- Dashboard ordering and `q`/status filtering happen in Go; store lists stay in
  id order (shared by other pages and the API). Loads are batched: keep the
  query count fixed; the heartbeat reads a narrow ping summary, not `body`.
- Check-page heartbeat: never narrow the window to `kind IN ('success','fail')`:
  run durations need the `start` pings. No run count in the caption.
- `view` adds display-only `late`/`running`; precedence
  `Paused > Down > Running > Late > Up`. Next due uses `scheduler.DueTime`, not
  `checks.next_due_at` (often NULL).

### Notifications (`internal/notify`)
- Six notifiers; delivery is fire-and-forget (goroutine) with a retry policy.
- Check creation (web and API) binds all project channels.
- Channel edit: **blank field keeps the stored value; never re-render a secret**
  (webhook/Slack URLs count). One validator; templates see only the edit view.
  `kind` is immutable. See ARCHITECTURE.md "Editing a channel without leaking
  its secrets".
- Message = `notify` event text, max **four lines**; the detail is built at the
  call site from the **pre-update** snapshot, never re-read during delivery.
  Webhook payload changes are additive only. See ARCHITECTURE.md "What a
  notification says".
- SSRF posture is pinned by `internal/notify/ssrf_test.go`: webhooks may target
  loopback and follow redirects exactly like the Rust build. Changing that is a
  product decision, and the test flips with it.

## Config (`internal/config`)

Test parsing through the loader that takes a map, not the real environment.
Unparseable duration env vars fall back to defaults. The README's table is the
user-facing reference; `/admin`'s Environment card shows these read-only,
secrets only as configured/not-set.

`DATABASE_URL` (`sqlite://pingward.sqlite3?mode=rwc`), `PINGWARD_BIND`
(`127.0.0.1:8080`), `PINGWARD_BASE_URL` (`http://localhost:8080`; ping URLs and
notification links), `PINGWARD_SCAN_INTERVAL` (30s), `PINGWARD_PRUNE_INTERVAL_SECS`
(3600), `PINGWARD_LOG_LEVEL` (`debug`/`info`/`warn`/`error`, default `info`;
environment only), `PINGWARD_LOG_FORMAT` (`json` for slog JSON, anything else
slog text), `PINGWARD_TRUSTED_PROXIES`, `PINGWARD_FORWARD_AUTH_HEADER`,
`PINGWARD_FORWARD_AUTH_LOGOUT_URL`, `PINGWARD_SECRET` (≥16 bytes, else random
per process → every restart signs everyone out), `PINGWARD_COOKIE_SECURE`
(default from `PINGWARD_BASE_URL` scheme; also switches the cookie name to
`__Host-pingward_session`), `PINGWARD_HSTS_MAX_AGE` (0 = off),
`PINGWARD_SMTP_{HOST,FROM,PORT,USERNAME,PASSWORD,TLS}` (host + from enable email).

**When you add, rename or remove an environment variable, update
`legacyEnvVars` in `cmd/pingward/env.go` in the same change.** Startup fails
when a retired variable is set (today: `RUST_LOG` → `PINGWARD_LOG_LEVEL`), so a
silently ignored setting cannot hide. Update the README table and `/admin`'s
Environment card too.
