# pingward retention / pruning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Background age-based pruning of `pings` and `notifications` history, configured globally, so the database and check-detail page do not grow without bound.

**Architecture:** A dedicated periodic tokio task (`run_prune_loop`) calls `prune_once` every `PINGWARD_PRUNE_INTERVAL_SECS` (default 3600). `prune_once` reads two global `settings` retention values (`pings_retention_days`, `notifications_retention_days`) and deletes rows whose `created_at` is older than N days via a TEXT comparison. No schema change: retention lives in the existing `settings` table; timestamps use existing `created_at` columns.

**Tech Stack:** Rust, tokio, sqlx 0.9 `Any` driver (SQLite + PostgreSQL), axum 0.8, askama, chrono.

## Global Constraints

- No migration / no schema change. Retention values are `settings` key/value rows; pruning uses existing `created_at` (TEXT RFC3339, UTC).
- Timestamp comparison is lexicographic TEXT (`created_at < $cutoff`), valid on both backends because all writes use `DateTime<Utc>.to_rfc3339()` (mirrors `find_session_user`'s `expires_at > $2`).
- SQL placeholders numbered `$N` (the `Any` driver does not translate `?`).
- Retention unset / blank / non-positive = OFF (delete nothing) for that table.
- `prune_once` takes an injected `now: DateTime<Utc>` for determinism (mirrors `scan_once`/`nag_once`).
- Rust: `cargo fmt` before commit; tests via `cargo nextest run`; clippy `-D warnings`. MSRV/`rust-version`, `version`, `CHANGELOG.md` MUST NOT change.
- Commits GPG-signed; stage files explicitly by name.
- Design spec: `docs/superpowers/specs/2026-07-13-pingward-retention-pruning-design.md`.

## File Structure

- `src/store.rs` — `delete_pings_before`, `delete_notifications_before`. **Modify.**
- `src/prune.rs` — `prune_once`, `run_prune_loop`, `parse_days` helper. **Create.**
- `src/config.rs` — `Config.prune_interval_secs` from `PINGWARD_PRUNE_INTERVAL_SECS`. **Modify.**
- `src/lib.rs` — `pub mod prune;`. **Modify.**
- `src/main.rs` — spawn `run_prune_loop`. **Modify.**
- `src/web.rs`, `templates/settings.html` — two retention fields on the settings page. **Modify.**
- `tests/auth_web.rs` — settings retention persistence. **Modify.**
- `tests/pg_store.rs` — prune round-trip on live Postgres. **Modify.**

---

### Task 1: Store delete methods

**Files:**
- Modify: `src/store.rs` (add two methods near `list_recent_notifications` / the pings-notifications section)
- Test: `src/store.rs` tests module

**Interfaces:**
- Produces:
  - `delete_pings_before(&self, cutoff: &str) -> Result<u64, sqlx::Error>`
  - `delete_notifications_before(&self, cutoff: &str) -> Result<u64, sqlx::Error>`

- [ ] **Step 1: Add the two delete methods**

In `src/store.rs`, in the `// --- pings / notifications ---` section, add:

```rust
    /// Delete pings older than `cutoff` (an RFC3339 timestamp). Returns the
    /// number of rows removed. `created_at` is TEXT RFC3339 (UTC), so the
    /// lexicographic `<` comparison is chronological on both backends.
    pub async fn delete_pings_before(&self, cutoff: &str) -> Result<u64, sqlx::Error> {
        let r = sqlx::query("DELETE FROM pings WHERE created_at < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    /// Delete notifications older than `cutoff` (an RFC3339 timestamp). Returns
    /// the number of rows removed.
    pub async fn delete_notifications_before(&self, cutoff: &str) -> Result<u64, sqlx::Error> {
        let r = sqlx::query("DELETE FROM notifications WHERE created_at < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }
```

- [ ] **Step 2: Write a test**

In `src/store.rs` tests module, add a test that inserts old + recent rows and prunes. Build the migrated in-memory store the same way the other store tests do (`seeded()` helper creates user id 1 / project id 1; then `create_check` for a check id; `insert_ping(check_id, kind, exit_code, body, source_ip, now)` inserts a ping at a chosen timestamp; `record_notification(check_id, channel_id, event, status, error, now)` inserts a notification — create a channel via `create_channel` first for a valid `channel_id`).

```rust
    #[tokio::test]
    async fn delete_before_removes_only_old_rows() {
        use chrono::Duration;
        let store = seeded().await;
        let cid = store
            .create_check(1, "c", "uu", ScheduleKind::Period, Some(60), 30, None, "UTC")
            .await
            .unwrap();
        let chan = store
            .create_channel(1, ChannelKind::Webhook, "h", "{\"url\":\"http://x\"}", Utc::now())
            .await
            .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
        let old = now - Duration::days(10);
        let recent = now - Duration::days(1);

        // two pings: one old, one recent
        store.insert_ping(cid, PingKind::Success, None, "", None, old).await.unwrap();
        store.insert_ping(cid, PingKind::Success, None, "", None, recent).await.unwrap();
        // two notifications: one old, one recent
        store.record_notification(cid, chan, EventKind::Down, NotifyStatus::Ok, None, old).await.unwrap();
        store.record_notification(cid, chan, EventKind::Up, NotifyStatus::Ok, None, recent).await.unwrap();

        // cutoff = 7 days before now → deletes the 10-day-old rows, keeps the 1-day-old
        let cutoff = (now - Duration::days(7)).to_rfc3339();
        assert_eq!(store.delete_pings_before(&cutoff).await.unwrap(), 1);
        assert_eq!(store.delete_notifications_before(&cutoff).await.unwrap(), 1);
        assert_eq!(store.list_recent_pings(cid, 10).await.unwrap().len(), 1);
        assert_eq!(store.list_recent_notifications(cid, 10).await.unwrap().len(), 1);

        // a far-past cutoff deletes nothing more
        let far = (now - Duration::days(365)).to_rfc3339();
        assert_eq!(store.delete_pings_before(&far).await.unwrap(), 0);
    }
```

Ensure the test module imports whatever it needs (`chrono::TimeZone` for `Utc.with_ymd_and_hms`, and `ChannelKind`, `EventKind`, `NotifyStatus`, `PingKind`, `ScheduleKind` — most are already imported by sibling tests; add any missing).

- [ ] **Step 3: Run the tests**

Run: `cargo nextest run --lib store`
Expected: PASS, including `delete_before_removes_only_old_rows`.

- [ ] **Step 4: Commit**

```bash
git add src/store.rs
git commit -m "feat: store delete-before methods for pings and notifications"
```

---

### Task 2: prune module, config interval, and wiring

**Files:**
- Create: `src/prune.rs`
- Modify: `src/lib.rs` (add `pub mod prune;`)
- Modify: `src/config.rs` (`prune_interval_secs`)
- Modify: `src/main.rs` (spawn the loop)
- Test: `src/prune.rs` tests, `src/config.rs` tests

**Interfaces:**
- Consumes: `Store::{get_setting, delete_pings_before, delete_notifications_before}` (Task 1).
- Produces: `prune::prune_once(&Store, DateTime<Utc>) -> Result<(u64, u64), sqlx::Error>`; `prune::run_prune_loop(Store, u64)`; `Config.prune_interval_secs: u64`.

- [ ] **Step 1: Add `prune_interval_secs` to Config**

In `src/config.rs`, add the field to `struct Config` (after `scan_interval_secs`):

```rust
    pub prune_interval_secs: u64,
```

In `from_map`, parse it (near the `scan_interval_secs` parse):

```rust
        let prune_interval_secs = get("PINGWARD_PRUNE_INTERVAL_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600);
```

and set it in the returned `Config { … }` literal:

```rust
            prune_interval_secs,
```

- [ ] **Step 2: Add a config test**

In `src/config.rs` tests, extend `defaults_apply_when_unset` and `env_overrides_defaults` (or add a focused test):

```rust
    #[test]
    fn prune_interval_defaults_and_overrides() {
        assert_eq!(Config::from_map(|_| None).prune_interval_secs, 3600);
        let c = Config::from_map(|k| (k == "PINGWARD_PRUNE_INTERVAL_SECS").then(|| "60".into()));
        assert_eq!(c.prune_interval_secs, 60);
    }
```

- [ ] **Step 3: Create `src/prune.rs`**

```rust
use crate::store::Store;
use chrono::{DateTime, Duration, Utc};
use tokio::time::{sleep, Duration as TokioDuration};

/// Parse a `settings` retention value into a positive day count, or `None`
/// (retention off) when unset, blank, non-numeric, or non-positive.
fn parse_days(v: Option<String>) -> Option<i64> {
    v.and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|&n| n > 0)
}

/// Delete `pings` and `notifications` older than their configured retention.
/// Each table's retention is an independent global setting; a table with
/// retention off is skipped (its count is 0). Returns
/// `(pings_deleted, notifications_deleted)`. `now` is injected for determinism.
pub async fn prune_once(
    store: &Store,
    now: DateTime<Utc>,
) -> Result<(u64, u64), sqlx::Error> {
    let pings_deleted = match parse_days(store.get_setting("pings_retention_days").await?) {
        Some(days) => {
            let cutoff = (now - Duration::days(days)).to_rfc3339();
            store.delete_pings_before(&cutoff).await?
        }
        None => 0,
    };
    let notifications_deleted =
        match parse_days(store.get_setting("notifications_retention_days").await?) {
            Some(days) => {
                let cutoff = (now - Duration::days(days)).to_rfc3339();
                store.delete_notifications_before(&cutoff).await?
            }
            None => 0,
        };
    Ok((pings_deleted, notifications_deleted))
}

/// Run the prune task forever: prune once immediately, then every
/// `interval_secs` (bounded to >= 1s). Errors are logged, never fatal.
pub async fn run_prune_loop(store: Store, interval_secs: u64) {
    let interval = TokioDuration::from_secs(interval_secs.max(1));
    loop {
        match prune_once(&store, Utc::now()).await {
            Ok((p, n)) => {
                if p > 0 || n > 0 {
                    tracing::info!("pruned {p} pings, {n} notifications");
                }
            }
            Err(e) => tracing::error!("prune_once failed: {e}"),
        }
        sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::models::{ChannelKind, EventKind, NotifyStatus, PingKind, ScheduleKind};
    use chrono::TimeZone;

    async fn store_with_check_and_channel() -> (Store, i64, i64) {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool, "sqlite::memory:").await.unwrap();
        let store = Store::new(pool);
        store.create_user("u", None, false, Utc::now()).await.unwrap();
        store.create_project(1, "p", None, None, Utc::now()).await.unwrap();
        let cid = store
            .create_check(1, "c", "uu", ScheduleKind::Period, Some(60), 30, None, "UTC")
            .await
            .unwrap();
        let chan = store
            .create_channel(1, ChannelKind::Webhook, "h", "{\"url\":\"http://x\"}", Utc::now())
            .await
            .unwrap();
        (store, cid, chan)
    }

    #[tokio::test]
    async fn prune_once_deletes_old_when_retention_set() {
        let (store, cid, chan) = store_with_check_and_channel().await;
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
        let old = now - Duration::days(10);
        let recent = now - Duration::days(1);
        store.insert_ping(cid, PingKind::Success, None, "", None, old).await.unwrap();
        store.insert_ping(cid, PingKind::Success, None, "", None, recent).await.unwrap();
        store.record_notification(cid, chan, EventKind::Down, NotifyStatus::Ok, None, old).await.unwrap();

        store.set_setting("pings_retention_days", "7").await.unwrap();
        store.set_setting("notifications_retention_days", "7").await.unwrap();

        let (p, n) = prune_once(&store, now).await.unwrap();
        assert_eq!((p, n), (1, 1));
        assert_eq!(store.list_recent_pings(cid, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn prune_once_off_when_unset_or_zero() {
        let (store, cid, _chan) = store_with_check_and_channel().await;
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
        store
            .insert_ping(cid, PingKind::Success, None, "", None, now - Duration::days(100))
            .await
            .unwrap();

        // unset → off
        assert_eq!(prune_once(&store, now).await.unwrap(), (0, 0));
        // explicit 0 → off
        store.set_setting("pings_retention_days", "0").await.unwrap();
        assert_eq!(prune_once(&store, now).await.unwrap(), (0, 0));
        assert_eq!(store.list_recent_pings(cid, 10).await.unwrap().len(), 1);
    }
}
```

- [ ] **Step 4: Register the module in `src/lib.rs`**

Add (keeping alphabetical order with the other `pub mod` lines):

```rust
pub mod prune;
```

- [ ] **Step 5: Wire the loop into `src/main.rs`**

After the existing `tokio::spawn(scheduler::run_scan_loop(...))` line, add:

```rust
    tokio::spawn(pingward::prune::run_prune_loop(
        store.clone(),
        config.prune_interval_secs,
    ));
```

(`config.prune_interval_secs` is read before `config` is moved into `AppState::new`; capture it into a local `let prune_interval_secs = config.prune_interval_secs;` next to `scan_interval_secs` if the borrow checker requires it — follow the existing `scan_interval_secs` pattern.)

- [ ] **Step 6: Run the tests**

Run: `cargo nextest run --lib prune` then `cargo nextest run --lib config`
Expected: PASS. Then `cargo build` (confirms main.rs wiring compiles) and `cargo clippy --all-targets -- -D warnings` clean.

- [ ] **Step 7: Commit**

```bash
git add src/prune.rs src/lib.rs src/config.rs src/main.rs
git commit -m "feat: periodic prune loop + config prune interval"
```

---

### Task 3: Settings UI for retention

**Files:**
- Modify: `src/web.rs` (`SettingsTemplate`, `SettingsForm`, `settings_page`, `settings_save`)
- Modify: `templates/settings.html`
- Test: `tests/auth_web.rs`

**Interfaces:**
- Consumes: `Store::{get_setting, set_setting}`.

- [ ] **Step 1: Add fields to `SettingsTemplate` and `SettingsForm`**

`SettingsTemplate` (after `nag_interval: String,`):

```rust
    pings_retention_days: String,
    notifications_retention_days: String,
```

`SettingsForm` (after `nag_interval: String,`):

```rust
    pings_retention_days: String,
    notifications_retention_days: String,
```

- [ ] **Step 2: Load them in `settings_page`**

After the `nag_interval` load:

```rust
    let pings_retention_days = state
        .store
        .get_setting("pings_retention_days")
        .await?
        .unwrap_or_default();
    let notifications_retention_days = state
        .store
        .get_setting("notifications_retention_days")
        .await?
        .unwrap_or_default();
```

and add both to the `SettingsTemplate { … }` literal.

- [ ] **Step 3: Persist them in `settings_save`**

After the `nag_interval` persistence block, mirror the same blank-clears / positive-int-only logic for each key:

```rust
    let pr = form.pings_retention_days.trim();
    if pr.is_empty() {
        state.store.set_setting("pings_retention_days", "").await?;
    } else if pr.parse::<u64>().map(|v| v > 0).unwrap_or(false) {
        state.store.set_setting("pings_retention_days", pr).await?;
    }
    let nr = form.notifications_retention_days.trim();
    if nr.is_empty() {
        state.store.set_setting("notifications_retention_days", "").await?;
    } else if nr.parse::<u64>().map(|v| v > 0).unwrap_or(false) {
        state.store.set_setting("notifications_retention_days", nr).await?;
    }
```

- [ ] **Step 4: Update the template**

In `templates/settings.html`, after the nag interval label:

```html
  <label>Pings retention (days, blank = keep forever) <input name="pings_retention_days" value="{{ pings_retention_days }}"></label>
  <label>Notifications retention (days, blank = keep forever) <input name="notifications_retention_days" value="{{ notifications_retention_days }}"></label>
```

- [ ] **Step 5: Write a web test + fix existing settings POSTs**

In `tests/auth_web.rs`, mirror `admin_sets_global_scan_interval` (around line 315) to add `admin_sets_retention_days`: POST `/settings` as an admin with `pings_retention_days=30` and `notifications_retention_days=90` (plus the existing `scan_interval`/`nag_interval` keys the form needs), then assert `store.get_setting("pings_retention_days") == Some("30")` and `... notifications_retention_days == Some("90")`.

Because `SettingsForm` now has two new required fields, add `("pings_retention_days","")` and `("notifications_retention_days","")` to the body of ANY existing settings-form POST in this file that would otherwise omit them (e.g. `admin_sets_global_scan_interval`) — axum `Form` rejects a missing non-Option field.

- [ ] **Step 6: Run the tests**

Run: `cargo nextest run`
Expected: PASS, including `admin_sets_retention_days` and the existing settings test. `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` clean.

- [ ] **Step 7: Commit**

```bash
git add src/web.rs templates/settings.html tests/auth_web.rs
git commit -m "feat: retention settings UI (pings + notifications days)"
```

---

### Task 4: PostgreSQL parity test

**Files:**
- Modify: `tests/pg_store.rs`
- Test: same file (runs only with `TEST_DATABASE_URL` set)

**Interfaces:**
- Consumes: `Store::{insert_ping, record_notification, delete_pings_before, delete_notifications_before}` and `prune::prune_once`.

- [ ] **Step 1: Extend the round-trip with a prune cycle**

In `tests/pg_store.rs` `postgres_full_round_trip`, after the existing assertions, add a prune cycle (adapt to the existing `cid`/`chan`/`now` variable names in the test; a channel id is available from the channel-creation section):

```rust
    // retention/pruning: an old ping + old notification are deleted by prune_once
    // when retention is configured; a far-future cutoff via a large retention
    // keeps recent rows.
    let old = now - chrono::Duration::days(30);
    store.insert_ping(cid, pingward::models::PingKind::Success, None, "", None, old).await.unwrap();
    store.record_notification(cid, chan, pingward::notify::EventKind::Down, pingward::models::NotifyStatus::Ok, None, old).await.unwrap();
    store.set_setting("pings_retention_days", "7").await.unwrap();
    store.set_setting("notifications_retention_days", "7").await.unwrap();
    let (pd, nd) = pingward::prune::prune_once(&store, now).await.unwrap();
    assert!(pd >= 1 && nd >= 1, "expected old ping+notification pruned, got ({pd},{nd})");
    // direct delete method also works with an explicit cutoff
    let far = (now - chrono::Duration::days(3650)).to_rfc3339();
    assert_eq!(store.delete_pings_before(&far).await.unwrap(), 0);
```

Use the actual check id and channel id variable names already bound earlier in the test (read the file first). If the test does not already retain the channel id, capture it from the existing `create_channel` call.

- [ ] **Step 2: Run against live Postgres**

Ensure the `apple/container` Postgres is running (IP from `container ls`), then:

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@<ip>:5432/postgres cargo nextest run --test pg_store`
Expected: PASS (not skipped) — confirm `postgres_full_round_trip` ran.

- [ ] **Step 3: Full gate**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: clean/green (pg test skips without `TEST_DATABASE_URL`; verified live in Step 2).

- [ ] **Step 4: Commit**

```bash
git add tests/pg_store.rs
git commit -m "test: postgres parity for retention pruning"
```

---

## Self-Review Notes

- **Spec coverage:** delete methods (Task 1), prune_once + loop + config interval + wiring (Task 2), settings UI (Task 3), Postgres parity (Task 4). No migration (none needed). All spec sections map to a task.
- **Build-green ordering:** Task 1 adds methods (no signature changes). Task 2 adds a module + a Config field (Config is constructed only in `from_map`, updated in the same task) + main.rs wiring. Task 3 adds two required `SettingsForm` fields and fixes the existing settings POST in the same task. Each task compiles and tests on its own.
- **Type consistency:** retention values are stored/read as `settings` TEXT; parsed to `i64` days; cutoffs are `to_rfc3339()` strings; delete methods take `&str` and return `u64` (`rows_affected`). `prune_interval_secs` is `u64` like `scan_interval_secs`.
- **Off-invariant:** `parse_days` returns `None` for unset/blank/non-numeric/≤0, and `prune_once` deletes nothing (count 0) for that table — verified by `prune_once_off_when_unset_or_zero`.
