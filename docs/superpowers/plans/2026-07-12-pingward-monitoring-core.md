# pingward Monitoring Core — Implementation Plan (Plan 1 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the headless monitoring engine: jobs check in over HTTP, the app detects overdue jobs on a background loop and fires a webhook notification.

**Architecture:** Single `axum` binary. Inbound pings hit public `/ping/<uuid>` endpoints keyed by a per-check UUIDv4 secret and are recorded, driving a check state machine (`new`/`up`/`down`/`paused`). A background `tokio` loop periodically computes each active check's due time (period- or cron-based) and transitions overdue checks to `down`, dispatching notifications through a `Notifier` trait. State is persisted in SQLite via `sqlx`; timestamps and enums are stored as TEXT for portability.

**Tech Stack:** Rust, `tokio`, `axum` 0.8, `sqlx` 0.9 (SQLite), `chrono` + `chrono-tz`, `cron`, `uuid` (v4), `reqwest`, `serde`/`serde_json`, `tracing`. Tests: `axum-test`, `wiremock`.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-07-12-pingward-design.md` — this plan implements §3–§6, §9–§11 (webhook only) for the monitoring core. Auth, Web UI, remaining channels, and PostgreSQL parity are Plans 2–3.
- **Dependency cooldown:** No crate version published < 7 days ago (today 2026-07-12). Before pinning any version, check publish date (`cargo info <crate>`); if the latest is younger than 7 days, pin the most recent version that is ≥ 7 days old.
- **Framework versions match existing repos:** `axum` 0.8, `sqlx` 0.9, `askama` 0.16 (askama not used until Plan 2).
- **Portability:** all timestamps stored as RFC3339 UTC TEXT; all enums stored as lowercase TEXT. No DB-specific column types beyond `INTEGER`/`TEXT` so the same schema shape ports to PostgreSQL in Plan 3.
- **UUID:** `ping_uuid` is UUIDv4 (`uuid::Uuid::new_v4()`), stored hyphenated, `UNIQUE`.
- **Rust hygiene:** `cargo fmt` before every commit; tests run with `cargo nextest run` (fall back to `cargo test` only if nextest unavailable).
- **Determinism:** all time-dependent logic (due computation, scan) takes an explicit `now: DateTime<Utc>` parameter so tests use a fixed clock; only `main`'s loop calls `Utc::now()`.
- **Commits:** GPG-signed (default git config already signs). Stage files explicitly by name — never `git add -A`/`.`.

---

## File Structure

```
pingward/
  Cargo.toml
  migrations/sqlite/0001_init.sql   -- schema (SQLite)
  src/
    main.rs        -- bootstrap: config, pool, migrate, router, spawn scan loop
    config.rs      -- Config struct + from_env()
    error.rs       -- AppError + IntoResponse
    db.rs          -- Pool alias, connect(), migrate()
    models.rs      -- Check/Ping structs, ScheduleKind/CheckStatus/PingKind enums
    store.rs       -- Store: all SQL queries (checks, pings)
    ping.rs        -- axum handlers for /ping/* + ping-handling service
    scheduler.rs   -- due_time(), scan_once(), run_scan_loop()
    notify.rs      -- Notifier trait, NotificationEvent, WebhookNotifier, dispatch()
  tests/
    ping_api.rs    -- integration: ping endpoints
    scheduler.rs   -- integration: overdue → down → notify
```

Responsibilities: `store.rs` is the only module that writes SQL. `scheduler.rs` and `notify.rs` are pure/trait-based and mockable. `ping.rs` owns the success/fail state transitions triggered by inbound pings; `scheduler.rs` owns the overdue transition. `main.rs` only wires things together.

---

### Task 1: Project scaffold + health endpoint

**Files:**
- Create: `Cargo.toml`, `src/main.rs`, `tests/ping_api.rs`

**Interfaces:**
- Produces: `pingward::app() -> axum::Router` (test entry point); `GET /healthz` → `200 "ok"`.

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "pingward"
version = "0.1.0"
edition = "2021"

[lib]
name = "pingward"
path = "src/lib.rs"

[[bin]]
name = "pingward"
path = "src/main.rs"

[dependencies]
axum = "0.8"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal", "time", "net", "sync"] }
sqlx = { version = "0.9", default-features = false, features = ["runtime-tokio", "sqlite", "tls-rustls-aws-lc-rs", "migrate"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
chrono-tz = "0.10"
cron = "0.15"
uuid = { version = "1", features = ["v4"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
axum-test = "21"
wiremock = "0.6"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "test-util"] }
```

Note: verify each version's publish date is ≥ 7 days old before running `cargo build`; downgrade any that isn't per Global Constraints.

- [ ] **Step 2: Write the failing test** — `tests/ping_api.rs`

```rust
use axum_test::TestServer;

#[tokio::test]
async fn healthz_returns_ok() {
    let server = TestServer::new(pingward::app()).unwrap();
    let res = server.get("/healthz").await;
    res.assert_status_ok();
    res.assert_text("ok");
}
```

- [ ] **Step 3: Run it — expect FAIL** (compile error: `app` not found)

Run: `cargo nextest run --test ping_api healthz_returns_ok`
Expected: FAIL (unresolved `pingward::app`).

- [ ] **Step 4: Create `src/lib.rs` + `src/main.rs`**

`src/lib.rs`:
```rust
use axum::{routing::get, Router};

pub fn app() -> Router {
    Router::new().route("/healthz", get(|| async { "ok" }))
}
```

`src/main.rs`:
```rust
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info".into()),
    ).init();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, pingward::app()).await.unwrap();
}
```

Move `path = "src/main.rs"` bin and `src/lib.rs` lib as in Cargo.toml above.

- [ ] **Step 5: Run test — expect PASS**

Run: `cargo nextest run --test ping_api healthz_returns_ok`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add Cargo.toml Cargo.lock src/lib.rs src/main.rs tests/ping_api.rs
git commit -m "feat: scaffold pingward binary with health endpoint"
```

---

### Task 2: Config from environment

**Files:**
- Create: `src/config.rs`, `src/error.rs`
- Modify: `src/lib.rs` (add `pub mod config; pub mod error;`)

**Interfaces:**
- Produces: `config::Config { database_url: String, bind: String, base_url: String, scan_interval_secs: u64, forward_auth_header: Option<String>, trusted_proxies: Vec<String> }`; `Config::from_env() -> Config`.
- Produces: `error::AppError` (enum) implementing `axum::response::IntoResponse` and `From<sqlx::Error>`.

- [ ] **Step 1: Write the failing test** — append to `src/config.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_apply_when_unset() {
        let c = Config::from_map(|_| None);
        assert_eq!(c.scan_interval_secs, 30);
        assert_eq!(c.bind, "127.0.0.1:8080");
        assert_eq!(c.database_url, "sqlite://pingward.db?mode=rwc");
    }

    #[test]
    fn env_overrides_defaults() {
        let c = Config::from_map(|k| match k {
            "PINGWARD_SCAN_INTERVAL" => Some("10".into()),
            "PINGWARD_TRUSTED_PROXIES" => Some("10.0.0.1,10.0.0.2".into()),
            _ => None,
        });
        assert_eq!(c.scan_interval_secs, 10);
        assert_eq!(c.trusted_proxies, vec!["10.0.0.1", "10.0.0.2"]);
    }
}
```

- [ ] **Step 2: Run — expect FAIL** (`Config` undefined)

Run: `cargo nextest run --lib config`
Expected: FAIL.

- [ ] **Step 3: Implement `src/config.rs`**

```rust
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub bind: String,
    pub base_url: String,
    pub scan_interval_secs: u64,
    pub forward_auth_header: Option<String>,
    pub trusted_proxies: Vec<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self::from_map(|k| std::env::var(k).ok())
    }

    /// Testable core: `get` resolves an env key to an optional value.
    pub fn from_map(get: impl Fn(&str) -> Option<String>) -> Self {
        let scan_interval_secs = get("PINGWARD_SCAN_INTERVAL")
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        let trusted_proxies = get("PINGWARD_TRUSTED_PROXIES")
            .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        Config {
            database_url: get("DATABASE_URL").unwrap_or_else(|| "sqlite://pingward.db?mode=rwc".into()),
            bind: get("PINGWARD_BIND").unwrap_or_else(|| "127.0.0.1:8080".into()),
            base_url: get("PINGWARD_BASE_URL").unwrap_or_else(|| "http://localhost:8080".into()),
            scan_interval_secs,
            forward_auth_header: get("PINGWARD_FORWARD_AUTH_HEADER"),
            trusted_proxies,
        }
    }
}
```

- [ ] **Step 4: Implement `src/error.rs`**

```rust
use axum::{http::StatusCode, response::{IntoResponse, Response}};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Other(#[from] anyhow_like::Error),
}

// Minimal boxed-error alias to avoid an extra dependency.
pub mod anyhow_like {
    pub type Error = Box<dyn std::error::Error + Send + Sync>;
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "not found").into_response(),
            AppError::Db(e) => {
                tracing::error!("db error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
            }
            AppError::Other(e) => {
                tracing::error!("error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
            }
        }
    }
}
```

- [ ] **Step 5: Wire modules** — add to `src/lib.rs`:

```rust
pub mod config;
pub mod error;
```

- [ ] **Step 6: Run — expect PASS**

Run: `cargo nextest run --lib config`
Expected: PASS (2 tests).

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/config.rs src/error.rs src/lib.rs
git commit -m "feat: config from env + AppError"
```

---

### Task 3: Database pool + migrations

**Files:**
- Create: `src/db.rs`, `migrations/sqlite/0001_init.sql`
- Modify: `src/lib.rs` (`pub mod db;`)

**Interfaces:**
- Produces: `db::Pool` (alias for `sqlx::SqlitePool`); `db::connect(url: &str) -> Result<Pool, sqlx::Error>`; `db::migrate(pool: &Pool) -> Result<(), sqlx::Error>`.

- [ ] **Step 1: Write `migrations/sqlite/0001_init.sql`**

```sql
CREATE TABLE users (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT,
  is_admin INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);

CREATE TABLE projects (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  scan_interval_secs INTEGER,
  created_at TEXT NOT NULL
);

CREATE TABLE checks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  ping_uuid TEXT NOT NULL UNIQUE,
  schedule_kind TEXT NOT NULL,           -- 'period' | 'cron'
  period_secs INTEGER,
  grace_secs INTEGER NOT NULL DEFAULT 300,
  cron_expr TEXT,
  timezone TEXT NOT NULL DEFAULT 'UTC',
  status TEXT NOT NULL DEFAULT 'new',     -- 'new'|'up'|'down'|'paused'
  last_ping_at TEXT,
  last_start_at TEXT,
  next_due_at TEXT,
  scan_interval_secs INTEGER,
  created_at TEXT NOT NULL
);
CREATE INDEX idx_checks_status ON checks(status);

CREATE TABLE channels (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,                     -- 'webhook'|'telegram'|'slack'|'ntfy'
  name TEXT NOT NULL,
  config_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE check_channels (
  check_id INTEGER NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  PRIMARY KEY (check_id, channel_id)
);

CREATE TABLE pings (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  check_id INTEGER NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,                     -- 'success'|'fail'|'start'|'log'|'exitcode'
  exit_code INTEGER,
  body TEXT NOT NULL DEFAULT '',
  source_ip TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX idx_pings_check ON pings(check_id, created_at);

CREATE TABLE notifications (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  check_id INTEGER NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  event TEXT NOT NULL,                    -- 'down'|'up'
  status TEXT NOT NULL,                   -- 'ok'|'error'
  error TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at TEXT NOT NULL
);

CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
```

- [ ] **Step 2: Write the failing test** — append to `src/db.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrate_creates_checks_table() {
        let pool = connect("sqlite::memory:").await.unwrap();
        migrate(&pool).await.unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='checks'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
    }
}
```

- [ ] **Step 3: Run — expect FAIL**

Run: `cargo nextest run --lib db`
Expected: FAIL (`connect` undefined).

- [ ] **Step 4: Implement `src/db.rs`**

```rust
use sqlx::migrate::Migrator;
use std::path::Path;

pub type Pool = sqlx::SqlitePool;

pub async fn connect(url: &str) -> Result<Pool, sqlx::Error> {
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await
}

pub async fn migrate(pool: &Pool) -> Result<(), sqlx::Error> {
    // Foreign keys are off by default in SQLite; enable per-connection is ideal,
    // but for the pool we enable here for the migration path.
    sqlx::query("PRAGMA foreign_keys = ON").execute(pool).await?;
    let m = Migrator::new(Path::new("migrations/sqlite")).await?;
    m.run(pool).await?;
    Ok(())
}
```

- [ ] **Step 5: Wire module** — `pub mod db;` in `src/lib.rs`.

- [ ] **Step 6: Run — expect PASS**

Run: `cargo nextest run --lib db`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add migrations/sqlite/0001_init.sql src/db.rs src/lib.rs
git commit -m "feat: sqlite schema + migration runner"
```

---

### Task 4: Domain models + enums

**Files:**
- Create: `src/models.rs`
- Modify: `src/lib.rs` (`pub mod models;`)

**Interfaces:**
- Produces enums `ScheduleKind { Period, Cron }`, `CheckStatus { New, Up, Down, Paused }`, `PingKind { Success, Fail, Start, Log, Exitcode }`, each with `as_str(&self) -> &'static str` and `FromStr`.
- Produces `Check` struct with fields mirroring the `checks` table (timestamps as `Option<chrono::DateTime<Utc>>`, ids as `i64`).

- [ ] **Step 1: Write the failing test** — append to `src/models.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn status_roundtrips_through_text() {
        for s in [CheckStatus::New, CheckStatus::Up, CheckStatus::Down, CheckStatus::Paused] {
            assert_eq!(CheckStatus::from_str(s.as_str()).unwrap(), s);
        }
    }

    #[test]
    fn unknown_status_is_error() {
        assert!(CheckStatus::from_str("bogus").is_err());
    }
}
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --lib models`
Expected: FAIL.

- [ ] **Step 3: Implement `src/models.rs`**

```rust
use chrono::{DateTime, Utc};
use std::str::FromStr;

macro_rules! str_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }
        }
        impl FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, String> {
                match s { $($text => Ok(Self::$variant),)+ other => Err(format!("invalid {}: {other}", stringify!($name))) }
            }
        }
    };
}

str_enum!(ScheduleKind { Period => "period", Cron => "cron" });
str_enum!(CheckStatus { New => "new", Up => "up", Down => "down", Paused => "paused" });
str_enum!(PingKind { Success => "success", Fail => "fail", Start => "start", Log => "log", Exitcode => "exitcode" });

#[derive(Debug, Clone)]
pub struct Check {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub ping_uuid: String,
    pub schedule_kind: ScheduleKind,
    pub period_secs: Option<i64>,
    pub grace_secs: i64,
    pub cron_expr: Option<String>,
    pub timezone: String,
    pub status: CheckStatus,
    pub last_ping_at: Option<DateTime<Utc>>,
    pub last_start_at: Option<DateTime<Utc>>,
    pub next_due_at: Option<DateTime<Utc>>,
    pub scan_interval_secs: Option<i64>,
    pub created_at: DateTime<Utc>,
}
```

- [ ] **Step 4: Wire module** — `pub mod models;` in `src/lib.rs`.

- [ ] **Step 5: Run — expect PASS**

Run: `cargo nextest run --lib models`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/models.rs src/lib.rs
git commit -m "feat: domain models and text enums"
```

---

### Task 5: Store — check & ping queries

**Files:**
- Create: `src/store.rs`
- Modify: `src/lib.rs` (`pub mod store;`)

**Interfaces:**
- Consumes: `db::Pool`, `models::{Check, CheckStatus, PingKind}`.
- Produces `Store { pool: Pool }` with:
  - `new(pool: Pool) -> Store`
  - `find_check_by_uuid(&self, uuid: &str) -> Result<Option<Check>, sqlx::Error>`
  - `list_active_checks(&self) -> Result<Vec<Check>, sqlx::Error>` (status in `new`,`up`)
  - `insert_ping(&self, check_id: i64, kind: PingKind, exit_code: Option<i64>, body: &str, source_ip: Option<&str>, now: DateTime<Utc>) -> Result<(), sqlx::Error>`
  - `mark_ping(&self, check_id: i64, status: CheckStatus, last_ping_at: Option<DateTime<Utc>>, last_start_at: Option<DateTime<Utc>>, next_due_at: Option<DateTime<Utc>>) -> Result<(), sqlx::Error>`
  - `set_status(&self, check_id: i64, status: CheckStatus) -> Result<(), sqlx::Error>`
  - Test helper `create_check(&self, ...) -> Result<i64, sqlx::Error>` (used by tests and, later, the web layer).

- [ ] **Step 1: Write the failing test** — append to `src/store.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db, models::{ScheduleKind, CheckStatus, PingKind}};
    use chrono::Utc;

    async fn seeded() -> Store {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        sqlx::query("INSERT INTO users (username, is_admin, created_at) VALUES ('u', 0, ?)")
            .bind(Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (user_id, name, created_at) VALUES (1, 'p', ?)")
            .bind(Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
        Store::new(pool)
    }

    #[tokio::test]
    async fn find_by_uuid_roundtrip() {
        let store = seeded().await;
        let id = store.create_check(1, "job", "uuid-1", ScheduleKind::Period, Some(60), 30, None, "UTC").await.unwrap();
        let found = store.find_check_by_uuid("uuid-1").await.unwrap().unwrap();
        assert_eq!(found.id, id);
        assert_eq!(found.status, CheckStatus::New);
        assert!(store.find_check_by_uuid("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn insert_ping_and_list_active() {
        let store = seeded().await;
        let id = store.create_check(1, "job", "u", ScheduleKind::Period, Some(60), 30, None, "UTC").await.unwrap();
        store.insert_ping(id, PingKind::Success, None, "hello", Some("1.2.3.4"), Utc::now()).await.unwrap();
        assert_eq!(store.list_active_checks().await.unwrap().len(), 1);
        store.set_status(id, CheckStatus::Paused).await.unwrap();
        assert_eq!(store.list_active_checks().await.unwrap().len(), 0);
    }
}
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --lib store`
Expected: FAIL.

- [ ] **Step 3: Implement `src/store.rs`**

```rust
use crate::db::Pool;
use crate::models::{Check, CheckStatus, PingKind, ScheduleKind};
use chrono::{DateTime, Utc};
use sqlx::Row;
use std::str::FromStr;

#[derive(Clone)]
pub struct Store {
    pub pool: Pool,
}

fn parse_ts(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|v| DateTime::parse_from_rfc3339(&v).ok().map(|d| d.with_timezone(&Utc)))
}

fn row_to_check(row: &sqlx::sqlite::SqliteRow) -> Check {
    Check {
        id: row.get("id"),
        project_id: row.get("project_id"),
        name: row.get("name"),
        ping_uuid: row.get("ping_uuid"),
        schedule_kind: ScheduleKind::from_str(row.get::<String, _>("schedule_kind").as_str()).unwrap(),
        period_secs: row.get("period_secs"),
        grace_secs: row.get("grace_secs"),
        cron_expr: row.get("cron_expr"),
        timezone: row.get("timezone"),
        status: CheckStatus::from_str(row.get::<String, _>("status").as_str()).unwrap(),
        last_ping_at: parse_ts(row.get("last_ping_at")),
        last_start_at: parse_ts(row.get("last_start_at")),
        next_due_at: parse_ts(row.get("next_due_at")),
        scan_interval_secs: row.get("scan_interval_secs"),
        created_at: parse_ts(row.get("created_at")).unwrap_or_else(Utc::now),
    }
}

impl Store {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    pub async fn find_check_by_uuid(&self, uuid: &str) -> Result<Option<Check>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM checks WHERE ping_uuid = ?")
            .bind(uuid)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(row_to_check))
    }

    pub async fn list_active_checks(&self) -> Result<Vec<Check>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM checks WHERE status IN ('new','up')")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_check).collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_ping(
        &self, check_id: i64, kind: PingKind, exit_code: Option<i64>,
        body: &str, source_ip: Option<&str>, now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at) VALUES (?,?,?,?,?,?)",
        )
        .bind(check_id).bind(kind.as_str()).bind(exit_code)
        .bind(body).bind(source_ip).bind(now.to_rfc3339())
        .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn mark_ping(
        &self, check_id: i64, status: CheckStatus,
        last_ping_at: Option<DateTime<Utc>>, last_start_at: Option<DateTime<Utc>>,
        next_due_at: Option<DateTime<Utc>>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE checks SET status=?, last_ping_at=COALESCE(?, last_ping_at), \
             last_start_at=COALESCE(?, last_start_at), next_due_at=? WHERE id=?",
        )
        .bind(status.as_str())
        .bind(last_ping_at.map(|d| d.to_rfc3339()))
        .bind(last_start_at.map(|d| d.to_rfc3339()))
        .bind(next_due_at.map(|d| d.to_rfc3339()))
        .bind(check_id)
        .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn set_status(&self, check_id: i64, status: CheckStatus) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE checks SET status=? WHERE id=?")
            .bind(status.as_str()).bind(check_id)
            .execute(&self.pool).await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_check(
        &self, project_id: i64, name: &str, ping_uuid: &str, kind: ScheduleKind,
        period_secs: Option<i64>, grace_secs: i64, cron_expr: Option<&str>, timezone: &str,
    ) -> Result<i64, sqlx::Error> {
        let res = sqlx::query(
            "INSERT INTO checks (project_id, name, ping_uuid, schedule_kind, period_secs, \
             grace_secs, cron_expr, timezone, status, created_at) VALUES (?,?,?,?,?,?,?,?, 'new', ?)",
        )
        .bind(project_id).bind(name).bind(ping_uuid).bind(kind.as_str())
        .bind(period_secs).bind(grace_secs).bind(cron_expr).bind(timezone)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool).await?;
        Ok(res.last_insert_rowid())
    }
}
```

- [ ] **Step 4: Wire module** — `pub mod store;` in `src/lib.rs`.

- [ ] **Step 5: Run — expect PASS**

Run: `cargo nextest run --lib store`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/store.rs src/lib.rs
git commit -m "feat: store layer for checks and pings"
```

---

### Task 6: Due-time computation

**Files:**
- Create: `src/scheduler.rs`
- Modify: `src/lib.rs` (`pub mod scheduler;`)

**Interfaces:**
- Consumes: `models::{Check, ScheduleKind}`, `chrono`, `chrono-tz`, `cron`.
- Produces: `scheduler::due_time(check: &Check) -> Option<DateTime<Utc>>` — the instant at/after which the check is overdue, computed from `last_ping_at` (or `created_at` for first run) + schedule + grace. `None` if uncomputable (e.g. period check with no `period_secs`).

- [ ] **Step 1: Write the failing test** — append to `src/scheduler.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Check, CheckStatus, ScheduleKind};
    use chrono::{TimeZone, Utc};

    fn base_check() -> Check {
        Check {
            id: 1, project_id: 1, name: "j".into(), ping_uuid: "u".into(),
            schedule_kind: ScheduleKind::Period, period_secs: Some(3600), grace_secs: 300,
            cron_expr: None, timezone: "UTC".into(), status: CheckStatus::Up,
            last_ping_at: Some(Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap()),
            last_start_at: None, next_due_at: None, scan_interval_secs: None,
            created_at: Utc.with_ymd_and_hms(2026, 7, 12, 11, 0, 0).unwrap(),
        }
    }

    #[test]
    fn period_due_is_last_ping_plus_period_plus_grace() {
        let c = base_check();
        // 12:00 + 3600s + 300s = 13:05
        assert_eq!(due_time(&c).unwrap(), Utc.with_ymd_and_hms(2026, 7, 12, 13, 5, 0).unwrap());
    }

    #[test]
    fn cron_due_is_next_trigger_plus_grace() {
        let mut c = base_check();
        c.schedule_kind = ScheduleKind::Cron;
        c.period_secs = None;
        c.cron_expr = Some("0 0 * * * *".into()); // top of every hour (sec min hour ...)
        // last_ping 12:00 → next trigger 13:00 + 300s grace = 13:05
        assert_eq!(due_time(&c).unwrap(), Utc.with_ymd_and_hms(2026, 7, 12, 13, 5, 0).unwrap());
    }

    #[test]
    fn period_without_period_secs_is_none() {
        let mut c = base_check();
        c.period_secs = None;
        assert!(due_time(&c).is_none());
    }
}
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --lib scheduler::tests`
Expected: FAIL (`due_time` undefined).

- [ ] **Step 3: Implement `due_time` in `src/scheduler.rs`**

```rust
use crate::models::{Check, ScheduleKind};
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use std::str::FromStr;

/// Anchor for the next expected check-in: last successful ping, else creation.
fn anchor(check: &Check) -> DateTime<Utc> {
    check.last_ping_at.unwrap_or(check.created_at)
}

pub fn due_time(check: &Check) -> Option<DateTime<Utc>> {
    let grace = Duration::seconds(check.grace_secs);
    match check.schedule_kind {
        ScheduleKind::Period => {
            let period = Duration::seconds(check.period_secs?);
            Some(anchor(check) + period + grace)
        }
        ScheduleKind::Cron => {
            let expr = check.cron_expr.as_ref()?;
            let schedule = Schedule::from_str(expr).ok()?;
            let tz: Tz = check.timezone.parse().unwrap_or(chrono_tz::UTC);
            let anchor_local = anchor(check).with_timezone(&tz);
            let next = schedule.after(&anchor_local).next()?;
            Some(next.with_timezone(&Utc) + grace)
        }
    }
}
```

Note: the `cron` crate expects 6- or 7-field expressions (leading seconds field). Document this for the UI in Plan 2; a 5-field standard cron must be normalized by prefixing `0 `.

- [ ] **Step 4: Wire module** — `pub mod scheduler;` in `src/lib.rs`.

- [ ] **Step 5: Run — expect PASS**

Run: `cargo nextest run --lib scheduler::tests`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/scheduler.rs src/lib.rs
git commit -m "feat: due-time computation for period and cron"
```

---

### Task 7: Notifier trait + webhook + dispatch

**Files:**
- Create: `src/notify.rs`
- Modify: `src/lib.rs` (`pub mod notify;`)

**Interfaces:**
- Produces: `notify::NotificationEvent { check_name: String, event: EventKind, at: DateTime<Utc>, project_id: i64 }`, `EventKind { Down, Up }`.
- Produces: `#[async_trait-free]` trait `Notifier { async fn send(&self, ev: &NotificationEvent) -> Result<(), NotifyError> }` implemented for `WebhookNotifier { url: String, client: reqwest::Client }`.
- Produces: `dispatch(notifiers: &[Box<dyn Notifier>], ev: &NotificationEvent) -> Vec<Result<(), NotifyError>>` (best-effort, one result per notifier).

Note: use native async-fn-in-trait (Rust ≥ 1.75) — no `async_trait` crate. `Box<dyn Notifier>` requires the trait be object-safe; wrap the async method with `-> Pin<Box<dyn Future...>>` OR keep `Notifier` object-safe by using the `trait-variant`-free pattern below (return a boxed future explicitly).

- [ ] **Step 1: Write the failing test** — append to `src/notify.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use wiremock::matchers::{method, path};

    #[tokio::test]
    async fn webhook_posts_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/hook"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server).await;

        let n = WebhookNotifier::new(format!("{}/hook", server.uri()));
        let ev = NotificationEvent {
            check_name: "backup".into(), event: EventKind::Down,
            at: Utc::now(), project_id: 1,
        };
        n.send(&ev).await.unwrap();
        // wiremock verifies expect(1) on drop
    }
}
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --lib notify::tests`
Expected: FAIL.

- [ ] **Step 3: Implement `src/notify.rs`**

```rust
use chrono::{DateTime, Utc};
use std::future::Future;
use std::pin::Pin;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind { Down, Up }

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self { EventKind::Down => "down", EventKind::Up => "up" }
    }
}

#[derive(Debug, Clone)]
pub struct NotificationEvent {
    pub check_name: String,
    pub event: EventKind,
    pub at: DateTime<Utc>,
    pub project_id: i64,
}

#[derive(Debug, thiserror::Error)]
#[error("notify failed: {0}")]
pub struct NotifyError(pub String);

pub trait Notifier: Send + Sync {
    fn send<'a>(&'a self, ev: &'a NotificationEvent)
        -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>>;
}

pub struct WebhookNotifier {
    url: String,
    client: reqwest::Client,
}

impl WebhookNotifier {
    pub fn new(url: String) -> Self {
        Self { url, client: reqwest::Client::new() }
    }
}

impl Notifier for WebhookNotifier {
    fn send<'a>(&'a self, ev: &'a NotificationEvent)
        -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "check": ev.check_name,
                "event": ev.event.as_str(),
                "at": ev.at.to_rfc3339(),
                "project_id": ev.project_id,
            });
            let resp = self.client.post(&self.url).json(&body).send().await
                .map_err(|e| NotifyError(e.to_string()))?;
            if resp.status().is_success() { Ok(()) }
            else { Err(NotifyError(format!("status {}", resp.status()))) }
        })
    }
}

pub async fn dispatch(notifiers: &[Box<dyn Notifier>], ev: &NotificationEvent)
    -> Vec<Result<(), NotifyError>> {
    let mut out = Vec::with_capacity(notifiers.len());
    for n in notifiers {
        out.push(n.send(ev).await);
    }
    out
}
```

- [ ] **Step 4: Wire module** — `pub mod notify;` in `src/lib.rs`.

- [ ] **Step 5: Run — expect PASS**

Run: `cargo nextest run --lib notify::tests`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/notify.rs src/lib.rs
git commit -m "feat: Notifier trait + webhook notifier + dispatch"
```

---

### Task 8: Ping API handlers + state transitions

**Files:**
- Create: `src/ping.rs`
- Modify: `src/lib.rs` (`pub mod ping;`), extend `app()` to accept a `Store` and mount ping routes.
- Test: `tests/ping_api.rs` (extend)

**Interfaces:**
- Consumes: `store::Store`, `models::{PingKind, CheckStatus}`, `scheduler::due_time`.
- Produces: router mounting
  - `GET|POST /ping/{uuid}` → success
  - `GET|POST /ping/{uuid}/fail` → fail
  - `GET|POST /ping/{uuid}/start` → start
  - `GET|POST /ping/{uuid}/log` → log
  - `GET|POST /ping/{uuid}/{code}` → exitcode
- Produces: `app(store: Store) -> Router` (signature change from Task 1; update health test).
- Behavior: unknown uuid → 404; success recomputes `next_due_at` and sets status `up`; fail sets `down`; body captured (cap 10 KB).

- [ ] **Step 1: Update the health test for the new `app` signature** — in `tests/ping_api.rs`, add a helper and adjust:

```rust
use axum_test::TestServer;
use pingward::{app, db, store::Store, models::ScheduleKind};

async fn test_server() -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    sqlx::query("INSERT INTO users (username, is_admin, created_at) VALUES ('u',0,datetime('now'))")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO projects (user_id, name, created_at) VALUES (1,'p',datetime('now'))")
        .execute(&pool).await.unwrap();
    let store = Store::new(pool);
    let server = TestServer::new(app(store.clone())).unwrap();
    (server, store)
}

#[tokio::test]
async fn healthz_returns_ok() {
    let (server, _) = test_server().await;
    server.get("/healthz").await.assert_status_ok();
}
```

- [ ] **Step 2: Write the failing ping tests** — append to `tests/ping_api.rs`

```rust
#[tokio::test]
async fn success_ping_marks_up_and_records() {
    let (server, store) = test_server().await;
    store.create_check(1, "job", "abc", ScheduleKind::Period, Some(60), 30, None, "UTC").await.unwrap();

    server.post("/ping/abc").text("done").await.assert_status_ok();

    let c = store.find_check_by_uuid("abc").await.unwrap().unwrap();
    assert_eq!(c.status, pingward::models::CheckStatus::Up);
    assert!(c.last_ping_at.is_some());
    assert!(c.next_due_at.is_some());
}

#[tokio::test]
async fn fail_ping_marks_down() {
    let (server, store) = test_server().await;
    store.create_check(1, "job", "abc", ScheduleKind::Period, Some(60), 30, None, "UTC").await.unwrap();
    server.post("/ping/abc/fail").await.assert_status_ok();
    let c = store.find_check_by_uuid("abc").await.unwrap().unwrap();
    assert_eq!(c.status, pingward::models::CheckStatus::Down);
}

#[tokio::test]
async fn unknown_uuid_is_404() {
    let (server, _) = test_server().await;
    server.get("/ping/does-not-exist").await.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn exit_code_nonzero_marks_down() {
    let (server, store) = test_server().await;
    store.create_check(1, "job", "abc", ScheduleKind::Period, Some(60), 30, None, "UTC").await.unwrap();
    server.post("/ping/abc/1").await.assert_status_ok();
    let c = store.find_check_by_uuid("abc").await.unwrap().unwrap();
    assert_eq!(c.status, pingward::models::CheckStatus::Down);
}
```

- [ ] **Step 3: Run — expect FAIL**

Run: `cargo nextest run --test ping_api`
Expected: FAIL (app signature / routes missing).

- [ ] **Step 4: Implement `src/ping.rs`**

```rust
use crate::error::AppError;
use crate::models::{CheckStatus, PingKind};
use crate::scheduler::due_time;
use crate::store::Store;
use axum::{
    body::Bytes,
    extract::{ConnectInfo, Path, State},
    http::StatusCode,
    routing::{get, post},
    Router,
};
use chrono::Utc;
use std::net::SocketAddr;

const MAX_BODY: usize = 10 * 1024;

fn truncate(bytes: &Bytes) -> String {
    let end = bytes.len().min(MAX_BODY);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

pub fn routes() -> Router<Store> {
    Router::new()
        .route("/ping/{uuid}", get(success).post(success))
        .route("/ping/{uuid}/fail", get(fail).post(fail))
        .route("/ping/{uuid}/start", get(start).post(start))
        .route("/ping/{uuid}/log", get(log).post(log))
        .route("/ping/{uuid}/{code}", get(exitcode).post(exitcode))
}

async fn resolve(store: &Store, uuid: &str) -> Result<crate::models::Check, AppError> {
    store.find_check_by_uuid(uuid).await?.ok_or(AppError::NotFound)
}

async fn success(State(store): State<Store>, Path(uuid): Path<String>,
                 conn: Option<ConnectInfo<SocketAddr>>, body: Bytes) -> Result<StatusCode, AppError> {
    apply(&store, &uuid, PingKind::Success, None, &body, conn).await
}
async fn fail(State(store): State<Store>, Path(uuid): Path<String>,
              conn: Option<ConnectInfo<SocketAddr>>, body: Bytes) -> Result<StatusCode, AppError> {
    apply(&store, &uuid, PingKind::Fail, None, &body, conn).await
}
async fn start(State(store): State<Store>, Path(uuid): Path<String>,
               conn: Option<ConnectInfo<SocketAddr>>, body: Bytes) -> Result<StatusCode, AppError> {
    apply(&store, &uuid, PingKind::Start, None, &body, conn).await
}
async fn log(State(store): State<Store>, Path(uuid): Path<String>,
             conn: Option<ConnectInfo<SocketAddr>>, body: Bytes) -> Result<StatusCode, AppError> {
    apply(&store, &uuid, PingKind::Log, None, &body, conn).await
}
async fn exitcode(State(store): State<Store>, Path((uuid, code)): Path<(String, i64)>,
                  conn: Option<ConnectInfo<SocketAddr>>, body: Bytes) -> Result<StatusCode, AppError> {
    let kind = if code == 0 { PingKind::Success } else { PingKind::Fail };
    apply(&store, &uuid, kind, Some(code), &body, conn).await
}

async fn apply(store: &Store, uuid: &str, kind: PingKind, exit_code: Option<i64>,
               body: &Bytes, conn: Option<ConnectInfo<SocketAddr>>) -> Result<StatusCode, AppError> {
    let check = resolve(store, uuid).await?;
    let now = Utc::now();
    let ip = conn.map(|c| c.0.ip().to_string());
    store.insert_ping(check.id, kind, exit_code, &truncate(body), ip.as_deref(), now).await?;

    match kind {
        PingKind::Success => {
            let mut updated = check.clone();
            updated.last_ping_at = Some(now);
            let next = due_time(&updated);
            store.mark_ping(check.id, CheckStatus::Up, Some(now), None, next).await?;
        }
        PingKind::Fail => {
            store.mark_ping(check.id, CheckStatus::Down, Some(now), None, None).await?;
        }
        PingKind::Start => {
            store.mark_ping(check.id, check.status, None, Some(now), check.next_due_at).await?;
        }
        PingKind::Log => { /* recorded only */ }
        PingKind::Exitcode => unreachable!("exitcode maps to Success/Fail above"),
    }
    Ok(StatusCode::OK)
}
```

- [ ] **Step 5: Update `src/lib.rs` `app()` to take `Store` and merge routes**

```rust
use axum::{routing::get, Router};
use store::Store;

