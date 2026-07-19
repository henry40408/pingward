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
  (`Authorization: Bearer pw_…`), created and revoked from the **API keys** page.
  An OpenAPI document (`/api/openapi.json`) and an interactive Scalar reference
  (`/api/docs`) are available to logged-in users.
- **Multi-user with admin** — session-cookie auth (argon2), per-user project /
  check ownership (other users' resources return 404, not 403), plus an
  `/admin/*` area for cross-user management. Optional trusted forward-auth header
  auto-provisions a passwordless user.
- **SQLite or Postgres** — one connection pool dispatches by URL scheme; no code
  change to switch backends.
- **Configurable retention** — a prune loop deletes old pings and notifications.

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
| `PINGWARD_FORWARD_AUTH_HEADER` + `PINGWARD_TRUSTED_PROXIES` | — | Trusted forward-auth header and the proxy CIDRs allowed to set it. |
| `PINGWARD_SMTP_*` | — | Instance SMTP for the email channel (`HOST`/`FROM` required to enable; port/TLS defaulted). |

Duration-valued settings (scan/nag/prune intervals and per-check period, grace,
max-runtime) accept either raw seconds or a human-readable string (`5m`,
`1h30m`, `2d`).

## Development

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

## License

[MIT](LICENSE.txt)
