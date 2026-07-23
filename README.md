# pingward

> A self-hosted, healthchecks-style uptime & cron monitor, built in Rust.

[![CI](https://github.com/henry40408/pingward/actions/workflows/ci.yml/badge.svg)](https://github.com/henry40408/pingward/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/henry40408/pingward/graph/badge.svg)](https://codecov.io/gh/henry40408/pingward)
[![Release](https://img.shields.io/github/v/release/henry40408/pingward)](https://github.com/henry40408/pingward/releases/latest)
[![License](https://img.shields.io/github/license/henry40408/pingward)](LICENSE.txt)
[![Rust toolchain](https://img.shields.io/badge/dynamic/toml?url=https://raw.githubusercontent.com/henry40408/pingward/main/rust-toolchain.toml&query=$.toolchain.channel&label=rust%20toolchain&logo=rust)](https://www.rust-lang.org/)
[![Docker](https://img.shields.io/badge/docker-ghcr.io-blue.svg)](https://ghcr.io/henry40408/pingward)
[![Casual Maintenance Intended](https://casuallymaintained.tech/badge.svg)](https://casuallymaintained.tech/)
[![Vibe Coded](https://img.shields.io/badge/vibe_coded-Claude-d97757?logo=anthropic&logoColor=white)](https://claude.com/claude-code)

Monitor cron jobs, backups, and any recurring task by having them "ping" a
per-check URL. A background loop marks a check **down** when a ping is overdue
and delivers a notification through the channels bound to that check. Ships as a
single binary with an embedded, server-rendered web UI — dark/light follows your
OS preference and the layout adapts to phones.

## Features

- **Two schedule kinds** — fixed `period` (interval) or a 6-field `cron`
  expression (`sec min hour dom mon dow`), evaluated in each check's timezone,
  with a configurable grace window and max-runtime.
- **Machine ping endpoints** — `success` / `fail` / `start` / `log` and
  `exitcode` pings (`/ping/<uuid>[/<kind>]`); a `start` ping opens an in-flight
  run so an overrun can be detected.
- **Six notification channels** — webhook, Telegram, Slack, ntfy, Pushover, and
  email (SMTP). Delivery is fire-and-forget with a retry policy, so a ping
  response is never blocked on notification I/O.
- **REST API** — a bearer-authenticated `/api/v1` for projects, checks,
  channels, and ping/notification history: read them, create/update/delete them,
  and drive the check actions (pause, resume, acknowledge, regenerate ping URL,
  bind channels). Authenticate with account-bound API keys
  (`Authorization: Bearer pw_…`), created and revoked from the **Account** page.
  An OpenAPI document (`/api/openapi.json`) and an interactive Scalar reference
  (`/api/docs`) are available to logged-in users.
- **Multi-user with admin** — session-cookie auth (argon2), per-user project /
  check ownership (other users' resources return 404, not 403), plus an
  `/admin/*` area for cross-user management. Optional trusted forward-auth header
  auto-provisions a passwordless user.
- **SQLite or Postgres** — one connection pool dispatches by URL scheme; no code
  change to switch backends.
- **Configurable retention** — a prune loop deletes old pings and notifications.

## Screenshots

The UI is server-rendered and embedded in the binary — no build step, no
JavaScript bundle. Dark/light follows your OS preference (with a manual
override), and the layout adapts to phones.

![Dashboard — one group per project, status tiles, per-check heartbeat strips and a name/status filter](docs/screenshots/dashboard-dark.png)

![A down check — status and acknowledge action, its ping URL, the heartbeat of the last runs, and the notification channels bound to it](docs/screenshots/check-dark.png)

![Recent pings and recent notifications for a check, with the failed run's captured output expanded](docs/screenshots/check-history-dark.png)

![Project page — its checks and the notification channels defined on it](docs/screenshots/project-dark.png)

![Admin — site-wide totals, check and notification health, and the scheduler heartbeat](docs/screenshots/admin-dark.png)

<table>
  <tr>
    <td width="60%"><img src="docs/screenshots/dashboard-light.png" alt="The dashboard in the light theme"></td>
    <td width="20%"><img src="docs/screenshots/dashboard-mobile.png" alt="The dashboard on a phone-width viewport"></td>
    <td width="20%"><img src="docs/screenshots/check-mobile.png" alt="A check page on a phone-width viewport"></td>
  </tr>
  <tr>
    <td align="center">Light theme</td>
    <td align="center" colspan="2">Phone layout</td>
  </tr>
</table>

## Quick Start

### Docker

Multi-arch (`amd64` / `arm64`) images are published to GitHub Container Registry:

```sh
docker run -d \
  --name pingward \
  -p 8080:8080 \
  -v pingward-data:/data \
  -e PINGWARD_BASE_URL=https://pingward.example.com \
  ghcr.io/henry40408/pingward:latest
```

The container binds HTTP on `0.0.0.0:8080` and stores its SQLite database at
`/data/pingward.sqlite3`. Set `PINGWARD_BASE_URL` to the externally reachable URL
so the ping URLs rendered in the UI are correct. Open the UI and create the first
admin account on first run.

To use Postgres instead, pass `-e DATABASE_URL=postgres://user:pass@host/db`.

`docker stop` / `docker compose down` shuts down gracefully: pingward handles
SIGTERM, stops accepting new connections, lets in-flight requests and the
current scan/prune pass finish, and closes the database pool — which
checkpoints SQLite's WAL and removes the `-wal`/`-shm` files — typically well
under a second, rather than waiting out Docker's 10s grace period.

### From source

```sh
cargo run
# defaults: SQLite file pingward.sqlite3, bind 127.0.0.1:8080
```

## Configuration

All configuration is via environment variables:

| Variable | Default | Purpose |
| --- | --- | --- |
| `DATABASE_URL` | `sqlite://pingward.sqlite3?mode=rwc` | SQLite (`sqlite://…`) or Postgres (`postgres://…`) — backend is chosen by scheme. |
| `PINGWARD_BIND` | `127.0.0.1:8080` | Listen address for the HTTP server. |
| `PINGWARD_BASE_URL` | `http://localhost:8080` | Base URL used to render ping URLs in the UI. |
| `PINGWARD_SCAN_INTERVAL` | `30s` | How often the scan loop re-evaluates checks. Accepts raw seconds or a duration (`5m`, `1h30m`). |
| `PINGWARD_PRUNE_INTERVAL_SECS` | — | How often the prune loop runs. |
| `PINGWARD_LOG_FORMAT` | `text` | Log renderer: `text` (human-readable) or `json` (one JSON object per line for a log aggregator). Verbosity is set with `RUST_LOG`. |
| `PINGWARD_TRUSTED_PROXIES` | — | Comma-separated addresses or CIDR blocks whose `X-Forwarded-For` (and forward-auth header) is believed. See below. |
| `PINGWARD_FORWARD_AUTH_HEADER` | — | Header carrying a pre-authenticated username; honoured only from a trusted proxy. |
| `PINGWARD_SECRET` | generated per process | Signing key for session cookies and CSRF tokens; at least 16 bytes. See below. |
| `PINGWARD_SMTP_*` | — | Instance SMTP for the email channel (`HOST`/`FROM` required to enable; port/TLS defaulted). |

### `PINGWARD_SECRET`

Session cookies are signed with this key, and each session's CSRF token is
derived from it. **Set it on any real deployment:**

```
PINGWARD_SECRET=$(openssl rand -hex 32)
```

Leave it unset and pingward generates a fresh key at every start, which
invalidates every signed-in browser session — so each restart signs all users
out (a warning at startup says so). A value shorter than 16 bytes is ignored
and treated the same way. Changing the key has the same effect and is the way
to force a global sign-out on purpose.

API keys are not affected by any of this: they are independent bearer tokens,
so programmatic clients keep working across restarts and key changes.

Duration-valued settings (scan/nag/prune intervals and per-check period, grace,
max-runtime) accept either raw seconds or a human-readable string (`5m`,
`1h30m`, `2d`).

### Running behind a reverse proxy

The address recorded for a login session (**Account** page) and for each ping
(the **Source** column on a check) is the socket peer — which behind a reverse
proxy is the proxy itself, the same value on every row. To record the real
client instead, list the proxy in `PINGWARD_TRUSTED_PROXIES`; its
`X-Forwarded-For` is then believed, and its first entry is stored. A request
from any other address has its `X-Forwarded-For` ignored, so a public `/ping/*`
endpoint cannot be used to forge a source address.

Entries are bare addresses (`10.0.0.1`, `::1`) or CIDR blocks
(`172.16.0.0/12`, `fd00::/8`), comma-separated. Prefer a block when the proxy
runs in a container: its address comes from the bridge network's pool and
changes whenever the network is recreated. Hostnames are **not** resolved and
match nothing. For pingward and Caddy in the same Compose project:

```yaml
environment:
  PINGWARD_TRUSTED_PROXIES: "172.16.0.0/12"
```

Confirm the peer's actual address with `docker inspect -f
'{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' <caddy-container>`,
and note that `/admin` shows the value pingward parsed on its **Environment**
card.

### Forward authentication

To let an authentication gateway (Authelia, Authentik, `oauth2-proxy`, …) sign
users in, point `PINGWARD_FORWARD_AUTH_HEADER` at the header your proxy sets:

```yaml
environment:
  PINGWARD_TRUSTED_PROXIES: "172.16.0.0/12"
  PINGWARD_FORWARD_AUTH_HEADER: "Remote-User"
```

The header is honoured **only** from an address listed in
`PINGWARD_TRUSTED_PROXIES` — otherwise anyone could set it and log in as
anybody. A username seen for the first time gets a non-admin, password-less
account provisioned automatically; promote it from `/admin`.

Such a user is given a normal pingward session on first request, so the
**Account** page lists it and it can be revoked like any other. Note that
revoking it only ends that session — the next request through the proxy is
authenticated again by the header. Sign-out is the gateway's job; there is
nothing pingward can log you out of.

#### Exclude the machine endpoints from the gateway

**This part is not optional.** An authentication gateway in front of pingward
protects *everything* by default, including the endpoints that have no browser
and no session to redirect. Leave them covered and your monitoring silently
stops working: every `curl` from a cron job gets the gateway's login page (a
`302`, which most ping scripts treat as success), so checks never receive a
heartbeat and pingward reports them all down.

Exclude at least these:

| Path | Why |
| --- | --- |
| `/ping/*` | The heartbeat endpoints. Public and unauthenticated by design — this is what your jobs call. |
| `/api/v1/*` | Bearer-token API. It authenticates with its own key and never reads a session cookie. |
| `/healthz` | Container/uptime health probes. |

With Authelia, add a `bypass` rule *above* your catch-all — rules are matched
in order, so a `bypass` listed after the `one_factor` rule never fires:

```yaml
access_control:
  default_policy: deny
  rules:
    - domain: pingward.example.com
      resources:
        - '^/ping/.*$'
        - '^/api/v1/.*$'
        - '^/healthz$'
      policy: bypass

    - domain: pingward.example.com
      policy: one_factor
```

If you also serve the OpenAPI docs to machines, add `^/api/openapi\.json$`.
The equivalent in other gateways is the same idea under a different name —
`skip_auth_routes` in `oauth2-proxy`, an unauthenticated path in Authentik.

To verify, `curl -sS -o /dev/null -w '%{http_code}\n' https://pingward.example.com/ping/<uuid>`
from outside your network: `200` means the bypass works, `302` means the
gateway is still intercepting it.

## REST API

pingward exposes a bearer-authenticated JSON API under `/api/v1`. Create a key
on the **Account** page (it is shown once), then send it as a bearer token:

```sh
BASE=https://pingward.example.com
KEY=pw_…                                   # from the Account page

# Create a project, then a check under it
pid=$(curl -s -X POST "$BASE/api/v1/projects" \
  -H "authorization: Bearer $KEY" -H "content-type: application/json" \
  -d '{"name":"Backups","scan_interval_secs":"5m"}' | jq -r .id)

curl -s -X POST "$BASE/api/v1/projects/$pid/checks" \
  -H "authorization: Bearer $KEY" -H "content-type: application/json" \
  -d '{"name":"nightly","period_secs":"1h","grace_secs":"5m"}'

# Read a check's ping history (keyset pagination)
curl -s "$BASE/api/v1/checks/1/pings?limit=20" -H "authorization: Bearer $KEY"

# Drive a check: pause / resume / acknowledge / regenerate the ping URL
curl -s -X POST "$BASE/api/v1/checks/1/pause" -H "authorization: Bearer $KEY"
```

Duration fields (`period_secs`, `grace_secs`, the interval overrides) accept
either an integer number of seconds or a human-readable string (`"5m"`,
`"1h30m"`). Paginated list responses carry `has_newer`/`has_older` plus
`next_after`/`next_before` cursor ids — pass `next_before` as `?before=` to
fetch the next (older) page. An admin key may reach another user's resources;
every such cross-user access is recorded in the audit log.

The full operation list — with request/response schemas — is served as an
OpenAPI document at `/api/openapi.json`, with an interactive
[Scalar](https://github.com/scalar/scalar) reference at `/api/docs` (both
require a logged-in session).

## Development

See [ARCHITECTURE.md](ARCHITECTURE.md) for the code map and how the pieces
fit together.

```sh
cargo build                       # required after any template or route change
cargo run                         # start the server
cargo fmt --all --check           # formatting (enforced in CI)
cargo clippy --all-targets -- -D warnings
cargo nextest run                 # Rust tests (use nextest, not `cargo test`)
cargo deny check                  # supply-chain / license checks
```

Postgres integration tests (`tests/pg_store.rs`) and SMTP delivery tests
(`tests/smtp_e2e.rs`) skip unless their backends are configured. Start both with
`docker compose up -d`, then export `TEST_DATABASE_URL`,
`PINGWARD_TEST_SMTP_HOST=localhost`, `PINGWARD_TEST_SMTP_PORT=1025`, and
`PINGWARD_TEST_MAILPIT_API=http://localhost:8025`.

### End-to-end tests

Browser E2E (Playwright + playwright-bdd) lives in `e2e/`; each scenario spawns a
fresh compiled binary against a temporary SQLite database:

```sh
cd e2e && npm test
```

### Regenerating the screenshots and the app icon

The images in `docs/screenshots/` come from a repeatable pipeline: it wipes a
throwaway SQLite database, creates the first admin through the product's own
`/setup` form, seeds backdated demo history with the `sqlite3` CLI, boots
`pingward` on a throwaway port, and re-captures every framed shot with
Playwright. Re-run it after a UI change and commit the updated PNGs:

~~~sh
cargo build                                  # the UI is compiled into the binary
cd e2e
npm ci && npx playwright install chromium    # first time only
npm run screenshots
~~~

`assets/apple-touch-icon.png` is rendered from `assets/favicon.svg` by the same
Chromium. Re-run it after editing the SVG:

~~~sh
cd e2e && npm run icons
~~~

## License

[MIT](LICENSE.txt)