pub mod config;
pub mod db;
pub mod error;
pub mod models;
pub mod notify;
pub mod ping;
pub mod scheduler;
pub mod store;

pub fn app(store: Store) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(ping::routes())
        .with_state(store)
}
```

- [ ] **Step 6: Run — expect PASS**

Run: `cargo nextest run --test ping_api`
Expected: PASS (5 tests). Also run `cargo nextest run` (whole suite) to confirm nothing regressed.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/ping.rs src/lib.rs tests/ping_api.rs
git commit -m "feat: ping API endpoints with success/fail/start/log/exitcode transitions"
```

---

### Task 9: Scan-once overdue detection

**Files:**
- Modify: `src/scheduler.rs` (add `scan_once`), `src/lib.rs` (already exports scheduler)
- Test: `tests/scheduler.rs`

**Interfaces:**
- Consumes: `store::Store`, `due_time`, `notify::{NotificationEvent, EventKind}`.
- Produces: `scheduler::scan_once(store: &Store, now: DateTime<Utc>) -> Result<Vec<NotificationEvent>, sqlx::Error>` — transitions every active check whose `due_time <= now` to `down` and returns one `Down` event per newly-downed check. Idempotent: a check already `down` is skipped (it is not in `list_active_checks`).

- [ ] **Step 1: Write the failing integration test** — `tests/scheduler.rs`

