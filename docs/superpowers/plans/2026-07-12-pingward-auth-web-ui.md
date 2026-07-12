# pingward Auth + Web UI — Implementation Plan (Plan 2 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the headless monitoring core (Plan 1) into a multi-user web application: authenticated CRUD for projects, checks, and channels; per-check channel binding with persisted delivery records; recovery notifications; and cascade-resolved scan intervals.

**Architecture:** The single `axum` binary gains two authenticated surfaces on top of the public ping API: an `auth` module (argon2 passwords, cookie sessions, forward-auth header trust) and a `web` module (server-rendered `askama` templates + plain form POSTs, no JS build step). A shared `AppState { store, config }` is threaded through all routers via `FromRef`, so the existing ping handlers keep their `State<Store>`. Notification delivery moves from a single global webhook to per-check channel resolution: a new `notify::deliver_event` loads the channels bound to a check, dispatches with bounded retry, and records every attempt in `notifications`. Recovery (`down`→`up`) and immediate fail-ping `down` notifications are wired at the same time. The scan loop resolves each active check's effective scan interval through the config cascade and ticks at the smallest one.

**Tech Stack:** Rust, `axum` 0.8, `axum-extra` 0.12 (`cookie`), `askama` 0.16 (rendered to `String` → `Html`), `argon2` 0.5, `sqlx` 0.9 (SQLite), `uuid` v4, `reqwest`, `tracing`. Tests: `axum-test`, `wiremock`.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-07-12-pingward-design.md` — this plan implements §2 (templating/auth stack), §3② (web surface), §4 (all remaining tables now used), §6 (per-check channel delivery, recovery, delivery records), §7 (auth), §8 (config cascade), §11 (webhook channel wired through the UI). Telegram/Slack/ntfy channels and PostgreSQL parity remain **Plan 3**.
- **Builds on Plan 1:** `pingward` already exposes `pingward::app(store) -> Router`, the ping API, `store::Store`, `models::{Check, ScheduleKind, CheckStatus, PingKind}`, `notify::{Notifier, WebhookNotifier, NotificationEvent, EventKind, NotifyError, dispatch}`, `scheduler::{due_time, scan_once, run_scan_loop}`, `config::Config`. This plan **modifies** several of these signatures — each such change is called out in the task that makes it, and the same task updates every caller/test.
- **Dependency cooldown:** No crate version published < 7 days ago (today 2026-07-12). Before pinning any version, check publish date (`cargo info <crate>`); if the latest is younger than 7 days, pin the most recent version that is ≥ 7 days old. **`argon2`'s latest is a release candidate (`0.6.0-rc.8`) — do not use a pre-release; pin stable `0.5`.**
- **Framework versions:** `axum` 0.8, `axum-extra` 0.12 (feature `cookie`), `askama` 0.16, `argon2` 0.5, `sqlx` 0.9. `askama` 0.16 has **no** axum integration feature — render with `template.render()?` and wrap in `axum::response::Html`.
- **Portability:** all timestamps stored as RFC3339 UTC TEXT; all enums stored as lowercase TEXT; only `INTEGER`/`TEXT` columns, so the schema still ports to PostgreSQL in Plan 3. Session-expiry comparisons rely on all timestamps being written by `DateTime::<Utc>::to_rfc3339()` (fixed `+00:00` offset) so lexicographic TEXT comparison equals chronological comparison.
- **UUID:** `ping_uuid` is UUIDv4 (`uuid::Uuid::new_v4()`), hyphenated, `UNIQUE`. Session ids are also UUIDv4 strings (bearer tokens; unguessability is the only requirement).
- **Security:** forward-auth headers are honored **only** when the request's peer IP is in `config.trusted_proxies` (spec §7). Session cookies are `HttpOnly`, `SameSite=Lax`, `Path=/`. Passwords are argon2-hashed; plaintext is never stored or logged.
- **Determinism:** all time-dependent logic takes an explicit `now: DateTime<Utc>`; only `main`'s loop and the request handlers call `Utc::now()`.
- **Rust hygiene:** `cargo fmt` before every commit; tests run with `cargo nextest run`. `cargo clippy --all-targets -- -D warnings` must stay clean (CI enforces it — see `.github/workflows/ci.yml`).
- **Commits:** GPG-signed (default git config already signs). Stage files explicitly by name — never `git add -A`/`.`.

---

## File Structure

```
pingward/
  Cargo.toml                         -- + askama, axum-extra(cookie), argon2
  migrations/sqlite/
    0001_init.sql                    -- (unchanged, Plan 1)
    0002_indexes_settings.sql        -- helper indexes for channel/notification lookups
  templates/                         -- askama templates (crate-root default dir)
    base.html                        -- layout: nav + {% block body %}
    setup.html  login.html           -- unauthenticated pages
    dashboard.html                   -- projects + checks overview
    project.html                     -- one project: its checks + channels
    check.html                       -- one check: schedule, ping URL, recent pings/notifications
    check_form.html  project_form.html  channel_form.html
    settings.html  users.html
  src/
    lib.rs        -- AppState, FromRef, app() wiring all routers
    state.rs      -- AppState struct + FromRef impls           (NEW)
    auth.rs       -- password hash/verify, sessions, cookie, CurrentUser/AdminUser, forward-auth  (NEW)
    web.rs        -- askama template structs + all authenticated page handlers  (NEW)
    models.rs     -- + User, Project, Channel, Ping, Notification, ChannelKind, NotifyStatus
    store.rs      -- + user/session/project/channel/binding/notification/setting/ping queries
    config.rs     -- + effective_scan_interval() cascade
    notify.rs     -- + channel->notifier factory, send_with_retry, deliver_event; NotificationEvent gains check_id
    scheduler.rs  -- scan loop resolves cascade tick + calls deliver_event
    ping.rs       -- success/fail pings now emit recovery/down deliveries
    main.rs       -- build AppState, drop global webhook, spawn loop, serve
  tests/
    ping_api.rs   -- (updated for app(AppState))
    scheduler.rs  -- (updated: per-check delivery + NotificationEvent.check_id)
    auth_web.rs   -- NEW: setup/login/logout + authz + CRUD integration
```

Responsibilities: `store.rs` remains the only module that writes SQL. `auth.rs` owns all credential/session logic and is the only place `argon2` is used. `web.rs` owns HTML and never writes SQL directly (only through `Store`). `notify.rs` owns channel→notifier construction and delivery recording. `scheduler.rs` and `ping.rs` call into `notify::deliver_event` but never build notifiers themselves.

---

### Task 1: Dependencies + password hashing

**Files:**
- Modify: `Cargo.toml`
- Create: `src/auth.rs`
- Modify: `src/lib.rs` (add `pub mod auth;`)

**Interfaces:**
- Produces: `auth::hash_password(plain: &str) -> Result<String, argon2::password_hash::Error>`, `auth::verify_password(plain: &str, phc: &str) -> bool`.

- [ ] **Step 1: Add dependencies to `Cargo.toml`** (verify each publish date ≥ 7 days per Global Constraints before building)

Under `[dependencies]`, add:
```toml
axum-extra = { version = "0.12", default-features = false, features = ["cookie"] }
askama = "0.16"
argon2 = "0.5"
```
Leave everything else unchanged. (`argon2` default features include `password-hash` with a CSPRNG, which `SaltString::generate` needs.)

- [ ] **Step 2: Write the failing test** — create `src/auth.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrips() {
        let phc = hash_password("hunter2").unwrap();
        assert!(phc.starts_with("$argon2"));
        assert!(verify_password("hunter2", &phc));
        assert!(!verify_password("wrong", &phc));
    }

    #[test]
    fn verify_rejects_garbage_hash() {
        assert!(!verify_password("hunter2", "not-a-phc-string"));
    }
}
```

- [ ] **Step 3: Run — expect FAIL** (unresolved `hash_password`)

Run: `cargo nextest run --lib auth`
Expected: FAIL (compile error).

- [ ] **Step 4: Implement the functions** — prepend to `src/auth.rs`:

```rust
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

/// Hash a plaintext password into a PHC string (`$argon2id$...`).
pub fn hash_password(plain: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let phc = Argon2::default().hash_password(plain.as_bytes(), &salt)?;
    Ok(phc.to_string())
}

