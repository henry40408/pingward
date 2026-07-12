# PostgreSQL Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make pingward run on **both SQLite and PostgreSQL** from one codebase (spec §2), selected at runtime by the `DATABASE_URL` scheme, with no behaviour change on SQLite.

**Architecture:** Switch the sqlx layer from the concrete `Sqlite` driver to the runtime-agnostic **`sqlx::Any`** driver. One code path serves both backends by using **`$1`-numbered placeholders** (SQLite accepts them natively; the `Any` driver does NOT translate `?` for Postgres — verified by live spike) and **`RETURNING id`** instead of SQLite-only `last_insert_rowid()`. Schema DDL differs per backend, so migrations are split into `migrations/sqlite/` (existing) and `migrations/postgres/` (new), chosen by URL scheme. Timestamps stay TEXT (RFC3339) and integer/boolean columns are BIGINT so `row.get::<i64,_>` is uniform across backends.

**Tech Stack:** Rust + tokio, sqlx 0.9 `Any` driver (enable existing crate's `any` + `postgres` features — no new dependency), the `container` CLI for a local Postgres test instance, GitHub Actions `postgres` service for CI.

## Global Constraints

- **Verified sqlx 0.9 facts (live spike 2026-07-12) — do not deviate:**
  - The `Any` driver does **not** translate placeholders: `?` is a **syntax error on Postgres**; `$1, $2, …` numbered placeholders work on **both** SQLite and Postgres. Use `$N` everywhere.
  - `RETURNING id` works on both backends. Replace every `last_insert_rowid()` with an `INSERT … RETURNING id` that is read via `fetch_one` + `row.get::<i64,_>("id")`.
  - sqlx 0.9 `sqlx::query()` requires `SqlSafeStr` (only `&'static str`). Every query string in `store.rs` is already a string literal — keep them literal. If any query must be built dynamically, wrap it in `sqlx::AssertSqlSafe(...)` (and audit it for injection first).
  - Call `sqlx::any::install_default_drivers()` once before the first connection.
- No behaviour change on SQLite: the full existing suite (`cargo nextest run`, currently 82 tests) MUST stay green and unmodified in intent — the default `DATABASE_URL` stays `sqlite://pingward.sqlite3?mode=rwc`.
- `sqlx` version stays `0.9`; only its feature list changes (add `any`, `postgres`). No third-party dependency published < 7 days ago; this plan adds **no** new crate.
- All SQL lives in `src/store.rs` (queries) and the `migrations/**` files only. `src/db.rs` owns connection + migration wiring.
- Tests run with `cargo nextest run`. `cargo fmt` before commit. `cargo clippy --all-targets -- -D warnings` must stay clean (CI-enforced).
- Commits GPG-signed; stage files explicitly by name.
- Integer/boolean columns are `BIGINT` in Postgres (so `row.get::<i64,_>` matches SQLite's INTEGER); timestamps are `TEXT` (RFC3339, parsed by the existing `parse_ts`). Do NOT use Postgres `TIMESTAMPTZ` or `int4`.

---

## File Structure

- `Cargo.toml` (MODIFY): add `any`, `postgres` to the sqlx feature list.
- `src/db.rs` (MODIFY): `Pool = sqlx::AnyPool`; `connect()` installs the Any drivers, caps in-memory SQLite to one connection, and enables the SQLite `foreign_keys` pragma per-connection via an `after_connect` hook (Postgres enforces FKs natively); `migrate(pool, url)` picks the migration directory by URL scheme.
- `src/store.rs` (MODIFY): all six `row_to_*` mappers take `&sqlx::any::AnyRow`; every query uses `$N` placeholders; the four inserts that returned `last_insert_rowid()` use `RETURNING id`.
- `migrations/postgres/0001_init.sql`, `migrations/postgres/0002_indexes.sql` (CREATE): the SQLite schema translated to Postgres.
- `tests/pg_store.rs` (CREATE): a Postgres integration test gated on `TEST_DATABASE_URL` (skips when unset).
- `docker-compose.yaml` (COMMIT existing untracked file): the Postgres service definition for local use.
- `.github/workflows/ci.yml` (MODIFY): add a `postgres` service and run the gated test with `TEST_DATABASE_URL`.

---

### Task 1: Convert the DB layer to the sqlx `Any` driver (both backends), SQLite suite stays green

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/db.rs`
- Modify: `src/store.rs`
- Create: `migrations/postgres/0001_init.sql`, `migrations/postgres/0002_indexes.sql`
- Create: `tests/pg_store.rs`

**Interfaces:**
- Consumes: the existing `models` types; `parse_ts`, `decode_err` (unchanged) in `store.rs`.
- Produces: `pub type Pool = sqlx::AnyPool`; `db::connect(url) -> Result<Pool, sqlx::Error>`; `db::migrate(pool: &Pool, url: &str) -> Result<(), sqlx::Error>`; a `Store` whose every method now runs backend-agnostic SQL.

This task is large but atomic: the `Pool` type change ripples through `store.rs`, so the whole layer must convert together to compile. Work in this order and lean on the compiler + the SQLite suite after each sub-step.

- [ ] **Step 1: Enable the sqlx `any` + `postgres` features**

In `Cargo.toml`, change the sqlx line to:

```toml
sqlx = { version = "0.9", default-features = false, features = ["runtime-tokio", "sqlite", "postgres", "any", "tls-rustls-aws-lc-rs", "migrate"] }
```

Run `cargo build` to fetch the features. Expected: builds (existing code still uses `SqlitePool` at this point).

- [ ] **Step 2: Rewrite `src/db.rs` for the `Any` driver**

Replace the top of `src/db.rs` (the imports, `Pool` alias, and `connect`/`migrate`) with the following. Keep `is_in_memory_url` and the `#[cfg(test)]` module (the tests call `connect("sqlite::memory:")` / `migrate(&pool, ...)` — update the test's `migrate` calls to pass the url in Step 6).

```rust
use sqlx::any::{install_default_drivers, AnyConnectOptions, AnyPoolOptions};
use sqlx::migrate::Migrator;
use sqlx::ConnectOptions;
use std::path::Path;
use std::str::FromStr;

pub type Pool = sqlx::AnyPool;

/// SQLite's `:memory:` database is scoped to a single physical connection.
fn is_in_memory_url(url: &str) -> bool {
    url.contains(":memory:") || url.contains("mode=memory")
}

fn is_sqlite_url(url: &str) -> bool {
    url.starts_with("sqlite:")
}

pub async fn connect(url: &str) -> Result<Pool, sqlx::Error> {
    // The `Any` driver dispatches to whichever concrete driver a URL names;
    // its default drivers must be registered once before connecting.
    install_default_drivers();

    let sqlite = is_sqlite_url(url);
    // Cap in-memory SQLite to one connection so all operations share the one
    // in-memory database. Postgres and file SQLite use a small pool.
    let max_connections = if sqlite && is_in_memory_url(url) { 1 } else { 5 };

    let opts = AnyConnectOptions::from_str(url)?;

    AnyPoolOptions::new()
        .max_connections(max_connections)
        .after_connect(move |conn, _meta| {
            Box::pin(async move {
                // `foreign_keys` is a per-connection SQLite pragma. Under the
                // `Any` driver we cannot set `SqliteConnectOptions.foreign_keys`,
                // so enable it on every new SQLite connection here (otherwise
                // `ON DELETE CASCADE` is silently unenforced). Postgres enforces
                // foreign keys natively and needs no pragma.
                if sqlite {
                    sqlx::query("PRAGMA foreign_keys = ON").execute(conn).await?;
                }
                Ok(())
            })
        })
        .connect_with(opts)
        .await
}

pub async fn migrate(pool: &Pool, url: &str) -> Result<(), sqlx::Error> {
    let dir = if is_sqlite_url(url) {
        "migrations/sqlite"
    } else {
        "migrations/postgres"
    };
    let m = Migrator::new(Path::new(dir)).await?;
    m.run(pool).await?;
    Ok(())
}
```

Notes for the implementer:
- `create_if_missing` for SQLite is now carried by the URL parameter `?mode=rwc` (the default `DATABASE_URL` already includes it), so it is not set programmatically. `sqlite::memory:` needs no create flag.
- Verify the exact `after_connect` closure signature against sqlx 0.9 (`AnyPoolOptions::after_connect`) — the callback receives `&mut AnyConnection` and `PoolConnectionMetadata` and returns a boxed future of `Result<(), sqlx::Error>`. Adjust the closure to match if the compiler disagrees; the intent (run the pragma on SQLite connections only) must hold.
- If `Migrator::run` does not accept an `AnyPool` in sqlx 0.9 (verify early — it should, `migrate` + `any` are both enabled), fall back to reading the `.sql` files in the chosen dir and executing each statement; keep the `migrations/**` files as the source of truth. Report which path you used.

- [ ] **Step 3: Create the Postgres migrations**

Create `migrations/postgres/0001_init.sql` — the SQLite schema with `INTEGER PRIMARY KEY AUTOINCREMENT` → `BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY`, every other `INTEGER` → `BIGINT`, `TEXT` unchanged, CHECK/REFERENCES/composite-PK unchanged:

```sql
CREATE TABLE users (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT,
  is_admin BIGINT NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);

CREATE TABLE projects (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  scan_interval_secs BIGINT,
  created_at TEXT NOT NULL
);

CREATE TABLE checks (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  ping_uuid TEXT NOT NULL UNIQUE,
  schedule_kind TEXT NOT NULL CHECK (schedule_kind IN ('period','cron')),
  period_secs BIGINT,
  grace_secs BIGINT NOT NULL DEFAULT 300,
  cron_expr TEXT,
  timezone TEXT NOT NULL DEFAULT 'UTC',
  status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new','up','down','paused')),
  last_ping_at TEXT,
  last_start_at TEXT,
  next_due_at TEXT,
  scan_interval_secs BIGINT,
  created_at TEXT NOT NULL
);
CREATE INDEX idx_checks_status ON checks(status);

CREATE TABLE channels (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  config_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE check_channels (
  check_id BIGINT NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  channel_id BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  PRIMARY KEY (check_id, channel_id)
);

CREATE TABLE pings (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  check_id BIGINT NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('success','fail','start','log','exitcode')),
  exit_code BIGINT,
  body TEXT NOT NULL DEFAULT '',
  source_ip TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX idx_pings_check ON pings(check_id, created_at);

CREATE TABLE notifications (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  check_id BIGINT NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  channel_id BIGINT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  event TEXT NOT NULL CHECK (event IN ('down','up')),
  status TEXT NOT NULL CHECK (status IN ('ok','error')),
  error TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at TEXT NOT NULL
);

CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
```

Create `migrations/postgres/0002_indexes.sql` (identical to the SQLite one — index DDL is portable):

```sql
CREATE INDEX idx_channels_project ON channels(project_id);
CREATE INDEX idx_check_channels_channel ON check_channels(channel_id);
CREATE INDEX idx_notifications_check ON notifications(check_id, created_at);
CREATE INDEX idx_sessions_user ON sessions(user_id);
```

- [ ] **Step 4: Convert every `row_to_*` mapper to `AnyRow`**

In `src/store.rs`, change all six mapper signatures from `&sqlx::sqlite::SqliteRow` to `&sqlx::any::AnyRow`. The bodies are unchanged — they already use the generic `sqlx::Row::get`, `parse_ts`, and `decode_err`. Example (apply the same signature change to `row_to_check`, `row_to_user`, `row_to_project`, `row_to_channel`, `row_to_ping`, `row_to_notification`):

```rust
fn row_to_user(row: &sqlx::any::AnyRow) -> Result<User, sqlx::Error> {
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

If any `.get("col")` fails to infer its type under `AnyRow` (the concrete `SqliteRow` may have allowed inference the `AnyRow` decode does not), annotate it explicitly — `row.get::<Option<String>, _>("col")` for nullable TEXT, `row.get::<i64, _>("col")` for integer columns, `row.get::<String, _>("col")` for non-null TEXT. `id` columns are `i64`.

- [ ] **Step 5: Convert every query to `$N` placeholders and `RETURNING id`**

Within each SQL string literal in `src/store.rs`, replace each `?` **placeholder** with a positional `$1, $2, …` restarting at `$1` for every query, in bind order. (Rust `?` try-operators are not in strings — leave them. `.bind(...)` calls and their order are unchanged.)

For the four inserts that currently end with `Ok(res.last_insert_rowid())` (create_user, create_project, create_check, create_channel — the ones at the four `last_insert_rowid()` sites), switch to `RETURNING id`. Pattern — before:

```rust
let res = sqlx::query("INSERT INTO users (username, password_hash, is_admin, created_at) VALUES (?, ?, ?, ?)")
    .bind(username).bind(password_hash).bind(is_admin as i64).bind(now.to_rfc3339())
    .execute(&self.pool).await?;
Ok(res.last_insert_rowid())
```

after:

```rust
let row = sqlx::query("INSERT INTO users (username, password_hash, is_admin, created_at) VALUES ($1, $2, $3, $4) RETURNING id")
    .bind(username).bind(password_hash).bind(is_admin as i64).bind(now.to_rfc3339())
    .fetch_one(&self.pool).await?;
Ok(row.get::<i64, _>("id"))
```

(Preserve each insert's actual columns/binds; only the placeholder style, the trailing `RETURNING id`, `execute`→`fetch_one`, and the return expression change. Ensure `use sqlx::Row;` is in scope — it already is.)

- [ ] **Step 6: Update `db::migrate` call sites**

`migrate` now takes `(pool, url)`. Update every call site: `src/main.rs` (`db::migrate(&pool, &config.database_url)`), the `#[cfg(test)]` block in `src/db.rs`, and every test that calls `db::migrate(&pool)` (in `src/store.rs`, `src/notify.rs`, `src/scheduler.rs`, and `tests/*.rs`) to pass the same URL used for `connect` (e.g. `db::migrate(&pool, "sqlite::memory:")`). Let the compiler enumerate the call sites.

- [ ] **Step 7: Run the SQLite suite (no behaviour change)**

Run: `cargo nextest run`
Expected: PASS — the full existing suite (82 tests) green, unmodified in intent. Then `cargo clippy --all-targets -- -D warnings` clean and `cargo fmt`.

Note: because SQLite accepts BOTH `?` and `$N`, a *missed* `?`→`$N` conversion will NOT fail here — only Postgres catches it. Step 9 is the real completeness gate.

- [ ] **Step 8: Write the Postgres-gated integration test**

Create `tests/pg_store.rs`. It runs only when `TEST_DATABASE_URL` is set (skips with a printed note otherwise), so `cargo nextest run` stays green with no Postgres. It must reset the schema, migrate, then exercise a representative slice of `Store` — the same operations the SQLite tests cover — so a missed placeholder conversion in ANY exercised query fails loudly on Postgres.

```rust
use pingward::{db, models::{ChannelKind, ScheduleKind}, store::Store};

fn pg_url() -> Option<String> {
    std::env::var("TEST_DATABASE_URL").ok().filter(|u| u.starts_with("postgres"))
}

async fn fresh_pg_store(url: &str) -> Store {
    let pool = db::connect(url).await.expect("connect postgres");
    // Reset to a clean schema so migrations apply idempotently across runs.
    sqlx::query("DROP SCHEMA public CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("CREATE SCHEMA public")
        .execute(&pool)
        .await
        .expect("recreate schema");
    db::migrate(&pool, url).await.expect("migrate postgres");
    Store::new(pool)
}

#[tokio::test]
async fn postgres_full_round_trip() {
    let Some(url) = pg_url() else {
        eprintln!("TEST_DATABASE_URL unset — skipping postgres_full_round_trip");
        return;
    };
    let store = fresh_pg_store(&url).await;
    let now = chrono::Utc::now();

    // users
    let uid = store.create_user("alice", Some("phc"), true, now).await.unwrap();
    assert!(store.find_user_by_username("alice").await.unwrap().unwrap().is_admin);

    // projects
    let pid = store.create_project(uid, "web", Some(45), now).await.unwrap();
    assert_eq!(store.list_projects_for_user(uid).await.unwrap().len(), 1);

    // checks
    let cid = store
        .create_check(pid, "job", "uuid-1", ScheduleKind::Period, Some(60), 30, None, "UTC")
        .await
        .unwrap();
    assert!(store.find_check(cid).await.unwrap().is_some());
    assert_eq!(store.list_checks_for_project(pid).await.unwrap().len(), 1);
    assert_eq!(store.list_active_checks().await.unwrap().len(), 1);

    // channels + binding
    let chid = store
        .create_channel(pid, ChannelKind::Webhook, "hook", "{\"url\":\"http://x\"}", now)
        .await
        .unwrap();
    store.bind_channel(cid, chid).await.unwrap();
    assert_eq!(store.bound_channel_ids(cid).await.unwrap(), vec![chid]);
    assert_eq!(store.channels_for_check(cid).await.unwrap().len(), 1);

    // pings + status transition
    store.insert_ping(cid, pingward::models::PingKind::Success, None, "ok", Some("127.0.0.1"), now).await.unwrap();
    store.mark_ping(cid, pingward::models::CheckStatus::Up, Some(now), None, None).await.unwrap();

    // notifications
    store
        .record_notification(cid, chid, pingward::notify::EventKind::Down, pingward::models::NotifyStatus::Ok, None, now)
        .await
        .unwrap();
    assert_eq!(store.list_recent_notifications(cid, 10).await.unwrap().len(), 1);

    // settings
    store.set_setting("scan_interval", "45").await.unwrap();
    assert_eq!(store.get_setting("scan_interval").await.unwrap().as_deref(), Some("45"));

    // cascade delete: removing the user removes project → checks → channels → pings
    store.delete_user(uid).await.unwrap();
    assert!(store.list_projects_for_user(uid).await.unwrap().is_empty());
    assert!(store.find_check(cid).await.unwrap().is_none());
}
```

Adjust the exact `Store` method names/signatures to match `src/store.rs` (the implementer must read the real signatures — e.g. `set_setting`, `mark_ping`, `insert_ping`, `list_active_checks` — and use them verbatim; do NOT invent methods). The goal is to touch every query family (create/list/find/update/delete + settings + notifications) so residual `?` placeholders surface.

- [ ] **Step 9: Run the Postgres test against the live instance**

A Postgres 17 instance is available via the `container` CLI (see the dispatch for the current IP). Run:

Run: `TEST_DATABASE_URL="postgres://postgres:postgres@<PG_IP>:5432/postgres" cargo nextest run --test pg_store`
Expected: PASS — `postgres_full_round_trip` green. If any query fails with a Postgres syntax error, a `?` placeholder was missed in that query — fix it and re-run. This is the completeness gate for Step 5.

Also re-run the full SQLite suite once more (`cargo nextest run`, no env) to confirm 82/82 still green.

- [ ] **Step 10: fmt + clippy + commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add Cargo.toml Cargo.lock src/db.rs src/store.rs src/main.rs migrations/postgres/0001_init.sql migrations/postgres/0002_indexes.sql tests/pg_store.rs
# plus any test files whose db::migrate call sites you updated:
git add src/notify.rs src/scheduler.rs tests/auth_web.rs tests/ping_api.rs tests/scheduler.rs
git commit -m "feat: run on both SQLite and PostgreSQL via sqlx Any driver"
```

(Stage only the files you actually changed — the `db::migrate` call-site updates determine which test files are included. Never `git add -A`.)

---

### Task 2: CI Postgres service + commit the compose file

**Files:**
- Modify: `.github/workflows/ci.yml`
- Commit: `docker-compose.yaml` (existing untracked file)

**Interfaces:**
- Consumes: the `tests/pg_store.rs` gate (`TEST_DATABASE_URL`) from Task 1.
- Produces: CI that runs the Postgres round-trip against a real Postgres service on every push/PR.

- [ ] **Step 1: Add a Postgres service + env to the test job**

In `.github/workflows/ci.yml`, add a `services` block to the `test` job and set `TEST_DATABASE_URL` on the test step. The GitHub-hosted `postgres` service is reachable at `localhost:5432` on the runner:

```yaml
jobs:
  test:
    name: fmt + clippy + test
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:17-alpine
        env:
          POSTGRES_PASSWORD: postgres
        ports:
          - 5432:5432
        options: >-
          --health-cmd "pg_isready -U postgres"
          --health-interval 2s
          --health-timeout 3s
          --health-retries 15
    steps:
      # ... existing checkout / toolchain / cache / nextest / fmt / clippy steps unchanged ...

      - name: Test
        env:
          TEST_DATABASE_URL: postgres://postgres:postgres@localhost:5432/postgres
        run: cargo nextest run
```

Keep every existing step (checkout, toolchain with rustfmt+clippy, cargo cache, nextest install, fmt check, clippy) exactly as-is; only add the `services` block and the `env` on the Test step. With `TEST_DATABASE_URL` set, `cargo nextest run` now also runs `postgres_full_round_trip` against the service; locally (no env) it still skips.

- [ ] **Step 2: Commit the compose file for local use**

`docker-compose.yaml` (Postgres 17-alpine on 5432) already exists untracked in the repo root. Commit it so contributors have a one-command local Postgres:

```bash
git add docker-compose.yaml .github/workflows/ci.yml
git commit -m "ci: run tests against a real PostgreSQL service"
```

- [ ] **Step 3: Verify the workflow is well-formed**

The workflow cannot run locally, but confirm it parses and the indentation is valid YAML (e.g. `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"`). Expected: no error.

---

## Self-Review

**1. Spec coverage (spec §2 "both SQLite and PostgreSQL support, one codebase, DB-agnostic SQL — TEXT storage for portability"):**
- One codebase / one code path — the `Any` driver + `$N` placeholders + `RETURNING id` mean `store.rs` has a single implementation for both backends. ✅ (Task 1)
- Both backends selected at runtime by `DATABASE_URL` scheme — `connect`/`migrate` branch on `sqlite:` vs otherwise. ✅ (Task 1 db.rs)
- TEXT storage for portability — timestamps stay TEXT (RFC3339); integer/bool columns are BIGINT so decoding is uniform. ✅ (Task 1 migrations)
- Verified against a real Postgres locally (Task 1 Step 9) and in CI (Task 2). ✅

**2. Placeholder scan:** every code step contains complete code (db.rs, both migrations, the mapper change, the RETURNING pattern, the full gated test, the CI YAML). The one deliberately rule-based step is the `?`→`$N` conversion across ~43 queries (Step 5) — reproducing all 43 verbatim would duplicate `store.rs`; instead the rule is exact and the Postgres test (Step 9) is the completeness gate that fails on any miss. The gated test's method calls are marked "match the real signatures in store.rs" because those signatures live in code the implementer edits in the same task.

**3. Type consistency:**
- `Pool = sqlx::AnyPool` produced in db.rs, consumed by `Store { pool: Pool }` and every `&self.pool` call. ✅
- `migrate(pool, url)` — the new two-arg signature is propagated to all call sites in Step 6. ✅
- `row_to_*(&sqlx::any::AnyRow)` — all six mappers change together; their callers pass `AnyRow` from `fetch_*` on an `AnyPool`. ✅
- `RETURNING id` read as `row.get::<i64,_>("id")` matches the `i64` return type of the four `create_*` methods. ✅
- Postgres BIGINT columns ↔ `row.get::<i64,_>` ↔ SQLite INTEGER — uniform. ✅
- `TEST_DATABASE_URL` gate string (`postgres` prefix) is the same in `tests/pg_store.rs` and the CI env. ✅