```rust
use chrono::{Duration, Utc};
use pingward::{db, store::Store, models::{ScheduleKind, CheckStatus}, scheduler::scan_once};

async fn store_with_up_check(period: i64, grace: i64, last_ping_ago: i64) -> (Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    sqlx::query("INSERT INTO users (username,is_admin,created_at) VALUES ('u',0,datetime('now'))").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO projects (user_id,name,created_at) VALUES (1,'p',datetime('now'))").execute(&pool).await.unwrap();
    let store = Store::new(pool);
    let id = store.create_check(1, "job", "u1", ScheduleKind::Period, Some(period), grace, None, "UTC").await.unwrap();
    let last = Utc::now() - Duration::seconds(last_ping_ago);
    store.mark_ping(id, CheckStatus::Up, Some(last), None, None).await.unwrap();
    (store, id)
}

#[tokio::test]
async fn overdue_check_transitions_to_down_and_emits_event() {
    // period 60 + grace 30 = 90s; last ping 200s ago → overdue
    let (store, id) = store_with_up_check(60, 30, 200).await;
    let events = scan_once(&store, Utc::now()).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(store.find_check_by_uuid("u1").await.unwrap().unwrap().status, CheckStatus::Down);
    let _ = id;
}

#[tokio::test]
async fn healthy_check_is_not_downed() {
    // last ping 10s ago, window 90s → healthy
    let (store, _) = store_with_up_check(60, 30, 10).await;
    let events = scan_once(&store, Utc::now()).await.unwrap();
    assert!(events.is_empty());
    assert_eq!(store.find_check_by_uuid("u1").await.unwrap().unwrap().status, CheckStatus::Up);
}
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --test scheduler`
Expected: FAIL (`scan_once` undefined).