/// Verify a plaintext password against a stored PHC string. A malformed
/// stored hash is treated as a non-match (never panics).
pub fn verify_password(plain: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default()
            .verify_password(plain.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}
```

- [ ] **Step 5: Wire module** — add `pub mod auth;` to `src/lib.rs` (alphabetical, before `config`).

- [ ] **Step 6: Run — expect PASS**

Run: `cargo nextest run --lib auth`
Expected: PASS (2 tests).

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add Cargo.toml Cargo.lock src/auth.rs src/lib.rs
git commit -m "feat: add auth deps and argon2 password hashing"
```

---

### Task 2: Domain models for users, projects, channels, pings, notifications

**Files:**
- Modify: `src/models.rs`

**Interfaces:**
- Produces enums `ChannelKind { Webhook, Telegram, Slack, Ntfy }` and `NotifyStatus { Ok, Error }` (each `as_str` + `FromStr`, via the existing `str_enum!` macro).
- Produces structs `User { id, username, password_hash: Option<String>, is_admin: bool, created_at }`, `Project { id, user_id, name, scan_interval_secs: Option<i64>, created_at }`, `Channel { id, project_id, kind: ChannelKind, name, config_json: String, created_at }`, `Ping { id, check_id, kind: PingKind, exit_code: Option<i64>, body, source_ip: Option<String>, created_at }`, `Notification { id, check_id, channel_id, event: crate::notify::EventKind, status: NotifyStatus, error: Option<String>, created_at }`.

- [ ] **Step 1: Write the failing test** — append to `src/models.rs` `mod tests`:

```rust
    #[test]
    fn channel_kind_roundtrips() {
        for k in [
            ChannelKind::Webhook,
            ChannelKind::Telegram,
            ChannelKind::Slack,
            ChannelKind::Ntfy,
        ] {
            assert_eq!(ChannelKind::from_str(k.as_str()).unwrap(), k);
        }
        assert!(ChannelKind::from_str("email").is_err());
    }

    #[test]
    fn notify_status_roundtrips() {
        assert_eq!(NotifyStatus::from_str("ok").unwrap(), NotifyStatus::Ok);
        assert_eq!(NotifyStatus::from_str("error").unwrap(), NotifyStatus::Error);
    }
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --lib models`
Expected: FAIL (`ChannelKind` undefined).

- [ ] **Step 3: Implement** — in `src/models.rs`, after the existing `str_enum!` invocations add:

```rust
str_enum!(ChannelKind { Webhook => "webhook", Telegram => "telegram", Slack => "slack", Ntfy => "ntfy" });
str_enum!(NotifyStatus { Ok => "ok", Error => "error" });
```

Then after the `Check` struct add:

```rust
#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub password_hash: Option<String>,
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub scan_interval_secs: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub id: i64,
    pub project_id: i64,
    pub kind: ChannelKind,
    pub name: String,
    pub config_json: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Ping {
    pub id: i64,
    pub check_id: i64,
    pub kind: PingKind,
    pub exit_code: Option<i64>,
    pub body: String,
    pub source_ip: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: i64,
    pub check_id: i64,
    pub channel_id: i64,
    pub event: crate::notify::EventKind,
    pub status: NotifyStatus,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}
```

Note: `EventKind` already `#[derive(Debug, Clone, Copy, PartialEq, Eq)]` in `notify.rs`; `Notification` referencing it compiles without changes there.

- [ ] **Step 4: Run — expect PASS**

Run: `cargo nextest run --lib models`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/models.rs
git commit -m "feat: domain models for users, projects, channels, pings, notifications"
```

---

### Task 3: Migration 0002 + store: users & sessions

**Files:**
- Create: `migrations/sqlite/0002_indexes.sql`
- Modify: `src/store.rs`

**Interfaces:**
- Consumes: `models::User`.
- Produces on `Store`:
  - `count_users(&self) -> Result<i64, sqlx::Error>`
  - `create_user(&self, username: &str, password_hash: Option<&str>, is_admin: bool, now: DateTime<Utc>) -> Result<i64, sqlx::Error>`
  - `find_user_by_username(&self, username: &str) -> Result<Option<User>, sqlx::Error>`
  - `find_user_by_id(&self, id: i64) -> Result<Option<User>, sqlx::Error>`
  - `list_users(&self) -> Result<Vec<User>, sqlx::Error>`
  - `delete_user(&self, id: i64) -> Result<(), sqlx::Error>`
  - `create_session(&self, id: &str, user_id: i64, expires_at: DateTime<Utc>) -> Result<(), sqlx::Error>`
  - `find_session_user(&self, session_id: &str, now: DateTime<Utc>) -> Result<Option<User>, sqlx::Error>`
  - `delete_session(&self, id: &str) -> Result<(), sqlx::Error>`

- [ ] **Step 1: Write migration** — `migrations/sqlite/0002_indexes.sql`:

```sql
CREATE INDEX idx_channels_project ON channels(project_id);
CREATE INDEX idx_check_channels_channel ON check_channels(channel_id);
CREATE INDEX idx_notifications_check ON notifications(check_id, created_at);
CREATE INDEX idx_sessions_user ON sessions(user_id);
```

- [ ] **Step 2: Write the failing test** — append to `src/store.rs` `mod tests`:

```rust
    #[tokio::test]
    async fn user_and_session_lifecycle() {
        let store = seeded().await; // seeds user id=1 already
        assert_eq!(store.count_users().await.unwrap(), 1);

        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let uid = store
            .create_user("bob", Some("phc"), true, now)
            .await
            .unwrap();
        assert_eq!(store.count_users().await.unwrap(), 2);

        let bob = store.find_user_by_username("bob").await.unwrap().unwrap();
        assert_eq!(bob.id, uid);
        assert!(bob.is_admin);
        assert_eq!(bob.password_hash.as_deref(), Some("phc"));
        assert!(store.find_user_by_username("nobody").await.unwrap().is_none());

        store
            .create_session("sess-1", uid, now + chrono::Duration::hours(1))
            .await
            .unwrap();
        // valid at now
        let u = store.find_session_user("sess-1", now).await.unwrap().unwrap();
        assert_eq!(u.id, uid);
        // expired two hours later
        assert!(store
            .find_session_user("sess-1", now + chrono::Duration::hours(2))
            .await
            .unwrap()
            .is_none());
        // deleted
        store.delete_session("sess-1").await.unwrap();
        assert!(store.find_session_user("sess-1", now).await.unwrap().is_none());
    }
```

- [ ] **Step 3: Run — expect FAIL**

Run: `cargo nextest run --lib store::tests::user_and_session_lifecycle`
Expected: FAIL (`count_users` undefined).

- [ ] **Step 4: Implement** — at the top of `src/store.rs`, extend the imports:

```rust
use crate::models::{Channel, ChannelKind, Check, CheckStatus, Notification, NotifyStatus, Ping, PingKind, Project, ScheduleKind, User};
use crate::notify::EventKind;
use std::collections::HashMap;
```
(Keep the existing `chrono`, `sqlx::Row`, `FromStr`, `Pool` imports.)

Add a row-mapper and the methods inside `impl Store` (place after `create_check`):

```rust
fn row_to_user(row: &sqlx::sqlite::SqliteRow) -> Result<User, sqlx::Error> {
    Ok(User {
        id: row.get("id"),
        username: row.get("username"),
        password_hash: row.get("password_hash"),
        is_admin: row.get::<i64, _>("is_admin") != 0,
        created_at: parse_ts(row.get("created_at"))
            .ok_or_else(|| decode_err("users.created_at must be RFC3339"))?,
    })
}
```

```rust
    pub async fn count_users(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await
    }

    pub async fn create_user(
        &self,
        username: &str,
        password_hash: Option<&str>,
        is_admin: bool,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        let res = sqlx::query(
            "INSERT INTO users (username, password_hash, is_admin, created_at) VALUES (?,?,?,?)",
        )
        .bind(username)
        .bind(password_hash)
        .bind(is_admin as i64)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(res.last_insert_rowid())
    }

    pub async fn find_user_by_username(&self, username: &str) -> Result<Option<User>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(row_to_user).transpose()
    }

    pub async fn find_user_by_id(&self, id: i64) -> Result<Option<User>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(row_to_user).transpose()
    }

    pub async fn list_users(&self) -> Result<Vec<User>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM users ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_user).collect()
    }

    pub async fn delete_user(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn create_session(
        &self,
        id: &str,
        user_id: i64,
        expires_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO sessions (id, user_id, expires_at) VALUES (?,?,?)")
            .bind(id)
            .bind(user_id)
            .bind(expires_at.to_rfc3339())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn find_session_user(
        &self,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<User>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT u.* FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.id = ? AND s.expires_at > ?",
        )
        .bind(session_id)
        .bind(now.to_rfc3339())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(row_to_user).transpose()
    }

    pub async fn delete_session(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
```

Note: `row_to_user` is a free function like `row_to_check`; place it beside `row_to_check`, not inside `impl`. The `HashMap`/`Channel`/etc. imports added above are used by Tasks 5–7; if clippy warns about unused imports before those tasks land, add them incrementally instead — but since tasks are implemented in order, add the full import line now and the later tasks consume them.

- [ ] **Step 5: Run — expect PASS**

Run: `cargo nextest run --lib store::tests::user_and_session_lifecycle`
Expected: PASS. (If unused-import warnings block via `-D warnings` in a clippy run, temporarily narrow the import to `User` only and widen it in Task 5. `cargo nextest` itself does not deny warnings.)

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add migrations/sqlite/0002_indexes.sql src/store.rs
git commit -m "feat: migration 0002 indexes + user/session store queries"
```

---

### Task 4: AppState + FromRef refactor

**Files:**
- Create: `src/state.rs`
- Modify: `src/lib.rs`, `src/ping.rs`, `src/main.rs`, `tests/ping_api.rs`, `tests/scheduler.rs` (only where they build the app/state)

**Interfaces:**
- Produces: `state::AppState { store: Store, config: std::sync::Arc<Config> }` deriving `Clone`, with `impl axum::extract::FromRef<AppState> for Store`.
- Changes: `pingward::app(state: AppState) -> Router` (was `app(store: Store)`). `ping::routes() -> Router<AppState>` (was `Router<Store>`; handlers keep `State<Store>` via `FromRef`).

- [ ] **Step 1: Write `src/state.rs`**

```rust
use crate::config::Config;
use crate::store::Store;
use axum::extract::FromRef;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub config: Arc<Config>,
}

impl AppState {
    pub fn new(store: Store, config: Config) -> Self {
        Self {
            store,
            config: Arc::new(config),
        }
    }
}

impl FromRef<AppState> for Store {
    fn from_ref(state: &AppState) -> Store {
        state.store.clone()
    }
}
```

- [ ] **Step 2: Update `src/lib.rs`** — add modules and change `app`:

```rust
use axum::{routing::get, Router};
use state::AppState;

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod models;
pub mod notify;
pub mod ping;
pub mod scheduler;
pub mod state;
pub mod store;
pub mod web; // added in Task 10; declare now behind a stub so app() compiles

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(ping::routes())
        .with_state(state)
}
```

Because `web` does not exist until Task 10, create a placeholder now: `src/web.rs` containing only `// filled in Task 10`. (An empty module compiles.)

- [ ] **Step 3: Update `src/ping.rs`** — change the router state type only:

```rust
use crate::state::AppState;
// ...
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/ping/{uuid}", get(success).post(success))
        .route("/ping/{uuid}/fail", get(fail).post(fail))
        .route("/ping/{uuid}/start", get(start).post(start))
        .route("/ping/{uuid}/log", get(log).post(log))
        .route("/ping/{uuid}/{code}", get(exitcode).post(exitcode))
}
```
Handlers keep `State(store): State<Store>` unchanged — `Store: FromRef<AppState>` makes that resolve.

- [ ] **Step 4: Update callers** — in `tests/ping_api.rs`, change the `test_server` helper:

```rust
use pingward::{app, config::Config, db, models::ScheduleKind, state::AppState, store::Store};

async fn test_server() -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    sqlx::query("INSERT INTO users (username, is_admin, created_at) VALUES ('u',0,datetime('now'))")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO projects (user_id, name, created_at) VALUES (1,'p',datetime('now'))")
        .execute(&pool).await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let server = TestServer::new(app(state)).unwrap();
    (server, store)
}
```

In `src/main.rs`, change the serve line (full rewrite lands in Task 15; for now just make it compile):

```rust
    let state = pingward::state::AppState::new(store.clone(), config);
    // ... spawn loop unchanged for now ...
    axum::serve(
        listener,
        pingward::app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
```
`config` is moved into `AppState::new`; read `config.bind`/`config.scan_interval_secs` into locals *before* moving it:
```rust
    let bind = config.bind.clone();
    let scan_interval_secs = config.scan_interval_secs;
```
and use those locals for `TcpListener::bind(&bind)` and the `run_scan_loop` call.

- [ ] **Step 5: Run — expect PASS** (refactor is behavior-preserving)

Run: `cargo nextest run --test ping_api && cargo build`
Expected: PASS; binary compiles.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/state.rs src/lib.rs src/ping.rs src/main.rs src/web.rs tests/ping_api.rs
git commit -m "refactor: introduce AppState and thread it via FromRef"
```

---

### Task 5: Store: projects, channels, bindings, settings, pings, notifications

**Files:**
- Modify: `src/store.rs`

**Interfaces:**
- Produces on `Store` (all `Result<_, sqlx::Error>`):
  - Projects: `create_project(user_id, name, scan_interval_secs: Option<i64>, now)`→`i64`; `find_project(id)`→`Option<Project>`; `list_projects_for_user(user_id)`→`Vec<Project>`; `update_project(id, name, scan_interval_secs: Option<i64>)`→`()`; `delete_project(id)`→`()`; `all_project_scan_intervals()`→`HashMap<i64, Option<i64>>`.
  - Channels: `create_channel(project_id, kind: ChannelKind, name, config_json, now)`→`i64`; `find_channel(id)`→`Option<Channel>`; `list_channels_for_project(project_id)`→`Vec<Channel>`; `delete_channel(id)`→`()`.
  - Bindings: `bind_channel(check_id, channel_id)`→`()`; `unbind_channel(check_id, channel_id)`→`()`; `bound_channel_ids(check_id)`→`Vec<i64>`; `channels_for_check(check_id)`→`Vec<Channel>`.
  - Checks (web-side): `find_check(id)`→`Option<Check>`; `list_checks_for_project(project_id)`→`Vec<Check>`; `update_check_schedule(id, name, kind, period_secs, grace_secs, cron_expr, timezone, scan_interval_secs)`→`()`; `regenerate_uuid(id, new_uuid)`→`()`; `delete_check(id)`→`()`.
  - Pings/Notifications: `list_recent_pings(check_id, limit: i64)`→`Vec<Ping>`; `record_notification(check_id, channel_id, event: EventKind, status: NotifyStatus, error: Option<&str>, now)`→`()`; `list_recent_notifications(check_id, limit: i64)`→`Vec<Notification>`.
  - Settings: `get_setting(key)`→`Option<String>`; `set_setting(key, value)`→`()`.

- [ ] **Step 1: Write the failing test** — append to `src/store.rs` `mod tests`:

```rust
    #[tokio::test]
    async fn project_channel_binding_and_settings() {
        let store = seeded().await;
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();

        let pid = store.create_project(1, "web", Some(15), now).await.unwrap();
        assert_eq!(store.list_projects_for_user(1).await.unwrap().len(), 2); // 'p' from seed + 'web'
        assert_eq!(store.find_project(pid).await.unwrap().unwrap().scan_interval_secs, Some(15));

        let cid = store
            .create_channel(pid, ChannelKind::Webhook, "hook", r#"{"url":"http://x"}"#, now)
            .await
            .unwrap();
        assert_eq!(store.list_channels_for_project(pid).await.unwrap().len(), 1);

        let chk = store
            .create_check(pid, "job", "uuid-x", ScheduleKind::Period, Some(60), 30, None, "UTC")
            .await
            .unwrap();
        store.bind_channel(chk, cid).await.unwrap();
        assert_eq!(store.bound_channel_ids(chk).await.unwrap(), vec![cid]);
        assert_eq!(store.channels_for_check(chk).await.unwrap().len(), 1);
        store.unbind_channel(chk, cid).await.unwrap();
        assert!(store.bound_channel_ids(chk).await.unwrap().is_empty());

        store.record_notification(chk, cid, EventKind::Down, NotifyStatus::Ok, None, now).await.unwrap();
        assert_eq!(store.list_recent_notifications(chk, 10).await.unwrap().len(), 1);

        assert!(store.get_setting("scan_interval").await.unwrap().is_none());
        store.set_setting("scan_interval", "45").await.unwrap();
        assert_eq!(store.get_setting("scan_interval").await.unwrap().as_deref(), Some("45"));
        store.set_setting("scan_interval", "60").await.unwrap(); // upsert
        assert_eq!(store.get_setting("scan_interval").await.unwrap().as_deref(), Some("60"));

        let map = store.all_project_scan_intervals().await.unwrap();
        assert_eq!(map.get(&pid), Some(&Some(15)));
    }
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --lib store::tests::project_channel_binding_and_settings`
Expected: FAIL.

- [ ] **Step 3: Implement** — add row-mappers beside `row_to_check`:

```rust
fn row_to_project(row: &sqlx::sqlite::SqliteRow) -> Result<Project, sqlx::Error> {
    Ok(Project {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        scan_interval_secs: row.get("scan_interval_secs"),
        created_at: parse_ts(row.get("created_at"))
            .ok_or_else(|| decode_err("projects.created_at must be RFC3339"))?,
    })
}

fn row_to_channel(row: &sqlx::sqlite::SqliteRow) -> Result<Channel, sqlx::Error> {
    let kind_raw: String = row.get("kind");
    let kind = ChannelKind::from_str(&kind_raw)
        .map_err(|e| decode_err(format!("invalid channel kind {kind_raw:?}: {e}")))?;
    Ok(Channel {
        id: row.get("id"),
        project_id: row.get("project_id"),
        kind,
        name: row.get("name"),
        config_json: row.get("config_json"),
        created_at: parse_ts(row.get("created_at"))
            .ok_or_else(|| decode_err("channels.created_at must be RFC3339"))?,
    })
}

fn row_to_ping(row: &sqlx::sqlite::SqliteRow) -> Result<Ping, sqlx::Error> {
    let kind_raw: String = row.get("kind");
    let kind = PingKind::from_str(&kind_raw)
        .map_err(|e| decode_err(format!("invalid ping kind {kind_raw:?}: {e}")))?;
    Ok(Ping {
        id: row.get("id"),
        check_id: row.get("check_id"),
        kind,
        exit_code: row.get("exit_code"),
        body: row.get("body"),
        source_ip: row.get("source_ip"),
        created_at: parse_ts(row.get("created_at"))
            .ok_or_else(|| decode_err("pings.created_at must be RFC3339"))?,
    })
}

fn row_to_notification(row: &sqlx::sqlite::SqliteRow) -> Result<Notification, sqlx::Error> {
    let event_raw: String = row.get("event");
    let event = EventKind::from_str(&event_raw)
        .map_err(|e| decode_err(format!("invalid notification event {event_raw:?}: {e}")))?;
    let status_raw: String = row.get("status");
    let status = NotifyStatus::from_str(&status_raw)
        .map_err(|e| decode_err(format!("invalid notification status {status_raw:?}: {e}")))?;
    Ok(Notification {
        id: row.get("id"),
        check_id: row.get("check_id"),
        channel_id: row.get("channel_id"),
        event,
        status,
        error: row.get("error"),
        created_at: parse_ts(row.get("created_at"))
            .ok_or_else(|| decode_err("notifications.created_at must be RFC3339"))?,
    })
}
```

Note: `EventKind::from_str` does not exist yet — add it. In `src/notify.rs`, extend `EventKind` with a parser (place under the existing `as_str` impl):

```rust
impl std::str::FromStr for EventKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "down" => Ok(EventKind::Down),
            "up" => Ok(EventKind::Up),
            other => Err(format!("invalid EventKind: {other}")),
        }
    }
}
```

Now add the `impl Store` methods:

```rust
    // --- projects ---
    pub async fn create_project(
        &self,
        user_id: i64,
        name: &str,
        scan_interval_secs: Option<i64>,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        let res = sqlx::query(
            "INSERT INTO projects (user_id, name, scan_interval_secs, created_at) VALUES (?,?,?,?)",
        )
        .bind(user_id).bind(name).bind(scan_interval_secs).bind(now.to_rfc3339())
        .execute(&self.pool).await?;
        Ok(res.last_insert_rowid())
    }

    pub async fn find_project(&self, id: i64) -> Result<Option<Project>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM projects WHERE id = ?")
            .bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(row_to_project).transpose()
    }

    pub async fn list_projects_for_user(&self, user_id: i64) -> Result<Vec<Project>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM projects WHERE user_id = ? ORDER BY id")
            .bind(user_id).fetch_all(&self.pool).await?;
        rows.iter().map(row_to_project).collect()
    }

    pub async fn update_project(
        &self,
        id: i64,
        name: &str,
        scan_interval_secs: Option<i64>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE projects SET name = ?, scan_interval_secs = ? WHERE id = ?")
            .bind(name).bind(scan_interval_secs).bind(id)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn delete_project(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM projects WHERE id = ?")
            .bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn all_project_scan_intervals(&self) -> Result<HashMap<i64, Option<i64>>, sqlx::Error> {
        let rows = sqlx::query("SELECT id, scan_interval_secs FROM projects")
            .fetch_all(&self.pool).await?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<i64, _>("id"), r.get::<Option<i64>, _>("scan_interval_secs")))
            .collect())
    }

    // --- channels ---
    pub async fn create_channel(
        &self,
        project_id: i64,
        kind: ChannelKind,
        name: &str,
        config_json: &str,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        let res = sqlx::query(
            "INSERT INTO channels (project_id, kind, name, config_json, created_at) VALUES (?,?,?,?,?)",
        )
        .bind(project_id).bind(kind.as_str()).bind(name).bind(config_json).bind(now.to_rfc3339())
        .execute(&self.pool).await?;
        Ok(res.last_insert_rowid())
    }

    pub async fn find_channel(&self, id: i64) -> Result<Option<Channel>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM channels WHERE id = ?")
            .bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(row_to_channel).transpose()
    }

    pub async fn list_channels_for_project(&self, project_id: i64) -> Result<Vec<Channel>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM channels WHERE project_id = ? ORDER BY id")
            .bind(project_id).fetch_all(&self.pool).await?;
        rows.iter().map(row_to_channel).collect()
    }

    pub async fn delete_channel(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM channels WHERE id = ?")
            .bind(id).execute(&self.pool).await?;
        Ok(())
    }

    // --- bindings ---
    pub async fn bind_channel(&self, check_id: i64, channel_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT OR IGNORE INTO check_channels (check_id, channel_id) VALUES (?,?)")
            .bind(check_id).bind(channel_id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn unbind_channel(&self, check_id: i64, channel_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM check_channels WHERE check_id = ? AND channel_id = ?")
            .bind(check_id).bind(channel_id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn bound_channel_ids(&self, check_id: i64) -> Result<Vec<i64>, sqlx::Error> {
        let rows = sqlx::query("SELECT channel_id FROM check_channels WHERE check_id = ? ORDER BY channel_id")
            .bind(check_id).fetch_all(&self.pool).await?;
        Ok(rows.iter().map(|r| r.get::<i64, _>("channel_id")).collect())
    }

    pub async fn channels_for_check(&self, check_id: i64) -> Result<Vec<Channel>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT c.* FROM channels c JOIN check_channels cc ON cc.channel_id = c.id \
             WHERE cc.check_id = ? ORDER BY c.id",
        )
        .bind(check_id).fetch_all(&self.pool).await?;
        rows.iter().map(row_to_channel).collect()
    }

    // --- checks (web) ---
    pub async fn find_check(&self, id: i64) -> Result<Option<Check>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM checks WHERE id = ?")
            .bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(row_to_check).transpose()
    }

    pub async fn list_checks_for_project(&self, project_id: i64) -> Result<Vec<Check>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM checks WHERE project_id = ? ORDER BY id")
            .bind(project_id).fetch_all(&self.pool).await?;
        rows.iter().map(row_to_check).collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_check_schedule(
        &self,
        id: i64,
        name: &str,
        kind: ScheduleKind,
        period_secs: Option<i64>,
        grace_secs: i64,
        cron_expr: Option<&str>,
        timezone: &str,
        scan_interval_secs: Option<i64>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE checks SET name=?, schedule_kind=?, period_secs=?, grace_secs=?, \
             cron_expr=?, timezone=?, scan_interval_secs=? WHERE id=?",
        )
        .bind(name).bind(kind.as_str()).bind(period_secs).bind(grace_secs)
        .bind(cron_expr).bind(timezone).bind(scan_interval_secs).bind(id)
        .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn regenerate_uuid(&self, id: i64, new_uuid: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE checks SET ping_uuid = ? WHERE id = ?")
            .bind(new_uuid).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn delete_check(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM checks WHERE id = ?")
            .bind(id).execute(&self.pool).await?;
        Ok(())
    }

    // --- pings / notifications ---
    pub async fn list_recent_pings(&self, check_id: i64, limit: i64) -> Result<Vec<Ping>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM pings WHERE check_id = ? ORDER BY id DESC LIMIT ?")
            .bind(check_id).bind(limit).fetch_all(&self.pool).await?;
        rows.iter().map(row_to_ping).collect()
    }

    pub async fn record_notification(
        &self,
        check_id: i64,
        channel_id: i64,
        event: EventKind,
        status: NotifyStatus,
        error: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO notifications (check_id, channel_id, event, status, error, created_at) \
             VALUES (?,?,?,?,?,?)",
        )
        .bind(check_id).bind(channel_id).bind(event.as_str()).bind(status.as_str())
        .bind(error).bind(now.to_rfc3339())
        .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn list_recent_notifications(
        &self,
        check_id: i64,
        limit: i64,
    ) -> Result<Vec<Notification>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM notifications WHERE check_id = ? ORDER BY id DESC LIMIT ?")
            .bind(check_id).bind(limit).fetch_all(&self.pool).await?;
        rows.iter().map(row_to_notification).collect()
    }

    // --- settings ---
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(key).fetch_optional(&self.pool).await
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?,?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key).bind(value).execute(&self.pool).await?;
        Ok(())
    }
