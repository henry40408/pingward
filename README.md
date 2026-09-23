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

Monitor cron jobs, backups and other recurring tasks by having them "ping" a
per-check URL. When a ping is overdue, pingward marks the check **down** and
notifies the channels bound to it. Ships as a single binary with an embedded,
server-rendered web UI (light/dark, phone-friendly, works without JavaScript).

## Features

- **Schedules** — fixed `period` or 6-field `cron` (`sec min hour dom mon dow`)
  in the check's timezone, with grace window, max-runtime and optional repeat
  reminders.
- **Ping endpoints** — `/ping/<uuid>` (success), `/fail`, `/start`, `/log`, and
  `/<exit-code>`; a `start` opens a run so overruns are detected. `POST` bodies
  are kept as run output.
- **Notifications** — webhook, Telegram, Slack, ntfy, Pushover, email (SMTP),
  with retries. Stored credentials are never shown again; leave an edit field
  blank to keep it.
- **REST API** — bearer-authenticated `/api/v1` with an OpenAPI document and
  Scalar reference (see [REST API](#rest-api)).
- **Multi-user** — per-user ownership, an `/admin` area with an audit trail, and
  optional forward-auth. Passwords are 15–128 characters (length is the only
  rule). Creating an API key re-asks for the password; admin actions that grant
  access (create user, reset password, promote) need a 15-minute re-unlock,
  while removing access never does. Login is throttled per address (5/min) and
  per account (10/15 min); failures are logged under `pingward::auth`.
- **SQLite or Postgres**, chosen by `DATABASE_URL` scheme.
- **Retention** — optional pruning of old pings, notifications and audit entries
  (off by default; set on `/admin`).

## Screenshots

![Dashboard — one group per project, status tiles, per-check heartbeat strips and a name/status filter](docs/screenshots/dashboard-dark.png)

![A down check — status and acknowledge action, its ping URL, the heartbeat of the last runs, and the notification channels bound to it](docs/screenshots/check-dark.png)

![Recent pings and recent notifications for a check, with the failed run's captured output expanded](docs/screenshots/check-history-dark.png)

![Project page — its checks and the notification channels defined on it](docs/screenshots/project-dark.png)

![Admin — site-wide totals, check and notification health, and the scheduler heartbeat](docs/screenshots/admin-dark.png)

![The admin audit trail — who reached across to whose data, filterable by actor and action, with the request behind an entry expanded](docs/screenshots/admin-audit-dark.png)

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

Multi-arch (`amd64`/`arm64`) images on GitHub Container Registry:

```sh
docker run -d \
  --name pingward \
  -p 8080:8080 \
  -v pingward-data:/data \
  -e PINGWARD_BASE_URL=https://pingward.example.com \
  -e PINGWARD_SECRET=replace-with-openssl-rand-hex-32 \
  ghcr.io/henry40408/pingward:latest
```

The container listens on `0.0.0.0:8080` and keeps its SQLite database at
`/data/pingward.sqlite3`. Set `PINGWARD_BASE_URL` to the external URL so
rendered ping URLs are correct, and generate `PINGWARD_SECRET` once and keep
it — a new value signs everyone out. Then open the UI and create the first
admin.
For Postgres, pass `-e DATABASE_URL=postgres://user:pass@host/db`.

pingward handles SIGTERM, so `docker stop` finishes in-flight work and closes
the database cleanly (SQLite's `-wal`/`-shm` files are removed) instead of
waiting out Docker's 10s timeout.

### From source

```sh
cargo run   # SQLite file pingward.sqlite3, bind 127.0.0.1:8080
```

## Configuration

All configuration is via environment variables:

| Variable | Default | Purpose |
| --- | --- | --- |
| `DATABASE_URL` | `sqlite://pingward.sqlite3?mode=rwc` | `sqlite://…` or `postgres://…`; the scheme picks the backend. |
| `PINGWARD_BIND` | `127.0.0.1:8080` (image: `0.0.0.0:8080`) | HTTP listen address. |
| `PINGWARD_BASE_URL` | `http://localhost:8080` | External URL, used for ping URLs and links in notifications. Its scheme also sets the cookie's `Secure` flag. |
| `PINGWARD_SECRET` | random per process | Signs session cookies and CSRF tokens; at least 16 bytes. See below. |
| `PINGWARD_SCAN_INTERVAL` | `30s` | How often checks are re-evaluated. |
| `PINGWARD_PRUNE_INTERVAL_SECS` | `3600` | How often the retention prune runs. |
| `PINGWARD_LOG_FORMAT` | `full` | `full`, `compact`, `pretty`, or `json` (one object per line). |
| `RUST_LOG` | `error,pingward=info` | Log filter by target and level, e.g. `pingward=debug`. |
| `PINGWARD_TRUSTED_PROXIES` | — | Comma-separated addresses or CIDR blocks whose `X-Forwarded-For` and forward-auth header are believed. |
| `PINGWARD_FORWARD_AUTH_HEADER` | — | Header carrying a pre-authenticated username (trusted proxies only). |
| `PINGWARD_FORWARD_AUTH_LOGOUT_URL` | — | Where **Log out** redirects, e.g. your gateway's sign-out URL. |
| `PINGWARD_COOKIE_SECURE` | from `PINGWARD_BASE_URL` | Force the cookie's `Secure` flag (`true`/`false`/`1`/`0`). |
| `PINGWARD_HSTS_MAX_AGE` | `0` (off) | Send `Strict-Transport-Security` with this `max-age`. |
| `PINGWARD_SMTP_HOST`, `PINGWARD_SMTP_FROM` | — | Both required to enable the email channel. |
| `PINGWARD_SMTP_TLS` | `starttls` | `starttls`, `tls` (implicit) or `none`. |
| `PINGWARD_SMTP_PORT` | `587` (`465` with `tls`) | SMTP port. |
| `PINGWARD_SMTP_USERNAME`, `PINGWARD_SMTP_PASSWORD` | — | Optional SMTP AUTH. |

Durations (env intervals, `PINGWARD_HSTS_MAX_AGE`, and the period, grace,
max-runtime and interval fields in the UI and API) accept raw seconds or
strings like `5m`, `1h30m`, `2d`. An unparseable env value falls back to its
default. `/admin` shows the parsed values on its **Environment** card.

Session creation, renewal and destruction are logged under `pingward::session`
(hashed session id, IP, user agent); `RUST_LOG=info,pingward::session=warn`
silences them.

### `PINGWARD_SECRET`

**Set it on any real deployment** (`openssl rand -hex 32`). Unset or shorter
than 16 bytes, a random key is generated at each start, so every restart signs
all users out (startup warns). Changing it signs everyone out on purpose. API
keys are unaffected.

### Running behind a reverse proxy

- **Client addresses.** Behind a proxy every request appears to come from the
  proxy. List it in `PINGWARD_TRUSTED_PROXIES` so its `X-Forwarded-For` is used
  for session and ping source IPs (first entry) and for login rate limiting
  (last entry, assuming exactly one trusted proxy). Otherwise all clients share
  one rate-limit bucket: five failed logins anywhere lock sign-in for a minute.
  Headers from other addresses are ignored, so `/ping/*` callers cannot forge
  their source.
- Entries are addresses (`10.0.0.1`, `::1`) or CIDR blocks (`172.16.0.0/12`,
  `fd00::/8`). Hostnames are not resolved. For a proxy container prefer a
  block, since its address changes when the network is recreated:

  ```yaml
  environment:
    PINGWARD_TRUSTED_PROXIES: "172.16.0.0/12"
  ```

- **HTTPS.** Use an `https://` `PINGWARD_BASE_URL`, or the cookie will lack
  `Secure`. With `Secure` on, the cookie is named `__Host-pingward_session`, so
  toggling `PINGWARD_COOKIE_SECURE` or the base URL's scheme signs everyone out
  once.

#### Security headers

Every response carries `X-Content-Type-Options: nosniff`,
`X-Frame-Options: DENY`, `Referrer-Policy: same-origin` and a
`Permissions-Policy` disabling geolocation, camera, microphone, payment and
USB. The web UI adds a Content-Security-Policy with `script-src 'self'`. Headers
already set by your proxy win. `/api/docs` is outside the CSP because Scalar
loads from `cdn.jsdelivr.net`; offline deployments can use
`/api/openapi.json` with a local viewer.

#### HSTS

pingward does not terminate TLS, so it sends no HSTS header by default. Set it
on the proxy:

```caddyfile
# Caddy
header Strict-Transport-Security "max-age=31536000"
```

```nginx
# nginx
add_header Strict-Transport-Security "max-age=31536000" always;
```

```yaml
# Traefik (dynamic config)
http:
  middlewares:
    hsts:
      headers:
        stsSeconds: 31536000
```

`includeSubDomains` and `preload` are effectively irreversible — add them only
when every subdomain is permanently HTTPS.

If the proxy cannot add headers, set `PINGWARD_HSTS_MAX_AGE` (e.g. `300`, or
`365d`). Ramp up `300` → `86400` → `31536000`; a cached policy cannot be
withdrawn before it expires. pingward never sends `includeSubDomains` or
`preload`.

### Forward authentication

To let a gateway (Authelia, Authentik, `oauth2-proxy`, …) sign users in:

```yaml
environment:
  PINGWARD_TRUSTED_PROXIES: "172.16.0.0/12"
  PINGWARD_FORWARD_AUTH_HEADER: "Remote-User"
```

The header is honoured **only** from a trusted proxy. An unknown username gets
a passwordless non-admin account (promote it on `/admin`). Revoking its session
on **Account** lasts only until the next proxied request.

#### Signing out

**Log out** deletes the local session, but the gateway re-authenticates the next
request. Point pingward at the gateway's sign-out URL:

```yaml
environment:
  PINGWARD_FORWARD_AUTH_LOGOUT_URL: "https://auth.example.com/logout"
```

Authelia: `https://<domain>/logout`; Authentik:
`https://<domain>/application/o/<slug>/end-session/`; `oauth2-proxy`:
`https://<domain>/oauth2/sign_out`. When set, it applies to every account,
including password ones. Unset, logging out lands on `/login` — or, for a
gateway-authenticated request, on `/` with a notice that only the gateway can
end the session.

#### Exclude the machine endpoints from the gateway

**Required.** Otherwise every ping gets the gateway's login redirect (a `302`
most scripts treat as success), no heartbeat arrives, and every check goes
down. Bypass at least:

| Path | Why |
| --- | --- |
| `/ping/*` | Heartbeat endpoints, unauthenticated by design. |
| `/api/v1/*` | Authenticates with its own bearer key. |
| `/healthz` | Health probes. |

Authelia (the `bypass` rule must come **before** the catch-all):

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

(`skip_auth_routes` in `oauth2-proxy`, an unauthenticated path in Authentik.)
Verify from outside your network:
`curl -sS -o /dev/null -w '%{http_code}\n' https://pingward.example.com/ping/<uuid>`
should print `200`, not `302`.

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

Duration fields accept seconds or strings like `"5m"`. Paginated lists return
`has_newer`/`has_older` and `next_after`/`next_before` cursors; pass
`next_before` as `?before=` for the next (older) page. An admin key can reach
other users' resources, and each such access is audited.

The full schema is at `/api/openapi.json`, with an interactive
[Scalar](https://github.com/scalar/scalar) reference at `/api/docs` (both need a
logged-in session).

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

Postgres (`tests/pg_store.rs`) and SMTP (`tests/smtp_e2e.rs`) tests skip unless
configured: `docker compose up -d`, then export
`TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres`,
`PINGWARD_TEST_SMTP_HOST=localhost`, `PINGWARD_TEST_SMTP_PORT=1025`,
`PINGWARD_TEST_MAILPIT_API=http://localhost:8025`.

### End-to-end tests

Browser tests (cucumber + thirtyfour) live in `e2e/`, a separate cargo
workspace. Each scenario runs a freshly built binary against a temporary SQLite
database. Requires a local Chrome or Chromium (the driver is downloaded
automatically; on macOS: `brew install --cask ungoogled-chromium`).

```sh
cargo build && cd e2e && cargo test --test e2e
```

### Screenshots and app icon

`docs/screenshots/` is generated by seeding demo data into a throwaway database
and capturing each page. Re-run after UI changes and commit the PNGs.
`assets/apple-touch-icon.png` is rendered from `assets/favicon.svg`.

```sh
cargo build && cd e2e && cargo run --bin screenshots
cd e2e && cargo run --bin icons
```

## License

[MIT](LICENSE.txt)