- [ ] **Step 3: Implement `scan_once` in `src/scheduler.rs`**

```rust
use crate::notify::{EventKind, NotificationEvent};
use crate::store::Store;
use crate::models::CheckStatus;

pub async fn scan_once(store: &Store, now: DateTime<Utc>) -> Result<Vec<NotificationEvent>, sqlx::Error> {
    let mut events = Vec::new();
    for check in store.list_active_checks().await? {
        let Some(due) = due_time(&check) else { continue };
        if now >= due {
            // Isolate per-check failures: log and continue rather than abort the round.
            if let Err(e) = store.set_status(check.id, CheckStatus::Down).await {
                tracing::error!("failed to down check {}: {e}", check.id);
                continue;
            }
            events.push(NotificationEvent {
                check_name: check.name.clone(),
                event: EventKind::Down,
                at: now,
                project_id: check.project_id,
            });
        }
    }
    Ok(events)
}
```

- [ ] **Step 4: Run — expect PASS**

Run: `cargo nextest run --test scheduler`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/scheduler.rs tests/scheduler.rs
git commit -m "feat: scan_once overdue detection emitting down events"
```

---

### Task 10: Wire the background loop + end-to-end notify

**Files:**
- Modify: `src/main.rs` (build store, migrate, spawn loop, serve), `src/scheduler.rs` (add `run_scan_loop`)
- Test: `tests/scheduler.rs` (add end-to-end test through a real notifier)

**Interfaces:**
- Consumes: `Store`, `scan_once`, `notify::{Notifier, WebhookNotifier, dispatch}`, `Config`.
- Produces: `scheduler::run_scan_loop(store: Store, interval: Duration, notifiers: Arc<Vec<Box<dyn Notifier>>>) -> !` (loops: `scan_once` → `dispatch` each event → record delivery). For Plan 1, notifiers are loaded once from `PINGWARD_WEBHOOK_URL` (a single global webhook) — per-check channel binding is Plan 2. Emit a `log::warn` documenting this bound.

- [ ] **Step 1: Write the failing end-to-end test** — append to `tests/scheduler.rs`

```rust
use pingward::notify::{Notifier, WebhookNotifier, dispatch};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wiremock::matchers::method;