```

- [ ] **Step 4: Run — expect PASS**

Run: `cargo nextest run --lib store::tests::project_channel_binding_and_settings`
Expected: PASS. Also run `cargo nextest run --lib store` to confirm the Plan 1 store tests still pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/store.rs src/notify.rs
git commit -m "feat: store queries for projects, channels, bindings, notifications, settings"
```

---

### Task 6: Config cascade — effective scan interval

**Files:**
- Modify: `src/config.rs`

**Interfaces:**
- Produces: `config::effective_scan_interval(check_secs: Option<i64>, project_secs: Option<i64>, global_secs: Option<i64>, env_default: u64) -> u64` — first defined, non-negative value wins, in the order check → project → global → env (spec §8). Non-positive overrides (`<= 0`) are ignored (fall through), and the result is clamped to `>= 1`.

- [ ] **Step 1: Write the failing test** — append to `src/config.rs` `mod tests`:

```rust
    #[test]
    fn cascade_prefers_most_specific() {
        // check wins
        assert_eq!(effective_scan_interval(Some(5), Some(10), Some(20), 30), 5);
        // project when no check
        assert_eq!(effective_scan_interval(None, Some(10), Some(20), 30), 10);
        // global when no check/project
        assert_eq!(effective_scan_interval(None, None, Some(20), 30), 20);
        // env default when nothing set
        assert_eq!(effective_scan_interval(None, None, None, 30), 30);
        // non-positive overrides are ignored
        assert_eq!(effective_scan_interval(Some(0), Some(-1), None, 30), 30);
        // result is clamped to >= 1 even if env default is 0
        assert_eq!(effective_scan_interval(None, None, None, 0), 1);
    }
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --lib config::tests::cascade_prefers_most_specific`
Expected: FAIL.

- [ ] **Step 3: Implement** — add to `src/config.rs` (free function, outside `impl Config`):

```rust
/// Resolve the effective scan interval for a check using the spec §8 cascade:
/// check → project → global (DB settings) → env default. A `Some(v)` override
/// with `v <= 0` is treated as unset and falls through. The result is clamped
/// to at least 1 second so the scan loop's timer is always valid.
pub fn effective_scan_interval(
    check_secs: Option<i64>,
    project_secs: Option<i64>,
    global_secs: Option<i64>,
    env_default: u64,
) -> u64 {
    for candidate in [check_secs, project_secs, global_secs] {
        if let Some(v) = candidate {
            if v > 0 {
                return v as u64;
            }
        }
    }
    env_default.max(1)
}
```

- [ ] **Step 4: Run — expect PASS**

Run: `cargo nextest run --lib config::tests::cascade_prefers_most_specific`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/config.rs
git commit -m "feat: config cascade for effective scan interval"
```

---

### Task 7: Notify — channel factory, retry, and delivery recording

**Files:**
- Modify: `src/notify.rs`
- Modify: `src/scheduler.rs`, `tests/scheduler.rs` (they construct `NotificationEvent`; add `check_id`)

**Interfaces:**
- Changes: `NotificationEvent` gains `pub check_id: i64` (first field).
- Produces: `notify::notifier_for(channel: &crate::models::Channel) -> Option<Box<dyn Notifier>>` — builds a `WebhookNotifier` for `kind == Webhook` (parsing `{"url": "..."}` from `config_json`); returns `None` (with a debug log) for Telegram/Slack/ntfy, which arrive in Plan 3.
- Produces: `notify::RetryPolicy { max_attempts: u32, base_backoff: std::time::Duration }` with `RetryPolicy::default()` = 3 attempts, 500ms base.
- Produces: `notify::send_with_retry(n: &dyn Notifier, ev: &NotificationEvent, policy: RetryPolicy) -> Result<(), NotifyError>` — retries on error with exponential backoff (`base * 2^attempt`), up to `max_attempts`.
- Produces: `notify::deliver_event(store: &crate::store::Store, ev: &NotificationEvent, policy: RetryPolicy, now: DateTime<Utc>) -> ()` — resolves the check's bound channels, delivers to each with retry, and records every outcome in `notifications`. Never returns an error (delivery failures are recorded, not propagated — spec §6: delivery is decoupled from state).

- [ ] **Step 1: Add `check_id` to `NotificationEvent`** — in `src/notify.rs`:

```rust
#[derive(Debug, Clone)]
pub struct NotificationEvent {
    pub check_id: i64,
    pub check_name: String,
    pub event: EventKind,
    pub at: DateTime<Utc>,
    pub project_id: i64,
}
```
Update the two constructions in `src/notify.rs` `mod tests` (`webhook_posts_json`, `webhook_send_times_out_on_hung_endpoint`) to add `check_id: 1,` as the first field. Update `src/scheduler.rs::scan_once` to set `check_id: check.id,` and the scheduler test file constructions likewise (compiler will point to each).

- [ ] **Step 2: Write the failing tests** — append to `src/notify.rs` `mod tests`:

```rust
    use crate::db;
    use crate::models::ChannelKind;
    use crate::store::Store;

    async fn store_with_check_and_channel(url: &str) -> (Store, i64) {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        sqlx::query("INSERT INTO users (username,is_admin,created_at) VALUES ('u',0,datetime('now'))")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO projects (user_id,name,created_at) VALUES (1,'p',datetime('now'))")
            .execute(&pool).await.unwrap();
        let store = Store::new(pool);
        let now = Utc::now();
        let cid = store
            .create_channel(1, ChannelKind::Webhook, "hook", &format!("{{\"url\":\"{url}\"}}"), now)
            .await.unwrap();
        let chk = store
            .create_check(1, "job", "u1", crate::models::ScheduleKind::Period, Some(60), 30, None, "UTC")
            .await.unwrap();
        store.bind_channel(chk, cid).await.unwrap();
        (store, chk)
    }

    #[tokio::test]
    async fn deliver_event_posts_and_records_ok() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let (store, chk) = store_with_check_and_channel(&server.uri()).await;
        let ev = NotificationEvent {
            check_id: chk, check_name: "job".into(), event: EventKind::Down,
            at: Utc::now(), project_id: 1,
        };
        deliver_event(&store, &ev, RetryPolicy::default(), Utc::now()).await;

        let recs = store.list_recent_notifications(chk, 10).await.unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].status, crate::models::NotifyStatus::Ok);
    }

    #[tokio::test]
    async fn deliver_event_records_error_when_channel_fails() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let (store, chk) = store_with_check_and_channel(&server.uri()).await;
        let ev = NotificationEvent {
            check_id: chk, check_name: "job".into(), event: EventKind::Down,
            at: Utc::now(), project_id: 1,
        };
        // fast policy so the test does not sleep for seconds
        let policy = RetryPolicy { max_attempts: 2, base_backoff: std::time::Duration::from_millis(1) };
        deliver_event(&store, &ev, policy, Utc::now()).await;

        let recs = store.list_recent_notifications(chk, 10).await.unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].status, crate::models::NotifyStatus::Error);
        assert!(recs[0].error.is_some());
    }
```

- [ ] **Step 3: Run — expect FAIL**

Run: `cargo nextest run --lib notify::tests::deliver_event_posts_and_records_ok`
Expected: FAIL (`deliver_event` undefined).

- [ ] **Step 4: Implement** — add to `src/notify.rs`:

```rust
use crate::models::{Channel, ChannelKind, NotifyStatus};
use crate::store::Store;
use chrono::DateTime;

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_backoff: std::time::Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_attempts: 3, base_backoff: std::time::Duration::from_millis(500) }
    }
}

/// Build a notifier for a channel. Only `webhook` is implemented in Plan 2;
/// Telegram/Slack/ntfy return `None` (logged) and arrive in Plan 3.
pub fn notifier_for(channel: &Channel) -> Option<Box<dyn Notifier>> {
    match channel.kind {
        ChannelKind::Webhook => {
            let url = serde_json::from_str::<serde_json::Value>(&channel.config_json)
                .ok()
                .and_then(|v| v.get("url").and_then(|u| u.as_str()).map(str::to_owned));
            match url {
                Some(u) => Some(Box::new(WebhookNotifier::new(u))),
                None => {
                    tracing::warn!(channel_id = channel.id, "webhook channel missing url in config_json");
                    None
                }
            }
        }
        other => {
            tracing::debug!(channel_id = channel.id, kind = other.as_str(), "channel kind not yet supported (Plan 3)");
            None
        }
    }
}

/// Send with bounded exponential-backoff retry. Returns the last error if all
/// attempts fail.
pub async fn send_with_retry(
    n: &dyn Notifier,
    ev: &NotificationEvent,
    policy: RetryPolicy,
) -> Result<(), NotifyError> {
    let mut last = NotifyError("no attempts".into());
    for attempt in 0..policy.max_attempts.max(1) {
        match n.send(ev).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = e;
                if attempt + 1 < policy.max_attempts {
                    let backoff = policy.base_backoff * 2u32.saturating_pow(attempt);
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
    Err(last)
}

/// Resolve the check's bound channels, deliver to each with retry, and record
/// every outcome in `notifications`. Delivery failures are recorded, never
/// propagated (spec §6: a failing channel must not affect state).
pub async fn deliver_event(store: &Store, ev: &NotificationEvent, policy: RetryPolicy, now: DateTime<Utc>) {
    let channels = match store.channels_for_check(ev.check_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(check_id = ev.check_id, "failed to load channels: {e}");
            return;
        }
    };
    if channels.is_empty() {
        tracing::debug!(check = %ev.check_name, event = ev.event.as_str(), "no channels bound; nothing to deliver");
        return;
    }
    for channel in &channels {
        let Some(notifier) = notifier_for(channel) else { continue };
        let (status, error) = match send_with_retry(notifier.as_ref(), ev, policy).await {
            Ok(()) => (NotifyStatus::Ok, None),
            Err(e) => (NotifyStatus::Error, Some(e.to_string())),
        };
        if let Err(e) = store
            .record_notification(ev.check_id, channel.id, ev.event, status, error.as_deref(), now)
            .await
        {
            tracing::error!(check_id = ev.check_id, channel_id = channel.id, "failed to record notification: {e}");
        }
    }
}
```

- [ ] **Step 5: Run — expect PASS**

Run: `cargo nextest run --lib notify`
Expected: PASS. Also `cargo nextest run --lib` to catch any `NotificationEvent` construction the compiler flagged.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/notify.rs src/scheduler.rs tests/scheduler.rs
git commit -m "feat: per-channel delivery with retry and notification recording"
```

---

### Task 8: Wire recovery + fail-ping notifications; rewrite scan loop for cascade + per-check delivery

**Files:**
- Modify: `src/ping.rs`, `src/scheduler.rs`, `tests/scheduler.rs`

**Interfaces:**
- Changes: `scheduler::run_scan_loop(store: Store, env_default_secs: u64)` (drops the `notifiers: Arc<...>` parameter; delivery now resolves per-check channels via `notify::deliver_event`).
- Changes: `scan_once` unchanged in signature; its returned events already carry `check_id` (Task 7).
- Behavior: on a `success` ping when the prior status was `down`, emit an `Up` (recovery) delivery; on a `fail` ping (or non-zero exitcode) when the prior status was `up`/`new`, emit a `Down` delivery. Both are spawned so the ping response is not blocked by delivery.

- [ ] **Step 1: Write the failing test** — replace the Plan 1 `overdue_dispatches_to_webhook` test in `tests/scheduler.rs` with a per-check-channel version, and add a recovery test:

```rust
use pingward::notify::{deliver_event, EventKind, NotificationEvent, RetryPolicy};
use pingward::models::{ChannelKind, NotifyStatus};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn overdue_downs_and_delivers_to_bound_channel() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&mock)
        .await;

    // build store with an overdue up check bound to a webhook channel
    let (store, id) = store_with_up_check(60, 30, 200).await;
    let now = Utc::now();
    let cid = store
        .create_channel(1, ChannelKind::Webhook, "hook", &format!("{{\"url\":\"{}\"}}", mock.uri()), now)
        .await
        .unwrap();
    store.bind_channel(id, cid).await.unwrap();

    let events = scan_once(&store, now).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].check_id, id);
    for ev in &events {
        deliver_event(&store, ev, RetryPolicy::default(), now).await;
    }
    assert_eq!(store.list_recent_notifications(id, 10).await.unwrap()[0].status, NotifyStatus::Ok);
}
```

(Keep `store_with_up_check` from Plan 1; it seeds user id=1 + project id=1, so `create_channel(1, ...)` is valid.)

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --test scheduler overdue_downs_and_delivers_to_bound_channel`
Expected: FAIL until `NotificationEvent.check_id` and delivery wiring compile. (`deliver_event` exists from Task 7.)

- [ ] **Step 3: Rewrite `run_scan_loop`** in `src/scheduler.rs`:

Replace the current `run_scan_loop` and its `notifiers`/`dispatch`/`log_dispatch_outcome` helpers with a cascade-aware loop. Update the imports at the top of `scheduler.rs`:

```rust
use crate::config::effective_scan_interval;
use crate::models::{Check, CheckStatus, ScheduleKind};
use crate::notify::{deliver_event, EventKind, NotificationEvent, RetryPolicy};
use crate::store::Store;
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use std::str::FromStr;
use tokio::time::{sleep, Duration as TokioDuration};
```
(Drop the `Arc`, `Notifier`, `NotifyError`, `dispatch`, `interval` imports — no longer used. Remove `log_dispatch_outcome` entirely.)

New loop:

```rust
/// Compute the loop's sleep interval: the smallest effective scan interval
/// across all active checks (spec §8 cascade), or `env_default` when there are
/// no active checks. Bounded to `>= 1s`.
fn loop_interval_secs(
    checks: &[Check],
    project_intervals: &std::collections::HashMap<i64, Option<i64>>,
    global_secs: Option<i64>,
    env_default: u64,
) -> u64 {
    checks
        .iter()
        .map(|c| {
            let project = project_intervals.get(&c.project_id).copied().flatten();
            effective_scan_interval(c.scan_interval_secs, project, global_secs, env_default)
        })
        .min()
        .unwrap_or(env_default.max(1))
}

/// Runs the scan loop forever. On each iteration it re-reads active checks,
/// resolves the cascade sleep interval, scans for overdue checks, and delivers
/// each resulting `Down` event to that check's bound channels. `Utc::now()` is
/// called only here so `scan_once` stays deterministic.
pub async fn run_scan_loop(store: Store, env_default_secs: u64) {
    loop {
        let now = Utc::now();
        match scan_once(&store, now).await {
            Ok(events) => {
                for ev in events {
                    let store = store.clone();
                    tokio::spawn(async move {
                        deliver_event(&store, &ev, RetryPolicy::default(), Utc::now()).await;
                    });
                }
            }
            Err(e) => tracing::error!("scan_once failed: {e}"),
        }

        // Resolve the next sleep from the cascade; failures fall back to the env default.
        let active = store.list_active_checks().await.unwrap_or_default();
        let projects = store.all_project_scan_intervals().await.unwrap_or_default();
        let global = store
            .get_setting("scan_interval")
            .await
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok());
        let secs = loop_interval_secs(&active, &projects, global, env_default_secs);
        sleep(TokioDuration::from_secs(secs)).await;
    }
}
```

- [ ] **Step 4: Wire recovery + fail delivery in `src/ping.rs`** — extend `apply`:

Add imports:
```rust
use crate::notify::{deliver_event, EventKind, NotificationEvent, RetryPolicy};
```
In `apply`, after computing `now` and before the `match kind`, capture the prior status; then spawn deliveries after the state update. Replace the `PingKind::Success` and `PingKind::Fail` arms:

```rust
    let prev_status = check.status;
    match kind {
        PingKind::Success => {
            let mut updated = check.clone();
            updated.last_ping_at = Some(now);
            let next = due_time(&updated);
            store
                .mark_ping(check.id, CheckStatus::Up, Some(now), None, next)
                .await?;
            if prev_status == CheckStatus::Down {
                spawn_delivery(store.clone(), check.id, check.name.clone(), check.project_id, EventKind::Up, now);
            }
        }
        PingKind::Fail => {
            store
                .mark_ping(check.id, CheckStatus::Down, Some(now), None, None)
                .await?;
            if matches!(prev_status, CheckStatus::Up | CheckStatus::New) {
                spawn_delivery(store.clone(), check.id, check.name.clone(), check.project_id, EventKind::Down, now);
            }
        }
        PingKind::Start => {
            store
                .mark_ping(check.id, check.status, None, Some(now), check.next_due_at)
                .await?;
        }
        PingKind::Log => { /* recorded only */ }
        PingKind::Exitcode => unreachable!("exitcode maps to Success/Fail above"),
    }
    Ok(StatusCode::OK)
}

/// Spawn a fire-and-forget delivery so the ping response is not blocked by
/// notification I/O. `store` is cheap to clone (holds an `Arc` pool).
fn spawn_delivery(
    store: Store,
    check_id: i64,
    check_name: String,
    project_id: i64,
    event: EventKind,
    now: chrono::DateTime<chrono::Utc>,
) {
    tokio::spawn(async move {
        let ev = NotificationEvent { check_id, check_name, event, at: now, project_id };
        deliver_event(&store, &ev, RetryPolicy::default(), now).await;
    });
}
```

Note: `apply` takes `store: &Store`; `store.clone()` yields an owned `Store` for the spawned task. The paused-check early return (Plan 1) still precedes this `match`, so paused checks neither transition nor deliver.

- [ ] **Step 5: Run — expect PASS**

Run: `cargo nextest run --test scheduler && cargo nextest run --test ping_api && cargo build`
Expected: PASS; binary compiles (note `main.rs` still calls the old `run_scan_loop` signature — Task 15 fixes `main.rs`; if `cargo build` fails only on `main.rs`'s `run_scan_loop` arity, that is expected and resolved in Task 15. To keep this task's build green, temporarily update the `main.rs` spawn to `scheduler::run_scan_loop(store.clone(), scan_interval_secs)` and drop the `notifiers` construction now — Task 15 finalizes the rest of `main.rs`.)

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/scheduler.rs src/ping.rs tests/scheduler.rs src/main.rs
git commit -m "feat: recovery + fail-ping notifications and cascade-aware scan loop"
```

---

### Task 9: Auth — session cookies, extractors, forward-auth decision

**Files:**
- Modify: `src/auth.rs`

**Interfaces:**
- Produces: `auth::SESSION_COOKIE: &str = "pingward_session"`; `auth::new_session_token() -> String` (UUIDv4); `auth::SESSION_TTL_DAYS: i64 = 30`.
- Produces: `auth::forward_auth_username(headers: &axum::http::HeaderMap, peer_ip: Option<std::net::IpAddr>, config: &Config) -> Option<String>` — returns the username from the configured forward-auth header **only** when `config.forward_auth_header` is set, the header is present, and `peer_ip`'s string form is in `config.trusted_proxies`.
- Produces extractors `auth::CurrentUser(pub models::User)` and `auth::AdminUser(pub models::User)`, both `impl FromRequestParts<AppState>`, rejecting unauthenticated requests with `Redirect::to("/login")` and non-admins with `StatusCode::FORBIDDEN`.

- [ ] **Step 1: Write the failing test** — append to `src/auth.rs` `mod tests`:

```rust
    use crate::config::Config;
    use axum::http::{HeaderMap, HeaderValue};
    use std::net::{IpAddr, Ipv4Addr};

    fn cfg_with_forward_auth() -> Config {
        Config::from_map(|k| match k {
            "PINGWARD_FORWARD_AUTH_HEADER" => Some("X-Forwarded-User".into()),
            "PINGWARD_TRUSTED_PROXIES" => Some("10.0.0.1".into()),
            _ => None,
        })
    }

    #[test]
    fn forward_auth_honored_only_from_trusted_proxy() {
        let cfg = cfg_with_forward_auth();
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-User", HeaderValue::from_static("alice"));
        let trusted = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let untrusted = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));

        assert_eq!(forward_auth_username(&headers, Some(trusted), &cfg), Some("alice".into()));
        assert_eq!(forward_auth_username(&headers, Some(untrusted), &cfg), None);
        assert_eq!(forward_auth_username(&headers, None, &cfg), None);
    }

    #[test]
    fn forward_auth_disabled_when_unconfigured() {
        let cfg = Config::from_map(|_| None);
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-User", HeaderValue::from_static("alice"));
        let trusted = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(forward_auth_username(&headers, Some(trusted), &cfg), None);
    }

    #[test]
    fn session_token_is_unique_uuid() {
        let a = new_session_token();
        let b = new_session_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36); // hyphenated uuid
    }
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --lib auth`
Expected: FAIL.

- [ ] **Step 3: Implement** — prepend to `src/auth.rs` (above the hashing fns):