#[tokio::test]
async fn overdue_dispatches_to_webhook() {
    let mock = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)).expect(1).mount(&mock).await;

    let (store, _) = store_with_up_check(60, 30, 200).await;
    let notifiers: Vec<Box<dyn Notifier>> = vec![Box::new(WebhookNotifier::new(mock.uri()))];

    let events = scan_once(&store, Utc::now()).await.unwrap();
    for ev in &events {
        let results = dispatch(&notifiers, ev).await;
        assert!(results.iter().all(|r| r.is_ok()));
    }
    // mock verifies expect(1) on drop
}
```

- [ ] **Step 2: Run — expect FAIL** (until imports resolve / loop helper referenced)

Run: `cargo nextest run --test scheduler overdue_dispatches_to_webhook`
Expected: FAIL (compile until `dispatch`/`WebhookNotifier` used correctly — they already exist from Task 7, so this should compile and pass once written; if it passes immediately, that is acceptable — proceed to wire the loop below).

- [ ] **Step 3: Implement `run_scan_loop` in `src/scheduler.rs`**

```rust
use crate::notify::{dispatch, Notifier};
use std::sync::Arc;
use tokio::time::{interval, Duration as TokioDuration};

pub async fn run_scan_loop(store: Store, interval_secs: u64, notifiers: Arc<Vec<Box<dyn Notifier>>>) {
    let mut tick = interval(TokioDuration::from_secs(interval_secs.max(1)));
    loop {
        tick.tick().await;
        match scan_once(&store, Utc::now()).await {
            Ok(events) => {
                for ev in &events {
                    let _ = dispatch(&notifiers, ev).await;
                    tracing::info!("notified: {} -> {}", ev.check_name, ev.event.as_str());
                }
            }
            Err(e) => tracing::error!("scan_once failed: {e}"),
        }
    }
}
```

- [ ] **Step 4: Wire `src/main.rs`**

```rust
use pingward::{app, config::Config, db, notify::{Notifier, WebhookNotifier}, scheduler, store::Store};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
    ).init();

    let cfg = Config::from_env();
    let pool = db::connect(&cfg.database_url).await?;
    db::migrate(&pool).await?;
    let store = Store::new(pool);

    // Plan 1 bound: a single global webhook from env; per-check channels arrive in Plan 2.
    let mut notifiers: Vec<Box<dyn Notifier>> = Vec::new();
    if let Ok(url) = std::env::var("PINGWARD_WEBHOOK_URL") {
        tracing::warn!("Plan 1: using single global PINGWARD_WEBHOOK_URL; per-check channels come in Plan 2");
        notifiers.push(Box::new(WebhookNotifier::new(url)));
    }
    let notifiers = Arc::new(notifiers);

    tokio::spawn(scheduler::run_scan_loop(store.clone(), cfg.scan_interval_secs, notifiers));

    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!("listening on {}", listener.local_addr()?);
    axum::serve(
        listener,
        app(store).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    ).await?;
    Ok(())
}
```

Note: `into_make_service_with_connect_info` is required so the `ConnectInfo<SocketAddr>` extractor in `ping.rs` resolves the client IP.

- [ ] **Step 5: Run — expect PASS + full suite green**

Run: `cargo nextest run`
Expected: all tests PASS. Then `cargo build` to confirm the binary compiles.

- [ ] **Step 6: Manual smoke test (evidence before claiming done)**

```bash
# terminal 1: a webhook sink
python3 -m http.server 9999
# terminal 2:
PINGWARD_WEBHOOK_URL=http://127.0.0.1:9999/ PINGWARD_SCAN_INTERVAL=2 \
  DATABASE_URL="sqlite:///tmp/pw.db?mode=rwc" cargo run &