```rust
use crate::models::User;
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::{request::Parts, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
use std::net::{IpAddr, SocketAddr};

pub const SESSION_COOKIE: &str = "pingward_session";
pub const SESSION_TTL_DAYS: i64 = 30;

pub fn new_session_token() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Returns the forward-auth username iff forward-auth is configured, the header
/// is present and valid UTF-8, and `peer_ip` is a configured trusted proxy.
pub fn forward_auth_username(
    headers: &HeaderMap,
    peer_ip: Option<IpAddr>,
    config: &crate::config::Config,
) -> Option<String> {
    let header_name = config.forward_auth_header.as_ref()?;
    let peer = peer_ip?;
    if !config.trusted_proxies.iter().any(|p| p == &peer.to_string()) {
        return None;
    }
    headers
        .get(header_name.as_str())
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
}
```

Then add the extractors (below the hashing fns, above `mod tests`):

```rust
/// Resolve the authenticated user from the session cookie, or (failing that)
/// from a trusted forward-auth header — auto-provisioning a non-admin,
/// password-less user for a first-seen forward-auth identity.
async fn resolve_user(parts: &mut Parts, state: &AppState) -> Option<User> {
    let now = Utc::now();
    let jar = CookieJar::from_headers(&parts.headers);
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        if let Ok(Some(user)) = state.store.find_session_user(cookie.value(), now).await {
            return Some(user);
        }
    }
    // forward-auth fallback
    let peer_ip = parts
        .extensions
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());
    if let Some(username) = forward_auth_username(&parts.headers, peer_ip, &state.config) {
        match state.store.find_user_by_username(&username).await {
            Ok(Some(user)) => return Some(user),
            Ok(None) => {
                if let Ok(id) = state.store.create_user(&username, None, false, now).await {
                    return state.store.find_user_by_id(id).await.ok().flatten();
                }
            }
            Err(_) => {}
        }
    }
    None
}

pub struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        match resolve_user(parts, state).await {
            Some(user) => Ok(CurrentUser(user)),
            None => Err(Redirect::to("/login").into_response()),
        }
    }
}

pub struct AdminUser(pub User);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let CurrentUser(user) = CurrentUser::from_request_parts(parts, state).await?;
        if user.is_admin {
            Ok(AdminUser(user))
        } else {
            Err((StatusCode::FORBIDDEN, "admin only").into_response())
        }
    }
}
```

- [ ] **Step 4: Run — expect PASS**

Run: `cargo nextest run --lib auth`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/auth.rs
git commit -m "feat: session cookies, auth extractors, forward-auth trust"
```

---

### Task 10: Web scaffolding — base layout, setup, login, logout

**Files:**
- Create: `templates/base.html`, `templates/setup.html`, `templates/login.html`
- Replace: `src/web.rs` (placeholder → real module)
- Modify: `src/lib.rs` (merge `web::routes()`)
- Create: `tests/auth_web.rs`

**Interfaces:**
- Consumes: `state::AppState`, `store::Store`, `auth::{hash_password, verify_password, new_session_token, CurrentUser, SESSION_COOKIE, SESSION_TTL_DAYS}`.
- Produces: `web::routes() -> Router<AppState>` mounting `GET /` (dashboard, redirects to `/setup` when no users, `/login` when unauthenticated), `GET /setup` + `POST /setup`, `GET /login` + `POST /login`, `POST /logout`.
- Produces render helper `web::render<T: askama::Template>(t: &T) -> Result<Html<String>, AppError>`.

- [ ] **Step 1: Write templates**

`templates/base.html`:
```html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>pingward</title>
  <style>
    body { font-family: system-ui, sans-serif; max-width: 900px; margin: 2rem auto; padding: 0 1rem; }
    nav a { margin-right: 1rem; }
    table { border-collapse: collapse; width: 100%; }
    th, td { border: 1px solid #ccc; padding: .4rem .6rem; text-align: left; }
    .status-up { color: #2e7d32; } .status-down { color: #c62828; }
    .status-new, .status-paused { color: #757575; }
    form.inline { display: inline; }
    label { display: block; margin: .5rem 0; }
  </style>
</head>
<body>
  <nav>
    <a href="/">Dashboard</a>
    {% if show_nav %}<a href="/settings">Settings</a>
    <form class="inline" method="post" action="/logout"><button type="submit">Log out</button></form>{% endif %}
  </nav>
  {% block body %}{% endblock %}
</body>
</html>
```

`templates/setup.html`:
```html
{% extends "base.html" %}
{% block body %}
<h1>Create the first admin</h1>
{% if error %}<p class="status-down">{{ error }}</p>{% endif %}
<form method="post" action="/setup">
  <label>Username <input name="username" required></label>
  <label>Password <input name="password" type="password" required></label>
  <button type="submit">Create admin</button>
</form>
{% endblock %}
```

`templates/login.html`:
```html
{% extends "base.html" %}
{% block body %}
<h1>Log in</h1>
{% if error %}<p class="status-down">{{ error }}</p>{% endif %}
<form method="post" action="/login">
  <label>Username <input name="username" required></label>
  <label>Password <input name="password" type="password" required></label>
  <button type="submit">Log in</button>
</form>
{% endblock %}
```

Note: `base.html` references `show_nav`; every template struct that extends it must expose a `show_nav: bool` field. `setup.html`/`login.html` set it to `false`.

- [ ] **Step 2: Write the failing integration test** — `tests/auth_web.rs`:

```rust
use axum_test::TestServer;
use pingward::{app, config::Config, db, state::AppState, store::Store};

async fn server() -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let mut server = TestServer::new(app(state)).unwrap();
    server.do_save_cookies(); // persist Set-Cookie between requests
    (server, store)
}

#[tokio::test]
async fn setup_creates_admin_then_dashboard_loads() {
    let (server, store) = server().await;

    // With no users, root redirects to /setup.
    let res = server.get("/").await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/setup");

    // Create the first admin.
    let res = server
        .post("/setup")
        .form(&[("username", "admin"), ("password", "pw12345")])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(store.count_users().await.unwrap(), 1);
    let admin = store.find_user_by_username("admin").await.unwrap().unwrap();
    assert!(admin.is_admin);

    // Now authenticated (cookie saved) — dashboard renders 200.
    server.get("/").await.assert_status_ok();
}

#[tokio::test]
async fn login_logout_cycle() {
    let (server, store) = server().await;
    let phc = pingward::auth::hash_password("secret1").unwrap();
    store.create_user("bob", Some(&phc), false, chrono::Utc::now()).await.unwrap();

    // wrong password → back to login with 200 + error
    server.post("/login").form(&[("username", "bob"), ("password", "nope")]).await.assert_status_ok();

    // right password → redirect, cookie set
    let res = server.post("/login").form(&[("username", "bob"), ("password", "secret1")]).await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    server.get("/").await.assert_status_ok();

    // logout → redirect, then root bounces to /login
    server.post("/logout").await.assert_status(axum::http::StatusCode::SEE_OTHER);
    let res = server.get("/").await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/login");
}
```

- [ ] **Step 3: Run — expect FAIL**

Run: `cargo nextest run --test auth_web`
Expected: FAIL (routes missing).

- [ ] **Step 4: Implement `src/web.rs`** (replace the placeholder):

```rust
use crate::auth::{
    hash_password, new_session_token, verify_password, CurrentUser, SESSION_COOKIE, SESSION_TTL_DAYS,
};
use crate::error::AppError;
use crate::models::{Check, Project};
use crate::state::AppState;
use crate::store::Store;
use askama::Template;
use axum::extract::State;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Form;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{Duration, Utc};
use serde::Deserialize;

pub fn render<T: Template>(t: &T) -> Result<Html<String>, AppError> {
    let body = t
        .render()
        .map_err(|e| AppError::Other(Box::new(e)))?;
    Ok(Html(body))
}

pub fn routes() -> Router<AppState> {
    use axum::Router;
    Router::new()
        .route("/", get(dashboard))
        .route("/setup", get(setup_page).post(setup_submit))
        .route("/login", get(login_page).post(login_submit))
        .route("/logout", post(logout))
}

// --- templates ---
#[derive(Template)]
#[template(path = "setup.html")]
struct SetupTemplate {
    show_nav: bool,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    show_nav: bool,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    show_nav: bool,
    projects: Vec<ProjectRow>,
}

pub struct ProjectRow {
    pub project: Project,
    pub checks: Vec<Check>,
}

// --- forms ---
#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
}

// --- handlers ---
async fn setup_page(State(state): State<AppState>) -> Result<Response, AppError> {
    if state.store.count_users().await? > 0 {
        return Ok(Redirect::to("/login").into_response());
    }
    Ok(render(&SetupTemplate { show_nav: false, error: None })?.into_response())
}

async fn setup_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(creds): Form<Credentials>,
) -> Result<Response, AppError> {
    if state.store.count_users().await? > 0 {
        return Ok(Redirect::to("/login").into_response());
    }
    if creds.username.is_empty() || creds.password.is_empty() {
        return Ok(render(&SetupTemplate {
            show_nav: false,
            error: Some("username and password are required".into()),
        })?
        .into_response());
    }
    let phc = hash_password(&creds.password).map_err(|e| AppError::Other(Box::new(e)))?;
    let uid = state
        .store
        .create_user(&creds.username, Some(&phc), true, Utc::now())
        .await?;
    let jar = start_session(&state.store, jar, uid).await?;
    Ok((jar, Redirect::to("/")).into_response())
}

async fn login_page(State(state): State<AppState>) -> Result<Response, AppError> {
    if state.store.count_users().await? == 0 {
        return Ok(Redirect::to("/setup").into_response());
    }
    Ok(render(&LoginTemplate { show_nav: false, error: None })?.into_response())
}

async fn login_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(creds): Form<Credentials>,
) -> Result<Response, AppError> {
    let user = state.store.find_user_by_username(&creds.username).await?;
    let ok = user
        .as_ref()
        .and_then(|u| u.password_hash.as_deref())
        .map(|phc| verify_password(&creds.password, phc))
        .unwrap_or(false);
    if !ok {
        return Ok(render(&LoginTemplate {
            show_nav: false,
            error: Some("invalid username or password".into()),
        })?
        .into_response());
    }
    let jar = start_session(&state.store, jar, user.unwrap().id).await?;
    Ok((jar, Redirect::to("/")).into_response())
}

async fn logout(State(state): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        state.store.delete_session(cookie.value()).await?;
    }
    let jar = jar.remove(Cookie::from(SESSION_COOKIE));
    Ok((jar, Redirect::to("/login")).into_response())
}

async fn dashboard(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
) -> Result<Response, AppError> {
    if state.store.count_users().await? == 0 {
        return Ok(Redirect::to("/setup").into_response());
    }
    let CurrentUser(user) = match user {
        Some(u) => u,
        None => return Ok(Redirect::to("/login").into_response()),
    };
    let projects = state.store.list_projects_for_user(user.id).await?;
    let mut rows = Vec::with_capacity(projects.len());
    for project in projects {
        let checks = state.store.list_checks_for_project(project.id).await?;
        rows.push(ProjectRow { project, checks });
    }
    Ok(render(&DashboardTemplate { show_nav: true, projects: rows })?.into_response())
}

/// Create a session row and return a jar carrying the session cookie.
async fn start_session(store: &Store, jar: CookieJar, user_id: i64) -> Result<CookieJar, AppError> {
    let token = new_session_token();
    let expires = Utc::now() + Duration::days(SESSION_TTL_DAYS);
    store.create_session(&token, user_id, expires).await?;
    let cookie = Cookie::build((SESSION_COOKIE, token))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .build();
    Ok(jar.add(cookie))
}
```

Note on `Option<CurrentUser>`: axum derives `OptionalFromRequestParts` for an extractor whose `Rejection: IntoResponse`, so `Option<CurrentUser>` is usable in the `dashboard` handler and yields `None` instead of redirecting — letting `dashboard` choose `/setup` vs `/login`. (This mirrors the `ClientIp` note in `ping.rs`; if the pinned axum version does not provide the blanket optional impl, replace `user: Option<CurrentUser>` by reading the session directly via `state.store.find_session_user` using a `CookieJar` extractor — same logic, no extractor option.)

The `dashboard.html` template referenced above is created in Task 11 (it needs the project/check rows). For this task, add a **minimal** `templates/dashboard.html` so the module compiles:
```html
{% extends "base.html" %}
{% block body %}<h1>Dashboard</h1>{% for r in projects %}<h2>{{ r.project.name }}</h2>{% endfor %}
<p><a href="/projects/new">New project</a></p>
{% endblock %}
```

- [ ] **Step 5: Merge routes in `src/lib.rs`**

```rust
pub fn app(state: AppState) -> Router {
    Router::new()
        .merge(web::routes())
        .merge(ping::routes())
        .with_state(state)
}
```
(The `/healthz` route can move into `web::routes()` or stay; keep it by adding `.route("/healthz", get(|| async { "ok" }))` to `web::routes()` and drop it here, OR keep it here before the merges. Keep it here.)

Corrected `app`:
```rust
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(web::routes())
        .merge(ping::routes())
        .with_state(state)
}
```

- [ ] **Step 6: Run — expect PASS**

Run: `cargo nextest run --test auth_web`
Expected: PASS (2 tests). Then `cargo nextest run` to confirm the whole suite is green.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add templates/base.html templates/setup.html templates/login.html templates/dashboard.html src/web.rs src/lib.rs tests/auth_web.rs
git commit -m "feat: web scaffolding with setup, login, logout, dashboard"
```

---

### Task 11: Web — projects CRUD

**Files:**
- Modify: `src/web.rs`
- Create: `templates/project_form.html`, `templates/project.html`; expand `templates/dashboard.html`
- Modify: `tests/auth_web.rs`

**Interfaces:**
- Consumes: `Store::{create_project, find_project, list_checks_for_project, list_channels_for_project, update_project, delete_project}`.
- Produces routes: `GET /projects/new`, `POST /projects`, `GET /projects/{id}`, `GET /projects/{id}/edit`, `POST /projects/{id}` (update), `POST /projects/{id}/delete`.
- Ownership rule: every project handler loads the project and returns `404` if `project.user_id != current_user.id` (via a shared `owned_project` helper).

- [ ] **Step 1: Write the failing test** — append to `tests/auth_web.rs`:

```rust
async fn logged_in_server() -> (TestServer, Store, i64) {
    let (server, store) = server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store.create_user("admin", Some(&phc), true, chrono::Utc::now()).await.unwrap();
    server.post("/login").form(&[("username", "admin"), ("password", "pw")]).await;
    (server, store, uid)
}

#[tokio::test]
async fn create_and_delete_project() {
    let (server, store, uid) = logged_in_server().await;

    let res = server.post("/projects").form(&[("name", "web"), ("scan_interval_secs", "")]).await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let projects = store.list_projects_for_user(uid).await.unwrap();
    assert_eq!(projects.len(), 1);
    let pid = projects[0].id;

    server.get(&format!("/projects/{pid}")).await.assert_status_ok();

    server.post(&format!("/projects/{pid}/delete")).await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.list_projects_for_user(uid).await.unwrap().is_empty());
}

#[tokio::test]
async fn cannot_view_another_users_project() {
    let (server, store, _uid) = logged_in_server().await;
    // project owned by a different user
    let other = store.create_user("other", Some("x"), false, chrono::Utc::now()).await.unwrap();
    let pid = store.create_project(other, "secret", None, chrono::Utc::now()).await.unwrap();
    server.get(&format!("/projects/{pid}")).await
        .assert_status(axum::http::StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --test auth_web::create_and_delete_project`
Expected: FAIL.

- [ ] **Step 3: Templates**

`templates/project_form.html`:
```html
{% extends "base.html" %}
{% block body %}
<h1>{{ heading }}</h1>
<form method="post" action="{{ action }}">
  <label>Name <input name="name" value="{{ name }}" required></label>
  <label>Scan interval (secs, blank = inherit)
    <input name="scan_interval_secs" value="{{ scan_interval_secs }}"></label>
  <button type="submit">Save</button>
</form>
{% endblock %}
```

`templates/project.html`:
```html
{% extends "base.html" %}
{% block body %}
<h1>{{ project.name }}</h1>
<p>
  <a href="/projects/{{ project.id }}/edit">Edit</a>
  <a href="/projects/{{ project.id }}/checks/new">New check</a>
  <a href="/projects/{{ project.id }}/channels/new">New channel</a>
  <form class="inline" method="post" action="/projects/{{ project.id }}/delete"
        onsubmit="return confirm('Delete project?')"><button>Delete project</button></form>
</p>
<h2>Checks</h2>
<table><tr><th>Name</th><th>Status</th><th>Schedule</th><th></th></tr>
{% for c in checks %}
<tr>
  <td><a href="/checks/{{ c.id }}">{{ c.name }}</a></td>
  <td class="status-{{ c.status.as_str() }}">{{ c.status.as_str() }}</td>
  <td>{{ c.schedule_kind.as_str() }}</td>
  <td><a href="/checks/{{ c.id }}/edit">edit</a></td>
</tr>
{% endfor %}
</table>
<h2>Channels</h2>
<table><tr><th>Name</th><th>Kind</th><th></th></tr>
{% for ch in channels %}
<tr><td>{{ ch.name }}</td><td>{{ ch.kind.as_str() }}</td>
  <td><form class="inline" method="post" action="/channels/{{ ch.id }}/delete"><button>delete</button></form></td></tr>
{% endfor %}
</table>
{% endblock %}
```

Expand `templates/dashboard.html`:
```html
{% extends "base.html" %}
{% block body %}
<h1>Dashboard</h1>
<p><a href="/projects/new">New project</a></p>
{% for r in projects %}
<h2><a href="/projects/{{ r.project.id }}">{{ r.project.name }}</a></h2>
<table><tr><th>Check</th><th>Status</th></tr>
{% for c in r.checks %}
<tr><td><a href="/checks/{{ c.id }}">{{ c.name }}</a></td>
  <td class="status-{{ c.status.as_str() }}">{{ c.status.as_str() }}</td></tr>
{% endfor %}
</table>
{% endfor %}
{% endblock %}
```

- [ ] **Step 4: Implement handlers** — add to `src/web.rs`:

```rust
use crate::models::Channel;
use axum::extract::Path;

#[derive(Template)]
#[template(path = "project_form.html")]
struct ProjectFormTemplate {
    show_nav: bool,
    heading: String,
    action: String,
    name: String,
    scan_interval_secs: String,
}

#[derive(Template)]
#[template(path = "project.html")]
struct ProjectTemplate {
    show_nav: bool,
    project: Project,
    checks: Vec<Check>,
    channels: Vec<Channel>,
}

#[derive(Deserialize)]
struct ProjectForm {
    name: String,
    scan_interval_secs: String,
}

fn parse_opt_i64(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() { None } else { t.parse::<i64>().ok() }
}

/// Load a project and enforce ownership, returning `AppError::NotFound` if it
/// does not exist or belongs to another user.
async fn owned_project(store: &Store, id: i64, user_id: i64) -> Result<Project, AppError> {
    let p = store.find_project(id).await?.ok_or(AppError::NotFound)?;
    if p.user_id != user_id {
        return Err(AppError::NotFound);
    }
    Ok(p)
}

async fn project_new(CurrentUser(_u): CurrentUser) -> Result<Response, AppError> {
    Ok(render(&ProjectFormTemplate {
        show_nav: true,
        heading: "New project".into(),
        action: "/projects".into(),
        name: String::new(),
        scan_interval_secs: String::new(),
    })?
    .into_response())
}

async fn project_create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Form(form): Form<ProjectForm>,
) -> Result<Response, AppError> {
    let id = state
        .store
        .create_project(user.id, &form.name, parse_opt_i64(&form.scan_interval_secs), Utc::now())
        .await?;
    Ok(Redirect::to(&format!("/projects/{id}")).into_response())
}

async fn project_show(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = owned_project(&state.store, id, user.id).await?;
    let checks = state.store.list_checks_for_project(id).await?;
    let channels = state.store.list_channels_for_project(id).await?;
    Ok(render(&ProjectTemplate { show_nav: true, project, checks, channels })?.into_response())
}

async fn project_edit(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = owned_project(&state.store, id, user.id).await?;
    Ok(render(&ProjectFormTemplate {
        show_nav: true,
        heading: "Edit project".into(),
        action: format!("/projects/{id}"),
        name: project.name,
        scan_interval_secs: project.scan_interval_secs.map(|v| v.to_string()).unwrap_or_default(),
    })?
    .into_response())
}

async fn project_update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<ProjectForm>,
) -> Result<Response, AppError> {
    owned_project(&state.store, id, user.id).await?;
    state
        .store
        .update_project(id, &form.name, parse_opt_i64(&form.scan_interval_secs))
        .await?;
    Ok(Redirect::to(&format!("/projects/{id}")).into_response())
}

async fn project_delete(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    owned_project(&state.store, id, user.id).await?;
    state.store.delete_project(id).await?;
    Ok(Redirect::to("/").into_response())
}
```

Register in `web::routes()`:
```rust
        .route("/projects/new", get(project_new))
        .route("/projects", post(project_create))
        .route("/projects/{id}", get(project_show).post(project_update))
        .route("/projects/{id}/edit", get(project_edit))
        .route("/projects/{id}/delete", post(project_delete))
```

- [ ] **Step 5: Run — expect PASS**

Run: `cargo nextest run --test auth_web`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add templates/project_form.html templates/project.html templates/dashboard.html src/web.rs tests/auth_web.rs
git commit -m "feat: project CRUD with ownership enforcement"
```

---

### Task 12: Web — checks CRUD (create, edit, pause/resume, regenerate UUID, delete)

**Files:**
- Modify: `src/web.rs`
- Create: `templates/check_form.html`, `templates/check.html`
- Modify: `tests/auth_web.rs`

**Interfaces:**
- Consumes: `Store::{create_check, find_check, update_check_schedule, set_status, regenerate_uuid, delete_check, list_recent_pings, list_recent_notifications, channels_for_check, list_channels_for_project, bound_channel_ids}`; `scheduler::due_time`; `config.base_url` for rendering the ping URL.
- Produces routes: `GET /projects/{pid}/checks/new`, `POST /projects/{pid}/checks`, `GET /checks/{id}`, `GET /checks/{id}/edit`, `POST /checks/{id}` (update), `POST /checks/{id}/pause`, `POST /checks/{id}/resume`, `POST /checks/{id}/regenerate`, `POST /checks/{id}/delete`.
- Validation: `cron` schedule kind requires a `cron_expr` that parses via `cron::Schedule::from_str`; `period` requires a positive `period_secs`. Invalid input re-renders the form with an error (no DB write).
- Ownership: a check is reachable only if its project is owned by the current user (`owned_check` helper joins through the project).

- [ ] **Step 1: Write the failing test** — append to `tests/auth_web.rs`:

```rust
async fn server_with_project() -> (TestServer, Store, i64) {
    let (server, store, uid) = logged_in_server().await;
    let pid = store.create_project(uid, "web", None, chrono::Utc::now()).await.unwrap();
    (server, store, pid)
}

#[tokio::test]
async fn create_check_and_pause_resume() {
    let (server, store, pid) = server_with_project().await;

    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("name", "backup"),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("grace_secs", "300"),
            ("cron_expr", ""),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let checks = store.list_checks_for_project(pid).await.unwrap();
    assert_eq!(checks.len(), 1);
    let cid = checks[0].id;

    server.post(&format!("/checks/{cid}/pause")).await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(store.find_check(cid).await.unwrap().unwrap().status, pingward::models::CheckStatus::Paused);

    server.post(&format!("/checks/{cid}/resume")).await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(store.find_check(cid).await.unwrap().unwrap().status, pingward::models::CheckStatus::New);
}

#[tokio::test]
async fn invalid_cron_is_rejected() {
    let (server, store, pid) = server_with_project().await;
    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("name", "bad"),
            ("schedule_kind", "cron"),
            ("period_secs", ""),
            ("grace_secs", "60"),
            ("cron_expr", "not a cron"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
        ])
        .await;
    res.assert_status_ok(); // re-rendered form, not a redirect
    assert!(store.list_checks_for_project(pid).await.unwrap().is_empty());
}

#[tokio::test]
async fn regenerate_uuid_changes_ping_url() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(pid, "job", "old-uuid", pingward::models::ScheduleKind::Period, Some(60), 30, None, "UTC")
        .await.unwrap();
    server.post(&format!("/checks/{cid}/regenerate")).await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_ne!(store.find_check(cid).await.unwrap().unwrap().ping_uuid, "old-uuid");
}
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --test auth_web::create_check_and_pause_resume`
Expected: FAIL.

- [ ] **Step 3: Templates**

`templates/check_form.html`:
```html
{% extends "base.html" %}
{% block body %}
<h1>{{ heading }}</h1>
{% if error %}<p class="status-down">{{ error }}</p>{% endif %}
<form method="post" action="{{ action }}">
  <label>Name <input name="name" value="{{ name }}" required></label>
  <label>Schedule kind
    <select name="schedule_kind">
      <option value="period"{% if schedule_kind == "period" %} selected{% endif %}>period</option>
      <option value="cron"{% if schedule_kind == "cron" %} selected{% endif %}>cron</option>
    </select></label>
  <label>Period seconds (period mode) <input name="period_secs" value="{{ period_secs }}"></label>
  <label>Cron expr (cron mode, 6-field "sec min hour dom mon dow") <input name="cron_expr" value="{{ cron_expr }}"></label>
  <label>Grace seconds <input name="grace_secs" value="{{ grace_secs }}" required></label>
  <label>Timezone <input name="timezone" value="{{ timezone }}" required></label>
  <label>Scan interval secs (blank = inherit) <input name="scan_interval_secs" value="{{ scan_interval_secs }}"></label>
  <button type="submit">Save</button>
</form>
{% endblock %}
```

`templates/check.html`:
```html
{% extends "base.html" %}
{% block body %}
<h1>{{ check.name }} <span class="status-{{ check.status.as_str() }}">({{ check.status.as_str() }})</span></h1>
<p>Ping URL: <code>{{ ping_url }}</code></p>
<p>
  <a href="/checks/{{ check.id }}/edit">Edit</a>
  {% if check.status.as_str() == "paused" %}
    <form class="inline" method="post" action="/checks/{{ check.id }}/resume"><button>Resume</button></form>
  {% else %}
    <form class="inline" method="post" action="/checks/{{ check.id }}/pause"><button>Pause</button></form>
  {% endif %}
  <form class="inline" method="post" action="/checks/{{ check.id }}/regenerate"><button>Regenerate URL</button></form>
  <form class="inline" method="post" action="/checks/{{ check.id }}/delete"
        onsubmit="return confirm('Delete check?')"><button>Delete</button></form>
</p>

<h2>Channels</h2>
<form method="post" action="/checks/{{ check.id }}/channels">
  {% for cb in channel_boxes %}
  <label><input type="checkbox" name="channel_ids" value="{{ cb.id }}"{% if cb.bound %} checked{% endif %}> {{ cb.name }} ({{ cb.kind }})</label>
  {% endfor %}
  <button type="submit">Save channels</button>
</form>

<h2>Recent pings</h2>
<table><tr><th>When</th><th>Kind</th><th>Exit</th></tr>
{% for p in pings %}<tr><td>{{ p.created_at }}</td><td>{{ p.kind.as_str() }}</td><td>{{ p.exit_code_display }}</td></tr>{% endfor %}
</table>