# seed a check directly, then:
curl -fsS http://127.0.0.1:8080/ping/<uuid>   # after creating one; or verify 404 for unknown
curl -fsS http://127.0.0.1:8080/ping/unknown  # expect HTTP 404
```

Confirm the health endpoint, a 404 for unknown uuid, and loop logs appear. (Full create-check-from-CLI ergonomics arrive with the Web UI in Plan 2; for now seeding via `sqlite3` is acceptable for the smoke test.)

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add src/main.rs src/scheduler.rs tests/scheduler.rs
git commit -m "feat: background scan loop wiring end-to-end to webhook notifier"
```

---

## Self-Review

**Spec coverage (Plan 1 scope):**
- §3 architecture (ping API, scan loop, modules) → Tasks 1,3,8,9,10 ✅
- §4 data model (all tables) → Task 3 migration ✅ (auth/UI tables created now, used in Plans 2–3)
- §5 ping protocol (success/fail/start/log/exitcode, 404, body cap, GET+POST) → Task 8 ✅
- §6 due-time (period + cron), state machine, single scan loop, transition-only notify, delivery record → Tasks 6,9,10 ✅ (delivery-record persistence into `notifications` table is deferred to Plan 2 alongside channel binding — noted below)
- §9 error handling (404, truncate, per-check isolation) → Tasks 8,9 ✅
- §10 testing (unit due-time/state, integration axum-test, notifier mock) → Tasks 6,8,9,10 ✅
- §11 Notifier trait + webhook → Task 7 ✅ (Telegram/Slack/ntfy are Plan 3)

**Deferred within monitoring core (explicit, not gaps):**
- Recovery (`down`→`up`) notification on a success ping: the state flips to `up` in Task 8, but emitting the `Up` event is wired in Plan 2 when channels are per-check. Plan 1 emits `Down` events only. (Tracked for Plan 2.)
- Per-check/project `scan_interval` cascade as a wake-cadence optimization: Plan 1 uses the single global interval and evaluates all active checks each tick (correct, just not optimized). (Tracked for Plan 2.)
- Persisting each delivery into `notifications`: Plan 2, together with channel binding.

**Placeholder scan:** none — every step has concrete code/commands.

**Type consistency:** `Store` methods, `Check` fields, enum `as_str`, `NotificationEvent`/`EventKind`, and `app(store)` signature are consistent across Tasks 4–10.