<h2>Recent notifications</h2>
<table><tr><th>When</th><th>Event</th><th>Status</th></tr>
{% for n in notifications %}<tr><td>{{ n.created_at }}</td><td>{{ n.event.as_str() }}</td><td>{{ n.status.as_str() }}</td></tr>{% endfor %}
</table>
{% endblock %}
```

Note: `p.exit_code_display` — askama cannot call `.map(...).unwrap_or_default()` on an `Option<i64>` inline cleanly, so pass a pre-rendered row struct (below).

- [ ] **Step 4: Implement handlers** — add to `src/web.rs`:

```rust
use crate::models::{CheckStatus, Notification, Ping, ScheduleKind};
use crate::scheduler::due_time;
use cron::Schedule;
use std::str::FromStr;

#[derive(Deserialize)]
struct CheckForm {
    name: String,
    schedule_kind: String,
    period_secs: String,
    cron_expr: String,
    grace_secs: String,
    timezone: String,
    scan_interval_secs: String,
}

struct PingRow { created_at: String, kind: PingKindWrap, exit_code_display: String }
struct PingKindWrap(crate::models::PingKind);
impl PingKindWrap { fn as_str(&self) -> &'static str { self.0.as_str() } }

struct ChannelBox { id: i64, name: String, kind: &'static str, bound: bool }

#[derive(Template)]
#[template(path = "check_form.html")]
struct CheckFormTemplate {
    show_nav: bool,
    heading: String,
    action: String,
    error: Option<String>,
    name: String,
    schedule_kind: String,
    period_secs: String,
    cron_expr: String,
    grace_secs: String,
    timezone: String,
    scan_interval_secs: String,
}

#[derive(Template)]
#[template(path = "check.html")]
struct CheckTemplate {
    show_nav: bool,
    check: Check,
    ping_url: String,
    channel_boxes: Vec<ChannelBox>,
    pings: Vec<PingRow>,
    notifications: Vec<Notification>,
}
```

Because the `pings` list needs `kind.as_str()` and a formatted exit code, map rows before rendering. To keep the template simple, use `PingRow` (created above). Wire `check.html`'s `{% for p in pings %}` to reference `p.kind.as_str()` — since `PingRow.kind` is a `PingKindWrap` exposing `as_str`, this compiles. (Alternatively expose `Ping` directly and render `{{ p.kind.as_str() }}` / a helper; `PingRow` is used here to pre-format `exit_code_display`.)

Handlers:

```rust
/// Load a check and enforce ownership through its project.
async fn owned_check(store: &Store, id: i64, user_id: i64) -> Result<Check, AppError> {
    let check = store.find_check(id).await?.ok_or(AppError::NotFound)?;
    owned_project(store, check.project_id, user_id).await?;
    Ok(check)
}

fn empty_check_form(heading: &str, action: String) -> CheckFormTemplate {
    CheckFormTemplate {
        show_nav: true,
        heading: heading.into(),
        action,
        error: None,
        name: String::new(),
        schedule_kind: "period".into(),
        period_secs: String::new(),
        cron_expr: String::new(),
        grace_secs: "300".into(),
        timezone: "UTC".into(),
        scan_interval_secs: String::new(),
    }
}

/// Validate a check form into (kind, period_secs, grace_secs, cron_expr). Returns
/// `Err(message)` on invalid input.
fn validate_check(form: &CheckForm) -> Result<(ScheduleKind, Option<i64>, i64, Option<String>), String> {
    let grace = parse_opt_i64(&form.grace_secs).ok_or("grace_secs must be an integer")?;
    if grace < 0 { return Err("grace_secs must be >= 0".into()); }
    let kind = ScheduleKind::from_str(&form.schedule_kind).map_err(|_| "invalid schedule kind".to_string())?;
    match kind {
        ScheduleKind::Period => {
            let secs = parse_opt_i64(&form.period_secs).ok_or("period_secs required for period mode")?;
            if secs <= 0 { return Err("period_secs must be > 0".into()); }
            Ok((kind, Some(secs), grace, None))
        }
        ScheduleKind::Cron => {
            let expr = form.cron_expr.trim();
            if expr.is_empty() { return Err("cron_expr required for cron mode".into()); }
            Schedule::from_str(expr).map_err(|e| format!("invalid cron expression: {e}"))?;
            Ok((kind, None, grace, Some(expr.to_string())))
        }
    }
}

async fn check_new(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    Ok(render(&empty_check_form("New check", format!("/projects/{pid}/checks")))?.into_response())
}

async fn check_create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
    Form(form): Form<CheckForm>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    let (kind, period_secs, grace, cron_expr) = match validate_check(&form) {
        Ok(v) => v,
        Err(msg) => {
            let mut t = empty_check_form("New check", format!("/projects/{pid}/checks"));
            t.error = Some(msg);
            t.name = form.name;
            t.schedule_kind = form.schedule_kind;
            t.period_secs = form.period_secs;
            t.cron_expr = form.cron_expr;
            t.grace_secs = form.grace_secs;
            t.timezone = form.timezone;
            t.scan_interval_secs = form.scan_interval_secs;
            return Ok(render(&t)?.into_response());
        }
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let id = state
        .store
        .create_check(pid, &form.name, &uuid, kind, period_secs, grace, cron_expr.as_deref(), &form.timezone)
        .await?;
    state
        .store
        .update_check_schedule(id, &form.name, kind, period_secs, grace, cron_expr.as_deref(), &form.timezone, parse_opt_i64(&form.scan_interval_secs))
        .await?;
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}

async fn check_show(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    let ping_url = format!("{}/ping/{}", state.config.base_url.trim_end_matches('/'), check.ping_uuid);
    let bound = state.store.bound_channel_ids(id).await?;
    let channel_boxes = state
        .store
        .list_channels_for_project(check.project_id)
        .await?
        .into_iter()
        .map(|c| ChannelBox { id: c.id, name: c.name, kind: c.kind.as_str(), bound: bound.contains(&c.id) })
        .collect();
    let pings = state
        .store
        .list_recent_pings(id, 20)
        .await?
        .into_iter()
        .map(|p| PingRow {
            created_at: p.created_at.to_rfc3339(),
            kind: PingKindWrap(p.kind),
            exit_code_display: p.exit_code.map(|c| c.to_string()).unwrap_or_default(),
        })
        .collect();
    let notifications = state.store.list_recent_notifications(id, 20).await?;
    Ok(render(&CheckTemplate { show_nav: true, check, ping_url, channel_boxes, pings, notifications })?.into_response())
}

async fn check_edit(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    Ok(render(&CheckFormTemplate {
        show_nav: true,
        heading: "Edit check".into(),
        action: format!("/checks/{id}"),
        error: None,
        name: check.name,
        schedule_kind: check.schedule_kind.as_str().into(),
        period_secs: check.period_secs.map(|v| v.to_string()).unwrap_or_default(),
        cron_expr: check.cron_expr.unwrap_or_default(),
        grace_secs: check.grace_secs.to_string(),
        timezone: check.timezone,
        scan_interval_secs: check.scan_interval_secs.map(|v| v.to_string()).unwrap_or_default(),
    })?
    .into_response())
}

async fn check_update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<CheckForm>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    let (kind, period_secs, grace, cron_expr) = match validate_check(&form) {
        Ok(v) => v,
        Err(msg) => {
            let mut t = CheckFormTemplate {
                show_nav: true, heading: "Edit check".into(), action: format!("/checks/{id}"),
                error: Some(msg), name: form.name, schedule_kind: form.schedule_kind,
                period_secs: form.period_secs, cron_expr: form.cron_expr, grace_secs: form.grace_secs,
                timezone: form.timezone, scan_interval_secs: form.scan_interval_secs,
            };
            let _ = &check;
            return Ok(render(&t)?.into_response());
        }
    };
    state
        .store
        .update_check_schedule(id, &form.name, kind, period_secs, grace, cron_expr.as_deref(), &form.timezone, parse_opt_i64(&form.scan_interval_secs))
        .await?;
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}

async fn check_pause(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    owned_check(&state.store, id, user.id).await?;
    state.store.set_status(id, CheckStatus::Paused).await?;
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}

async fn check_resume(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    owned_check(&state.store, id, user.id).await?;
    state.store.set_status(id, CheckStatus::New).await?;
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}

async fn check_regenerate(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    owned_check(&state.store, id, user.id).await?;
    state.store.regenerate_uuid(id, &uuid::Uuid::new_v4().to_string()).await?;
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}

async fn check_delete(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    state.store.delete_check(id).await?;
    Ok(Redirect::to(&format!("/projects/{}", check.project_id)).into_response())
}
```

Note: `due_time` is imported for the ping-URL/next-due display but only `ping_url` is shown here; if clippy flags `due_time` as unused, drop its import (the check detail page shows status, not next-due, in this minimal cut). Keep the import only if you also render `check.next_due_at`.

Register in `web::routes()`:
```rust
        .route("/projects/{pid}/checks/new", get(check_new))
        .route("/projects/{pid}/checks", post(check_create))
        .route("/checks/{id}", get(check_show).post(check_update))
        .route("/checks/{id}/edit", get(check_edit))
        .route("/checks/{id}/pause", post(check_pause))
        .route("/checks/{id}/resume", post(check_resume))
        .route("/checks/{id}/regenerate", post(check_regenerate))
        .route("/checks/{id}/delete", post(check_delete))
```

- [ ] **Step 5: Run — expect PASS**

Run: `cargo nextest run --test auth_web`
Expected: PASS (all check tests). Remove the unused `due_time`/`_check` bindings if clippy complains: `cargo clippy --all-targets -- -D warnings`.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add templates/check_form.html templates/check.html src/web.rs tests/auth_web.rs
git commit -m "feat: check CRUD with schedule validation, pause/resume, uuid regen"
```

---

### Task 13: Web — channels CRUD + bind channels to a check

**Files:**
- Modify: `src/web.rs`
- Create: `templates/channel_form.html`
- Modify: `tests/auth_web.rs`

**Interfaces:**
- Consumes: `Store::{create_channel, find_channel, delete_channel, list_channels_for_project, bind_channel, unbind_channel, bound_channel_ids}`.
- Produces routes: `GET /projects/{pid}/channels/new`, `POST /projects/{pid}/channels`, `POST /channels/{id}/delete`, `POST /checks/{id}/channels` (set the full bound set from submitted checkboxes).
- Plan 2 exposes only the **webhook** channel kind in the UI (Telegram/Slack/ntfy are Plan 3); the form stores `config_json = {"url": "..."}`.

- [ ] **Step 1: Write the failing test** — append to `tests/auth_web.rs`:

```rust
#[tokio::test]
async fn create_channel_and_bind_to_check() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(pid, "job", "cu", pingward::models::ScheduleKind::Period, Some(60), 30, None, "UTC")
        .await.unwrap();

    // create a webhook channel
    let res = server
        .post(&format!("/projects/{pid}/channels"))
        .form(&[("name", "hook"), ("kind", "webhook"), ("url", "http://example.test/h")])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let channels = store.list_channels_for_project(pid).await.unwrap();
    assert_eq!(channels.len(), 1);
    let chid = channels[0].id;
    assert!(channels[0].config_json.contains("example.test"));

    // bind it to the check
    server.post(&format!("/checks/{cid}/channels"))
        .form(&[("channel_ids", chid.to_string().as_str())])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(store.bound_channel_ids(cid).await.unwrap(), vec![chid]);

    // unbind by submitting no channel_ids
    server.post(&format!("/checks/{cid}/channels")).form(&[("_", "")]).await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.bound_channel_ids(cid).await.unwrap().is_empty());
}
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --test auth_web::create_channel_and_bind_to_check`
Expected: FAIL.

- [ ] **Step 3: Template** — `templates/channel_form.html`:

```html
{% extends "base.html" %}
{% block body %}
<h1>New channel</h1>
{% if error %}<p class="status-down">{{ error }}</p>{% endif %}
<form method="post" action="/projects/{{ project_id }}/channels">
  <label>Name <input name="name" required></label>
  <label>Kind
    <select name="kind"><option value="webhook">webhook</option></select></label>
  <label>Webhook URL <input name="url" required></label>
  <button type="submit">Create</button>
</form>
<p><small>Telegram, Slack, and ntfy channels arrive in a later release.</small></p>
{% endblock %}
```

- [ ] **Step 4: Implement handlers** — add to `src/web.rs`:

```rust
use crate::models::ChannelKind;

#[derive(Template)]
#[template(path = "channel_form.html")]
struct ChannelFormTemplate {
    show_nav: bool,
    project_id: i64,
    error: Option<String>,
}

#[derive(Deserialize)]
struct ChannelForm {
    name: String,
    kind: String,
    url: String,
}

#[derive(Deserialize)]
struct BindForm {
    #[serde(default)]
    channel_ids: Vec<i64>,
}

async fn channel_new(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    Ok(render(&ChannelFormTemplate { show_nav: true, project_id: pid, error: None })?.into_response())
}

async fn channel_create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
    Form(form): Form<ChannelForm>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;
    // Plan 2: only webhook is accepted.
    if form.kind != ChannelKind::Webhook.as_str() || form.url.trim().is_empty() {
        return Ok(render(&ChannelFormTemplate {
            show_nav: true,
            project_id: pid,
            error: Some("a webhook URL is required".into()),
        })?
        .into_response());
    }
    let config = serde_json::json!({ "url": form.url.trim() }).to_string();
    state
        .store
        .create_channel(pid, ChannelKind::Webhook, &form.name, &config, Utc::now())
        .await?;
    Ok(Redirect::to(&format!("/projects/{pid}")).into_response())
}

async fn channel_delete(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let channel = state.store.find_channel(id).await?.ok_or(AppError::NotFound)?;
    let project = owned_project(&state.store, channel.project_id, user.id).await?;
    state.store.delete_channel(id).await?;
    Ok(Redirect::to(&format!("/projects/{}", project.id)).into_response())
}

/// Replace a check's bound channel set with exactly the submitted ids (only
/// those that belong to the same project are honored).
async fn check_set_channels(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<BindForm>,
) -> Result<Response, AppError> {
    let check = owned_check(&state.store, id, user.id).await?;
    let valid: std::collections::HashSet<i64> = state
        .store
        .list_channels_for_project(check.project_id)
        .await?
        .into_iter()
        .map(|c| c.id)
        .collect();
    let current: std::collections::HashSet<i64> =
        state.store.bound_channel_ids(id).await?.into_iter().collect();
    let desired: std::collections::HashSet<i64> =
        form.channel_ids.into_iter().filter(|c| valid.contains(c)).collect();

    for add in desired.difference(&current) {
        state.store.bind_channel(id, *add).await?;
    }
    for remove in current.difference(&desired) {
        state.store.unbind_channel(id, *remove).await?;
    }
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}
```

Register in `web::routes()`:
```rust
        .route("/projects/{pid}/channels/new", get(channel_new))
        .route("/projects/{pid}/channels", post(channel_create))
        .route("/channels/{id}/delete", post(channel_delete))
        .route("/checks/{id}/channels", post(check_set_channels))
```

Note on `BindForm`: `axum::Form` uses `serde_html_form` semantics via `serde_urlencoded`; repeated `channel_ids=1&channel_ids=2` deserializes into a `Vec<i64>` only if the extractor supports sequences. `axum::Form` (backed by `serde_urlencoded`) does **not** collect repeated keys into a `Vec`. Use `axum_extra::extract::Form` (feature `form`) which uses `serde_html_form` and supports repeated keys. Add `"form"` to the `axum-extra` features in `Cargo.toml` and import `use axum_extra::extract::Form;` **for this handler** (shadow the `axum::Form` import locally, or alias: `use axum_extra::extract::Form as HtmlForm;` and use `HtmlForm<BindForm>`). Verify with the test: the checkbox form posts repeated `channel_ids`.

- [ ] **Step 5: Adjust `Cargo.toml`** — update the axum-extra line:
```toml
axum-extra = { version = "0.12", default-features = false, features = ["cookie", "form"] }
```

- [ ] **Step 6: Run — expect PASS**

Run: `cargo nextest run --test auth_web`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add Cargo.toml Cargo.lock templates/channel_form.html src/web.rs tests/auth_web.rs
git commit -m "feat: channel CRUD and per-check channel binding"
```

---

### Task 14: Web — settings + user administration (admin only)

**Files:**
- Modify: `src/web.rs`
- Create: `templates/settings.html`, `templates/users.html`
- Modify: `tests/auth_web.rs`

**Interfaces:**
- Consumes: `AdminUser` extractor; `Store::{get_setting, set_setting, list_users, create_user, delete_user, count_users}`; `auth::hash_password`.
- Produces routes: `GET /settings`, `POST /settings` (global `scan_interval`), `GET /users`, `POST /users` (create), `POST /users/{id}/delete`.
- Guard: the last remaining admin cannot be deleted (avoid lockout).

- [ ] **Step 1: Write the failing test** — append to `tests/auth_web.rs`:

```rust
#[tokio::test]
async fn admin_sets_global_scan_interval() {
    let (server, store, _uid) = logged_in_server().await; // admin
    server.get("/settings").await.assert_status_ok();
    server.post("/settings").form(&[("scan_interval", "45")]).await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(store.get_setting("scan_interval").await.unwrap().as_deref(), Some("45"));
}

#[tokio::test]
async fn non_admin_forbidden_from_settings() {
    let (server, store) = server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    store.create_user("plain", Some(&phc), false, chrono::Utc::now()).await.unwrap();
    server.post("/login").form(&[("username", "plain"), ("password", "pw")]).await;
    server.get("/settings").await.assert_status(axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_creates_and_deletes_user() {
    let (server, store, _uid) = logged_in_server().await;
    server.post("/users").form(&[("username", "carol"), ("password", "pw2"), ("is_admin", "")]).await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    let carol = store.find_user_by_username("carol").await.unwrap().unwrap();
    assert!(!carol.is_admin);
    server.post(&format!("/users/{}/delete", carol.id)).await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.find_user_by_username("carol").await.unwrap().is_none());
}
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo nextest run --test auth_web::admin_sets_global_scan_interval`
Expected: FAIL.

- [ ] **Step 3: Templates**

`templates/settings.html`:
```html
{% extends "base.html" %}
{% block body %}
<h1>Settings</h1>
<form method="post" action="/settings">
  <label>Global scan interval (secs) <input name="scan_interval" value="{{ scan_interval }}"></label>
  <button type="submit">Save</button>
</form>
<p><a href="/users">Manage users</a></p>
{% endblock %}
```

`templates/users.html`:
```html
{% extends "base.html" %}
{% block body %}
<h1>Users</h1>
<table><tr><th>Username</th><th>Admin</th><th></th></tr>
{% for u in users %}
<tr><td>{{ u.username }}</td><td>{% if u.is_admin %}yes{% else %}no{% endif %}</td>
  <td><form class="inline" method="post" action="/users/{{ u.id }}/delete"><button>delete</button></form></td></tr>
{% endfor %}
</table>
<h2>Add user</h2>
{% if error %}<p class="status-down">{{ error }}</p>{% endif %}
<form method="post" action="/users">
  <label>Username <input name="username" required></label>
  <label>Password <input name="password" type="password" required></label>
  <label>Admin <input type="checkbox" name="is_admin" value="1"></label>
  <button type="submit">Create user</button>
</form>
{% endblock %}
```

- [ ] **Step 4: Implement handlers** — add to `src/web.rs`:

```rust
use crate::auth::AdminUser;
use crate::models::User;

#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsTemplate {
    show_nav: bool,
    scan_interval: String,
}

#[derive(Template)]
#[template(path = "users.html")]
struct UsersTemplate {
    show_nav: bool,
    users: Vec<User>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct SettingsForm {
    scan_interval: String,
}

#[derive(Deserialize)]
struct NewUserForm {
    username: String,
    password: String,
    #[serde(default)]
    is_admin: Option<String>,
}

async fn settings_page(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Response, AppError> {
    let scan_interval = state.store.get_setting("scan_interval").await?.unwrap_or_default();
    Ok(render(&SettingsTemplate { show_nav: true, scan_interval })?.into_response())
}

async fn settings_save(
    State(state): State<AppState>,
    _admin: AdminUser,
    Form(form): Form<SettingsForm>,
) -> Result<Response, AppError> {
    let trimmed = form.scan_interval.trim();
    // Only persist a positive integer; blank clears to default behavior.
    if trimmed.is_empty() {
        state.store.set_setting("scan_interval", "").await?;
    } else if trimmed.parse::<u64>().map(|v| v > 0).unwrap_or(false) {
        state.store.set_setting("scan_interval", trimmed).await?;
    }
    Ok(Redirect::to("/settings").into_response())
}

async fn users_page(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Response, AppError> {
    let users = state.store.list_users().await?;
    Ok(render(&UsersTemplate { show_nav: true, users, error: None })?.into_response())
}

async fn users_create(
    State(state): State<AppState>,
    _admin: AdminUser,
    Form(form): Form<NewUserForm>,
) -> Result<Response, AppError> {
    if form.username.trim().is_empty() || form.password.is_empty() {
        let users = state.store.list_users().await?;
        return Ok(render(&UsersTemplate {
            show_nav: true,
            users,
            error: Some("username and password are required".into()),
        })?
        .into_response());
    }
    let phc = hash_password(&form.password).map_err(|e| AppError::Other(Box::new(e)))?;
    let is_admin = form.is_admin.is_some();
    state.store.create_user(form.username.trim(), Some(&phc), is_admin, Utc::now()).await?;
    Ok(Redirect::to("/users").into_response())
}

async fn users_delete(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    // Never allow deleting yourself or the last admin (lockout guard).
    if id == admin.id {
        return Ok(Redirect::to("/users").into_response());
    }
    let admins = state.store.list_users().await?.into_iter().filter(|u| u.is_admin).count();
    let target = state.store.find_user_by_id(id).await?;
    if let Some(t) = target {
        if t.is_admin && admins <= 1 {
            return Ok(Redirect::to("/users").into_response());
        }
    }
    state.store.delete_user(id).await?;
    Ok(Redirect::to("/users").into_response())
}
```

Register in `web::routes()`:
```rust
        .route("/settings", get(settings_page).post(settings_save))
        .route("/users", get(users_page).post(users_create))
        .route("/users/{id}/delete", post(users_delete))
```

Note: `settings_save` stores a blank string when cleared; `run_scan_loop` already treats a non-parseable/absent global as "no override" (`.and_then(|v| v.parse::<i64>().ok())`), so a blank value falls through the cascade correctly.

- [ ] **Step 5: Run — expect PASS**

Run: `cargo nextest run --test auth_web`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add templates/settings.html templates/users.html src/web.rs tests/auth_web.rs
git commit -m "feat: admin settings and user management"
```

---

### Task 15: Wire main.rs + full-suite verification + smoke test

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `AppState`, `scheduler::run_scan_loop(store, env_default_secs)`, `pingward::app`.

- [ ] **Step 1: Rewrite `src/main.rs`**

```rust
use pingward::{config::Config, db, scheduler, state::AppState, store::Store};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::from_env();
    let bind = config.bind.clone();
    let scan_interval_secs = config.scan_interval_secs;

    let pool = db::connect(&config.database_url)
        .await
        .expect("failed to connect to database");
    db::migrate(&pool).await.expect("failed to run migrations");
    let store = Store::new(pool);

    // Per-check channel binding replaces Plan 1's single global webhook: the scan
    // loop now resolves each check's bound channels via notify::deliver_event.
    tokio::spawn(scheduler::run_scan_loop(store.clone(), scan_interval_secs));

    let state = AppState::new(store, config);
    let listener = tokio::net::TcpListener::bind(&bind).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        pingward::app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}
```

- [ ] **Step 2: Full suite + clippy + build**

Run:
```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo nextest run
cargo build --release
```
Expected: all green. Fix any unused imports (`due_time` in `web.rs`, dropped `dispatch`/`Arc` in `scheduler.rs`) that clippy flags.

- [ ] **Step 3: Manual smoke test (evidence before claiming done)**

```bash
# terminal 1: a webhook sink
python3 -m http.server 9999
# terminal 2:
PINGWARD_BASE_URL=http://127.0.0.1:8080 PINGWARD_SCAN_INTERVAL=2 \
  DATABASE_URL="sqlite:///tmp/pw2.db?mode=rwc" cargo run
```
In a browser:
1. Visit `http://127.0.0.1:8080/` → redirected to `/setup`; create the first admin.
2. Create a project, then a check (period 60s, grace 5s).
3. Create a webhook channel pointing at `http://127.0.0.1:9999/hook`; bind it to the check.
4. `curl -fsS http://127.0.0.1:8080/ping/<uuid>` → check goes `up`.
5. Wait > 65s; the scan loop transitions it to `down` and the webhook sink logs a POST. Ping again → `up` and a recovery POST fires.

Confirm: setup/login flow, ping URL works, `down` + recovery deliveries land, and the check detail page lists recent pings and notifications.

- [ ] **Step 4: Commit**

```bash
cargo fmt
git add src/main.rs
git commit -m "feat: wire AppState and per-check delivery into main"
```

---

## Self-Review

**Spec coverage (Plan 2 scope):**
- §2 stack (askama, axum-extra cookies, argon2) → Tasks 1, 10 ✅
- §3② authenticated web surface (dashboard, CRUD) → Tasks 10–14 ✅
- §4 remaining tables now used (users, sessions, projects, channels, check_channels, notifications, settings) → Tasks 3, 5, 7, 10–14 ✅
- §6 per-check channel delivery, recovery (`down`→`up`), delivery records, decoupled delivery → Tasks 7, 8 ✅
- §7 auth: local password sessions + forward-auth gated by trusted proxies → Tasks 1, 9, 10 ✅
- §8 config cascade (check→project→global→env) → Tasks 6, 8, 14 ✅
- §9 error handling: ownership 404s, validation re-renders, delivery failures recorded not propagated → Tasks 7, 11–13 ✅
- §11 webhook channel wired through the UI (Telegram/Slack/ntfy explicitly deferred to Plan 3) → Tasks 7, 13 ✅
- §10 testing: unit (cascade, forward-auth trust, password hash) + integration (`auth_web.rs`, updated `scheduler.rs`) + notifier mock (wiremock) → Tasks 1, 6, 7, 9, 10–14 ✅

**Deferred to Plan 3 (explicit, not gaps):**
- Telegram, Slack, ntfy notifiers (`notifier_for` returns `None` for them today).
- PostgreSQL parity (schema is already portable TEXT/INTEGER; a `migrations/postgres` tree + `AnyPool`/feature split lands in Plan 3).
- Nag re-notifications, "started but never finished" detection, retention/pruning, API tokens (spec §12 future work).

**Signature changes made here (each updated in-task with all callers):**
- `app(store)` → `app(state: AppState)` (Task 4); `ping::routes() -> Router<AppState>` (Task 4).
- `NotificationEvent` gains `check_id` (Task 7); `run_scan_loop(store, interval, notifiers)` → `run_scan_loop(store, env_default_secs)` (Task 8).

**Placeholder scan:** none — every step carries concrete code/commands. Two implementation risks are flagged inline with fallbacks: `Option<CurrentUser>` optional-extractor support (Task 10) and `axum::Form` vs `axum_extra::extract::Form` for repeated `channel_ids` (Task 13).

**Type consistency:** `AppState`/`Store` via `FromRef`; `Check`/`Project`/`Channel`/`User`/`Notification`/`Ping` fields; enum `as_str`/`FromStr`; `EventKind`/`NotifyStatus`; `RetryPolicy`; and the `render`/`owned_project`/`owned_check` helpers are used consistently across Tasks 2–15.
</content>
</invoke>
