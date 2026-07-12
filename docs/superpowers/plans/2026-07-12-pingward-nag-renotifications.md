# pingward nag (repeat down-notifications) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add opt-in, interval-based repeat ("nag") notifications for checks that stay `down`, stoppable by recovery, pause, or a manual per-incident acknowledge.

**Architecture:** A new `EventKind::Reminder` is emitted by a `nag_once` scan function (symmetric to `scan_once`) for every down, un-acknowledged check whose nag interval has elapsed since its last alert. Nag interval resolves through the existing scan cascade (check → project → global), but with no env fallback (opt-in). New `checks` columns (`nag_interval_secs`, `last_alert_at`, `acknowledged`) and a `projects.nag_interval_secs` column carry the state.

**Tech Stack:** Rust, axum 0.8, sqlx 0.9 `Any` driver (SQLite + PostgreSQL), askama templates, chrono.

## Global Constraints

- Dual backend: every schema change ships in BOTH `migrations/sqlite/` and `migrations/postgres/`. SQLite uses `INTEGER`; Postgres uses `BIGINT`. Timestamps are `TEXT` (RFC3339). Integer/boolean columns are `INTEGER`/`BIGINT` so `row.get::<i64,_>` / `row.get::<Option<i64>,_>` stays uniform.
- SQL placeholders are numbered `$1, $2, …` (the `Any` driver does not translate `?`). Inserts that need the new id use `RETURNING id` + `row.get::<i64,_>("id")`.
- nag is **opt-in**: `effective_nag_interval` returns `None` (off) when every cascade level is unset or ≤ 0. There is NO env-var default for nag.
- Rust: `cargo fmt` before commit; tests via `cargo nextest run`; clippy is `-D warnings` in CI (no `map_or(true, …)`; use `is_none_or`). MSRV / `rust-version` MUST NOT change. Do not modify `version` or `CHANGELOG.md`.
- Commits GPG-signed. Stage files explicitly by name (never `git add -A`/`.`).
- The design spec is `docs/superpowers/specs/2026-07-12-pingward-nag-renotifications-design.md`.

## File Structure

- `migrations/sqlite/0004_nag.sql`, `migrations/postgres/0004_nag.sql` — new columns + widened `notifications.event` CHECK. **Create.**
- `src/models.rs` — `Check` gains `nag_interval_secs`, `last_alert_at`, `acknowledged`; `Project` gains `nag_interval_secs`. **Modify.**
- `src/store.rs` — `row_to_check`/`row_to_project` read new columns; new nag-state methods; `update_check_schedule` / `create_project` / `update_project` gain a nag param. **Modify.**
- `src/notify.rs` — `EventKind::Reminder` + rendering. **Modify.**
- `src/config.rs` — `effective_nag_interval` resolver. **Modify.**
- `src/scheduler.rs` — `nag_once`; scan_once + loop wiring; `base_check` test helper gains fields. **Modify.**
- `src/ping.rs` — recovery clears nag state; fail-down sets alert baseline. **Modify.**
- `src/web.rs` — check/project/settings forms carry nag interval; `check_ack` handler + route. **Modify.**
- `templates/check_form.html`, `project_form.html`, `settings.html`, `check.html` — nag interval fields + Acknowledge button. **Modify.**
- `tests/pg_store.rs` — extend round-trip to cover nag columns + a nag cycle. **Modify.**

---

### Task 1: Schema migration, model fields, row mappers

**Files:**
- Create: `migrations/sqlite/0004_nag.sql`
- Create: `migrations/postgres/0004_nag.sql`
- Modify: `src/models.rs:29-46` (`Check`), `src/models.rs:57-64` (`Project`)
- Modify: `src/store.rs:45-62` (`row_to_check`), `src/store.rs:76-85` (`row_to_project`)
- Modify: `src/scheduler.rs:151-170` (`base_check` test helper)
- Test: `src/db.rs` (migration test), `src/store.rs` (round-trip defaults)

**Interfaces:**
- Produces: `Check.nag_interval_secs: Option<i64>`, `Check.last_alert_at: Option<DateTime<Utc>>`, `Check.acknowledged: bool`, `Project.nag_interval_secs: Option<i64>`. Columns `checks.nag_interval_secs`, `checks.last_alert_at`, `checks.acknowledged`, `projects.nag_interval_secs`; `notifications.event` CHECK now allows `'reminder'`.

- [ ] **Step 1: Write the SQLite migration**

Create `migrations/sqlite/0004_nag.sql`:

```sql
ALTER TABLE checks ADD COLUMN nag_interval_secs INTEGER;
ALTER TABLE checks ADD COLUMN last_alert_at TEXT;
ALTER TABLE checks ADD COLUMN acknowledged INTEGER NOT NULL DEFAULT 0;
ALTER TABLE projects ADD COLUMN nag_interval_secs INTEGER;

-- Widen notifications.event CHECK to include 'reminder'. SQLite cannot ALTER a
-- CHECK constraint, so rebuild the table. Nothing references notifications, so
-- the drop is safe.
CREATE TABLE notifications_new (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  check_id INTEGER NOT NULL REFERENCES checks(id) ON DELETE CASCADE,
  channel_id INTEGER NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
  event TEXT NOT NULL CHECK (event IN ('down','up','reminder')),
  status TEXT NOT NULL CHECK (status IN ('ok','error')),
  error TEXT,
  created_at TEXT NOT NULL
);
INSERT INTO notifications_new (id, check_id, channel_id, event, status, error, created_at)
  SELECT id, check_id, channel_id, event, status, error, created_at FROM notifications;
DROP TABLE notifications;
ALTER TABLE notifications_new RENAME TO notifications;
```

- [ ] **Step 2: Write the Postgres migration**

Create `migrations/postgres/0004_nag.sql`:

```sql
ALTER TABLE checks ADD COLUMN nag_interval_secs BIGINT;
ALTER TABLE checks ADD COLUMN last_alert_at TEXT;
ALTER TABLE checks ADD COLUMN acknowledged BIGINT NOT NULL DEFAULT 0;
ALTER TABLE projects ADD COLUMN nag_interval_secs BIGINT;

ALTER TABLE notifications DROP CONSTRAINT notifications_event_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_event_check
  CHECK (event IN ('down','up','reminder'));
```

- [ ] **Step 3: Add fields to the models**

In `src/models.rs`, add to `struct Check` (after `pub scan_interval_secs: Option<i64>,` / `pub max_runtime_secs: Option<i64>,`):

```rust
    pub nag_interval_secs: Option<i64>,
    pub last_alert_at: Option<DateTime<Utc>>,
    pub acknowledged: bool,
```

Add to `struct Project` (after `pub scan_interval_secs: Option<i64>,`):

```rust
    pub nag_interval_secs: Option<i64>,
```

- [ ] **Step 4: Read the new columns in the row mappers**

In `src/store.rs` `row_to_check`, inside the `Ok(Check { … })` literal (after `max_runtime_secs: row.get("max_runtime_secs"),`):

```rust
        nag_interval_secs: row.get("nag_interval_secs"),
        last_alert_at: parse_ts(row.get("last_alert_at")),
        acknowledged: row.get::<i64, _>("acknowledged") != 0,
```

In `row_to_project`, inside `Ok(Project { … })` (after `scan_interval_secs: row.get("scan_interval_secs"),`):

```rust
        nag_interval_secs: row.get("nag_interval_secs"),
```

- [ ] **Step 5: Fix the `base_check` test helper**

In `src/scheduler.rs` `base_check()`, add the three new fields to the `Check { … }` literal (after `max_runtime_secs: None,`):

```rust
            nag_interval_secs: None,
            last_alert_at: None,
            acknowledged: false,
```

- [ ] **Step 6: Add a store round-trip test for defaults**

In `src/store.rs` tests module, add:

```rust
    #[tokio::test]
    async fn new_check_has_nag_defaults() {
        let store = fresh_store().await;
        store.create_user("u", None, false, Utc::now()).await.unwrap();
        store.create_project(1, "p", None, Utc::now()).await.unwrap();
        let id = store
            .create_check(1, "c", "uu", ScheduleKind::Period, Some(60), 30, None, "UTC")
            .await
            .unwrap();
        let c = store.find_check(id).await.unwrap().unwrap();
        assert_eq!(c.nag_interval_secs, None);
        assert_eq!(c.last_alert_at, None);
        assert!(!c.acknowledged);
    }
```

Use whatever in-module helper already builds a migrated in-memory store (mirror an existing `#[tokio::test]` in `src/store.rs` — e.g. the pattern used by `list_active_checks_includes_up_status`; if there is no shared `fresh_store()` helper, inline `db::connect("sqlite::memory:")` + `db::migrate` exactly as that test does).

- [ ] **Step 7: Run the tests**

Run: `cargo nextest run --lib`
Expected: PASS, including `new_check_has_nag_defaults`, `migrate_creates_checks_table`, and the existing scheduler tests (which construct `base_check`).

- [ ] **Step 8: Commit**

```bash
git add migrations/sqlite/0004_nag.sql migrations/postgres/0004_nag.sql src/models.rs src/store.rs src/scheduler.rs
git commit -m "feat: nag schema — nag_interval/last_alert/acknowledged columns"
```

---

### Task 2: `EventKind::Reminder` and notification rendering

**Files:**
- Modify: `src/notify.rs:5-29` (`EventKind` + `FromStr`), `src/notify.rs:63-87` (`event_text`/`event_title`), `src/notify.rs:249-252` (ntfy), `src/notify.rs:306-309` (pushover)
- Test: `src/notify.rs` tests module

**Interfaces:**
- Produces: `EventKind::Reminder` with `as_str() == "reminder"`, `FromStr` accepting `"reminder"`. Text channels render reminders as a red "STILL DOWN" line; ntfy `high`/`red_circle`; pushover priority `1`.

- [ ] **Step 1: Add the enum variant + string mapping**

In `src/notify.rs`, extend `EventKind`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Down,
    Up,
    Reminder,
}
```

Extend `as_str`:

```rust
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::Down => "down",
            EventKind::Up => "up",
            EventKind::Reminder => "reminder",
        }
    }
```

Extend `FromStr`:

```rust
        match s {
            "down" => Ok(EventKind::Down),
            "up" => Ok(EventKind::Up),
            "reminder" => Ok(EventKind::Reminder),
            other => Err(format!("invalid EventKind: {other}")),
        }
```

- [ ] **Step 2: Render reminder text**

In `event_text`, extend the match so a reminder reads as a red "STILL DOWN" line:

```rust
    let (emoji, word) = match ev.event {
        EventKind::Down => ("\u{1F534}", "DOWN"),          // red circle
        EventKind::Up => ("\u{1F7E2}", "UP"),              // green circle
        EventKind::Reminder => ("\u{1F534}", "STILL DOWN"), // red circle
    };
```

(`event_title` already uses `ev.event.as_str()`, so it yields `"pingward: {name} reminder"` with no change.)

- [ ] **Step 3: Set reminder priority on ntfy + pushover**

In the ntfy `send` match (`let (priority, tags) = match ev.event { … }`):

```rust
            let (priority, tags) = match ev.event {
                EventKind::Down => ("high", "red_circle"),
                EventKind::Up => ("default", "green_circle"),
                EventKind::Reminder => ("high", "red_circle"),
            };
```

In the pushover `send` match (`let priority = match ev.event { … }`):

```rust
            let priority = match ev.event {
                EventKind::Down => "1",
                EventKind::Up => "0",
                EventKind::Reminder => "1",
            };
```

- [ ] **Step 4: Write tests**

In `src/notify.rs` tests module, add:

```rust
    #[test]
    fn reminder_event_roundtrips_and_renders_still_down() {
        assert_eq!(EventKind::Reminder.as_str(), "reminder");
        assert_eq!(
            std::str::FromStr::from_str("reminder"),
            Ok(EventKind::Reminder)
        );
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Reminder,
            at: Utc::now(),
            project_id: 1,
        };
        let text = event_text(&ev);
        assert!(text.contains("STILL DOWN"), "got: {text}");
        assert_eq!(event_title(&ev), "pingward: backup reminder");
    }
```

- [ ] **Step 5: Run the tests**

Run: `cargo nextest run --lib notify`
Expected: PASS, including `reminder_event_roundtrips_and_renders_still_down`.

- [ ] **Step 6: Commit**

```bash
git add src/notify.rs
git commit -m "feat: EventKind::Reminder with STILL DOWN rendering"
```

---

### Task 3: Nag-state store methods

**Files:**
- Modify: `src/store.rs` (add methods; near `list_active_checks` and `all_project_scan_intervals`)
- Test: `src/store.rs` tests module

**Interfaces:**
- Produces:
  - `list_down_checks(&self) -> Result<Vec<Check>, sqlx::Error>`
  - `all_project_nag_intervals(&self) -> Result<HashMap<i64, Option<i64>>, sqlx::Error>`
  - `begin_down_alert(&self, check_id: i64, at: DateTime<Utc>) -> Result<(), sqlx::Error>`
  - `record_reminder(&self, check_id: i64, at: DateTime<Utc>) -> Result<(), sqlx::Error>`
  - `clear_nag(&self, check_id: i64) -> Result<(), sqlx::Error>`
  - `acknowledge(&self, check_id: i64) -> Result<(), sqlx::Error>`

- [ ] **Step 1: Add `list_down_checks`**

In `src/store.rs`, after `list_active_checks` (ends ~line 169), add a method that mirrors it but selects down checks, reusing the same corrupt-row-skipping loop:

```rust
    /// Checks currently in `down` status — the candidates for nag reminders.
    /// Corrupt rows are logged and skipped, mirroring `list_active_checks`.
    pub async fn list_down_checks(&self) -> Result<Vec<Check>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM checks WHERE status = 'down'")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            match row_to_check(row) {
                Ok(check) => out.push(check),
                Err(e) => {
                    let id: i64 = row.get("id");
                    tracing::error!("skipping corrupt checks row id={id}: {e}");
                    continue;
                }
            }
        }
        Ok(out)
    }
```

- [ ] **Step 2: Add `all_project_nag_intervals`**

After `all_project_scan_intervals` (~line 414), add:

```rust
    pub async fn all_project_nag_intervals(
        &self,
    ) -> Result<HashMap<i64, Option<i64>>, sqlx::Error> {
        let rows = sqlx::query("SELECT id, nag_interval_secs FROM projects")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get::<i64, _>("id"),
                    r.get::<Option<i64>, _>("nag_interval_secs"),
                )
            })
            .collect())
    }
```

- [ ] **Step 3: Add the four nag-state mutators**

Add near `set_status` (~line 219):

```rust
    /// Mark the start of a down incident's alerting: stamp the alert baseline
    /// and clear any prior acknowledgement so a fresh incident is never silent.
    pub async fn begin_down_alert(
        &self,
        check_id: i64,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE checks SET last_alert_at=$1, acknowledged=0 WHERE id=$2")
            .bind(at.to_rfc3339())
            .bind(check_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Advance the alert baseline after emitting a reminder.
    pub async fn record_reminder(
        &self,
        check_id: i64,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE checks SET last_alert_at=$1 WHERE id=$2")
            .bind(at.to_rfc3339())
            .bind(check_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Clear nag state on recovery: no acknowledgement, no alert baseline.
    pub async fn clear_nag(&self, check_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE checks SET acknowledged=0, last_alert_at=NULL WHERE id=$1")
            .bind(check_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Silence reminders for the current down incident.
    pub async fn acknowledge(&self, check_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE checks SET acknowledged=1 WHERE id=$1")
            .bind(check_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
```

- [ ] **Step 4: Write tests**

In `src/store.rs` tests module, add a test that exercises the lifecycle (build the migrated in-memory store the same way the existing store tests do; create user id 1, project id 1, one check):

```rust
    #[tokio::test]
    async fn nag_state_methods_roundtrip() {
        let store = fresh_store().await; // or inline connect+migrate as elsewhere in this module
        store.create_user("u", None, false, Utc::now()).await.unwrap();
        store.create_project(1, "p", Some(0), Utc::now()).await.unwrap();
        let id = store
            .create_check(1, "c", "uu", ScheduleKind::Period, Some(60), 30, None, "UTC")
            .await
            .unwrap();
        store.set_status(id, CheckStatus::Down).await.unwrap();

        // down check appears in list_down_checks
        let down = store.list_down_checks().await.unwrap();
        assert_eq!(down.len(), 1);
        assert_eq!(down[0].id, id);

        let t0 = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();
        store.begin_down_alert(id, t0).await.unwrap();
        let c = store.find_check(id).await.unwrap().unwrap();
        assert_eq!(c.last_alert_at, Some(t0));
        assert!(!c.acknowledged);

        store.acknowledge(id).await.unwrap();
        assert!(store.find_check(id).await.unwrap().unwrap().acknowledged);

        let t1 = t0 + chrono::Duration::seconds(90);
        store.record_reminder(id, t1).await.unwrap();
        assert_eq!(store.find_check(id).await.unwrap().unwrap().last_alert_at, Some(t1));

        store.clear_nag(id).await.unwrap();
        let c = store.find_check(id).await.unwrap().unwrap();
        assert_eq!(c.last_alert_at, None);
        assert!(!c.acknowledged);

        // project nag intervals map exposes the (possibly-null) override
        let map = store.all_project_nag_intervals().await.unwrap();
        assert!(map.contains_key(&1));
    }
```

Ensure `use chrono::TimeZone;` is available in the test (the module already imports chrono in other tests; add the import if missing).

- [ ] **Step 5: Run the tests**

Run: `cargo nextest run --lib store`
Expected: PASS, including `nag_state_methods_roundtrip`.

- [ ] **Step 6: Commit**

```bash
git add src/store.rs
git commit -m "feat: nag-state store methods (list_down, alert baseline, ack, clear)"
```

---

### Task 4: Cascade resolver + scan/ping/nag wiring

**Files:**
- Modify: `src/config.rs` (add `effective_nag_interval` + tests)
- Modify: `src/scheduler.rs` (`scan_once` sets alert baseline; add `nag_once`; wire loop)
- Modify: `src/ping.rs:146-180` (recovery clears nag; fail-down sets baseline)
- Test: `src/config.rs` tests, `src/scheduler.rs` tests

**Interfaces:**
- Consumes: `Store::{list_down_checks, all_project_nag_intervals, get_setting, begin_down_alert, record_reminder, clear_nag}` (Task 3); `EventKind::Reminder` (Task 2); `Check.{nag_interval_secs, last_alert_at, acknowledged}` (Task 1).
- Produces: `config::effective_nag_interval(check, project, global) -> Option<i64>`; `scheduler::nag_once(&Store, DateTime<Utc>) -> Result<Vec<NotificationEvent>, sqlx::Error>`.

- [ ] **Step 1: Write the resolver test**

In `src/config.rs` tests module, add:

```rust
    #[test]
    fn nag_cascade_prefers_most_specific_and_is_opt_in() {
        assert_eq!(effective_nag_interval(Some(5), Some(10), Some(20)), Some(5));
        assert_eq!(effective_nag_interval(None, Some(10), Some(20)), Some(10));
        assert_eq!(effective_nag_interval(None, None, Some(20)), Some(20));
        // opt-in: all unset → off (no env default)
        assert_eq!(effective_nag_interval(None, None, None), None);
        // non-positive levels are skipped
        assert_eq!(effective_nag_interval(Some(0), Some(-1), Some(30)), Some(30));
        assert_eq!(effective_nag_interval(Some(0), None, None), None);
    }
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo nextest run --lib config::tests::nag_cascade_prefers_most_specific_and_is_opt_in`
Expected: FAIL (function not defined).

- [ ] **Step 3: Implement the resolver**

In `src/config.rs`, after `effective_scan_interval`, add:

```rust
/// Resolve the effective nag (repeat-notification) interval for a check from
/// the cascade: check → project → global. Returns `None` when nag is off at
/// every level (unset or non-positive). Unlike `effective_scan_interval`,
/// there is no env-default fallback — nag is opt-in.
pub fn effective_nag_interval(
    check_secs: Option<i64>,
    project_secs: Option<i64>,
    global_secs: Option<i64>,
) -> Option<i64> {
    [check_secs, project_secs, global_secs]
        .into_iter()
        .flatten()
        .find(|&v| v > 0)
}
```

- [ ] **Step 4: Run the resolver test to confirm it passes**

Run: `cargo nextest run --lib config`
Expected: PASS.

- [ ] **Step 5: Stamp the alert baseline when `scan_once` downs a check**

In `src/scheduler.rs` `scan_once`, replace the `set_status(Down)` block so a successful down also stamps the alert baseline and clears any stale ack. Change:

```rust
        if let Err(e) = store.set_status(check.id, CheckStatus::Down).await {
            tracing::error!("failed to down check {}: {e}", check.id);
            continue;
        }
```

to:

```rust
        if let Err(e) = store.set_status(check.id, CheckStatus::Down).await {
            tracing::error!("failed to down check {}: {e}", check.id);
            continue;
        }
        if let Err(e) = store.begin_down_alert(check.id, now).await {
            tracing::error!("failed to set alert baseline for {}: {e}", check.id);
        }
```

(The reminder baseline is best-effort: a failure here is logged but does not suppress the initial Down event, which is already queued below.)

- [ ] **Step 6: Add `nag_once`**

In `src/scheduler.rs`, add after `scan_once` (import `crate::config::effective_nag_interval` at the top alongside `effective_scan_interval`):

```rust
/// Emit a `Reminder` event for every down, un-acknowledged check whose nag
/// interval has elapsed since its last alert, advancing each reminded check's
/// `last_alert_at` so the next reminder is one interval later. `now` is
/// injected so the function stays deterministic (mirrors `scan_once`).
pub async fn nag_once(
    store: &Store,
    now: DateTime<Utc>,
) -> Result<Vec<NotificationEvent>, sqlx::Error> {
    let project_nags = store.all_project_nag_intervals().await?;
    let global_nag = store
        .get_setting("nag_interval")
        .await?
        .and_then(|v| v.parse::<i64>().ok());

    let mut events = Vec::new();
    for check in store.list_down_checks().await? {
        if check.acknowledged {
            continue;
        }
        let project = project_nags.get(&check.project_id).copied().flatten();
        let Some(interval) = effective_nag_interval(check.nag_interval_secs, project, global_nag)
        else {
            continue; // nag off for this check
        };
        let Some(last) = check.last_alert_at else {
            continue; // no baseline yet (e.g. downed before this feature shipped)
        };
        if now < last + Duration::seconds(interval) {
            continue; // not yet due
        }
        if let Err(e) = store.record_reminder(check.id, now).await {
            tracing::error!("failed to record reminder for {}: {e}", check.id);
            continue;
        }
        events.push(NotificationEvent {
            check_id: check.id,
            check_name: check.name.clone(),
            event: EventKind::Reminder,
            at: now,
            project_id: check.project_id,
        });
    }
    Ok(events)
}
```

- [ ] **Step 7: Wire `nag_once` into the loop**

In `run_scan_loop`, after the `scan_once` match block that spawns Down deliveries (right before the "Resolve the next sleep" comment), add a symmetric block:

```rust
        match nag_once(&store, Utc::now()).await {
            Ok(events) => {
                for ev in events {
                    let store = store.clone();
                    tokio::spawn(async move {
                        deliver_event(&store, &ev, RetryPolicy::default(), Utc::now()).await;
                    });
                }
            }
            Err(e) => tracing::error!("nag_once failed: {e}"),
        }
```

- [ ] **Step 8: Clear nag on recovery; stamp baseline on fail-down (ping.rs)**

In `src/ping.rs`, in the `PingKind::Success` arm, inside `if prev_status == CheckStatus::Down { … }`, before/after `spawn_delivery(… EventKind::Up …)` clear nag state:

```rust
            if prev_status == CheckStatus::Down {
                store.clear_nag(check.id).await?;
                spawn_delivery(
                    store.clone(),
                    check.id,
                    check.name.clone(),
                    check.project_id,
                    EventKind::Up,
                    now,
                );
            }
```

In the `PingKind::Fail` arm, inside `if matches!(prev_status, CheckStatus::Up | CheckStatus::New) { … }`, stamp the alert baseline so a fail-triggered down starts the nag clock:

```rust
            if matches!(prev_status, CheckStatus::Up | CheckStatus::New) {
                store.begin_down_alert(check.id, now).await?;
                spawn_delivery(
                    store.clone(),
                    check.id,
                    check.name.clone(),
                    check.project_id,
                    EventKind::Down,
                    now,
                );
            }
```

- [ ] **Step 9: Write scheduler tests for `nag_once`**

In `src/scheduler.rs` tests module, add (reuse the in-memory store setup pattern from `scan_once_downs_overrun_check`):

```rust
    async fn down_check_store() -> (Store, i64) {
        use crate::db;
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool, "sqlite::memory:").await.unwrap();
        let store = Store::new(pool);
        store.create_user("u", Some("x"), false, Utc::now()).await.unwrap();
        store.create_project(1, "p", None, Utc::now()).await.unwrap();
        let id = store
            .create_check(1, "job", "u1", ScheduleKind::Period, Some(3600), 300, None, "UTC")
            .await
            .unwrap();
        store.set_status(id, CheckStatus::Down).await.unwrap();
        (store, id)
    }

    #[tokio::test]
    async fn nag_once_reminds_due_unacked_check() {
        let (store, id) = down_check_store().await;
        // per-check nag interval 60s, baseline at t0
        let t0 = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();
        store
            .update_check_schedule(id, "job", ScheduleKind::Period, Some(3600), 300, None, "UTC", None, None, Some(60))
            .await
            .unwrap();
        store.begin_down_alert(id, t0).await.unwrap();

        // not yet due
        assert!(nag_once(&store, t0 + Duration::seconds(59)).await.unwrap().is_empty());
        // due
        let evs = nag_once(&store, t0 + Duration::seconds(60)).await.unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].event, EventKind::Reminder);
        // baseline advanced, so immediately after it is not due again
        assert!(nag_once(&store, t0 + Duration::seconds(61)).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn nag_once_skips_acked_and_off_and_no_baseline() {
        let (store, id) = down_check_store().await;
        let t0 = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();

        // no nag interval configured anywhere → off
        store.begin_down_alert(id, t0).await.unwrap();
        assert!(nag_once(&store, t0 + Duration::seconds(3600)).await.unwrap().is_empty());

        // configure interval, but acknowledged → skipped
        store
            .update_check_schedule(id, "job", ScheduleKind::Period, Some(3600), 300, None, "UTC", None, None, Some(60))
            .await
            .unwrap();
        store.acknowledge(id).await.unwrap();
        assert!(nag_once(&store, t0 + Duration::seconds(3600)).await.unwrap().is_empty());
    }
```

Note: these tests call `update_check_schedule` with the **10-argument** signature that Task 5 introduces (trailing `nag_interval_secs`). Task 4 is implemented before Task 5, so to keep Task 4's build green, add the trailing `nag_interval_secs: Option<i64>` parameter to `update_check_schedule` as the FIRST step here — see Step 10.

- [ ] **Step 10: Extend `update_check_schedule` signature (needed by the tests above and Task 5)**

In `src/store.rs` `update_check_schedule`, add a trailing parameter and persist it. Change the signature to end with `max_runtime_secs: Option<i64>, nag_interval_secs: Option<i64>,` and the SQL to `… max_runtime_secs=$8, nag_interval_secs=$9 WHERE id=$10`, binding `nag_interval_secs` before `id`:

```rust
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
        max_runtime_secs: Option<i64>,
        nag_interval_secs: Option<i64>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE checks SET name=$1, schedule_kind=$2, period_secs=$3, grace_secs=$4, \
             cron_expr=$5, timezone=$6, scan_interval_secs=$7, max_runtime_secs=$8, \
             nag_interval_secs=$9 WHERE id=$10",
        )
        .bind(name)
        .bind(kind.as_str())
        .bind(period_secs)
        .bind(grace_secs)
        .bind(cron_expr)
        .bind(timezone)
        .bind(scan_interval_secs)
        .bind(max_runtime_secs)
        .bind(nag_interval_secs)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
```

Then update the TWO existing call sites in `src/web.rs` (`check_create` ~line 507 and `check_update` ~line 627) to pass a trailing `parse_opt_i64(&form.nag_interval_secs)`. **But** `CheckForm` does not yet have a `nag_interval_secs` field — to keep THIS task's build green without pulling in the whole form change, pass `None` at both call sites for now:

```rust
            parse_opt_i64(&form.scan_interval_secs),
            parse_opt_i64(&form.max_runtime_secs),
            None,
        )
```

Task 5 replaces this `None` with `parse_opt_i64(&form.nag_interval_secs)`. Also fix the `scan_once_downs_overrun_check` test in `src/scheduler.rs`, which calls `update_check_schedule` — add a trailing `None`.

- [ ] **Step 11: Run tests**

Run: `cargo nextest run --lib`
Expected: PASS, including the new `nag_once_*` tests and existing scheduler/store/ping tests. Run `cargo clippy --all-targets -- -D warnings` and confirm clean.

- [ ] **Step 12: Commit**

```bash
git add src/config.rs src/scheduler.rs src/ping.rs src/store.rs src/web.rs
git commit -m "feat: nag_once reminder scan + cascade resolver + recovery/down wiring"
```

---

### Task 5: Nag interval config UI (check / project / settings forms)

**Files:**
- Modify: `src/web.rs` (`CheckForm`, `CheckFormTemplate`, `empty_check_form`, `check_create`, `check_edit`, `check_update`; `ProjectFormTemplate`, `ProjectForm`, `project_new`, `project_create`, `project_edit`, `project_update`; `SettingsTemplate`, `SettingsForm`, `settings_page`, `settings_save`)
- Modify: `src/store.rs` (`create_project`, `update_project` gain `nag_interval_secs`)
- Modify: `templates/check_form.html`, `templates/project_form.html`, `templates/settings.html`
- Test: `tests/auth_web.rs`

**Interfaces:**
- Consumes: `update_check_schedule`'s trailing `nag_interval_secs` (Task 4).
- Produces: `create_project`/`update_project` with a trailing `nag_interval_secs: Option<i64>`; check/project/settings forms round-trip a nag interval.

- [ ] **Step 1: Add `nag_interval_secs` to project store methods**

In `src/store.rs`, extend `create_project` and `update_project` with a trailing `nag_interval_secs: Option<i64>` param and persist it:

```rust
    pub async fn create_project(
        &self,
        user_id: i64,
        name: &str,
        scan_interval_secs: Option<i64>,
        nag_interval_secs: Option<i64>,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query(
            "INSERT INTO projects (user_id, name, scan_interval_secs, nag_interval_secs, created_at) \
             VALUES ($1,$2,$3,$4,$5) RETURNING id",
        )
        .bind(user_id)
        .bind(name)
        .bind(scan_interval_secs)
        .bind(nag_interval_secs)
        .bind(now.to_rfc3339())
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get::<i64, _>("id"))
    }

    pub async fn update_project(
        &self,
        id: i64,
        name: &str,
        scan_interval_secs: Option<i64>,
        nag_interval_secs: Option<i64>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE projects SET name = $1, scan_interval_secs = $2, nag_interval_secs = $3 WHERE id = $4",
        )
        .bind(name)
        .bind(scan_interval_secs)
        .bind(nag_interval_secs)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
```

Fix ALL `create_project` call sites to add the new arg (search: `create_project(`). Existing callers in `src/store.rs` tests, `src/notify.rs` tests, `src/scheduler.rs` tests, `tests/*.rs`, and `src/web.rs:project_create` — pass `None` except where the test specifically checks nag (pass an explicit value there). Fix `update_project` call site in `src/web.rs:project_update` (Step 4 wires the real value).

- [ ] **Step 2: Add the field to `CheckForm` + `CheckFormTemplate` + `empty_check_form`**

`CheckForm` (add after `max_runtime_secs: String,`): `nag_interval_secs: String,`
`CheckFormTemplate` (same): `nag_interval_secs: String,`
`empty_check_form` (after `max_runtime_secs: String::new(),`): `nag_interval_secs: String::new(),`

- [ ] **Step 3: Populate + preserve the field in check handlers**

- `check_create` error branch (~line 487): add `t.nag_interval_secs = form.nag_interval_secs;`
- `check_create` success call to `update_check_schedule`: replace the trailing `None` (from Task 4 Step 10) with `parse_opt_i64(&form.nag_interval_secs)`.
- `check_edit` `CheckFormTemplate { … }` (~line 590): add
  ```rust
        nag_interval_secs: check
            .nag_interval_secs
            .map(|v| v.to_string())
            .unwrap_or_default(),
  ```
- `check_update` error-branch `CheckFormTemplate { … }` (~line 619): add `nag_interval_secs: form.nag_interval_secs,`
- `check_update` success call to `update_check_schedule`: replace trailing `None` with `parse_opt_i64(&form.nag_interval_secs)`.

- [ ] **Step 4: Add the field to project form + handlers**

- `ProjectFormTemplate` (after `scan_interval_secs: String,`): `nag_interval_secs: String,`
- `ProjectForm` (after `scan_interval_secs: String,`): `nag_interval_secs: String,`
- `project_new`: `nag_interval_secs: String::new(),`
- `project_create`: pass `parse_opt_i64(&form.nag_interval_secs)` as the new `create_project` arg.
- `project_edit` `ProjectFormTemplate { … }`: add
  ```rust
        nag_interval_secs: project
            .nag_interval_secs
            .map(|v| v.to_string())
            .unwrap_or_default(),
  ```
- `project_update`: pass `parse_opt_i64(&form.nag_interval_secs)` as the new `update_project` arg.

- [ ] **Step 5: Add the global setting to the settings page**

- `SettingsTemplate` (after `scan_interval: String,`): `nag_interval: String,`
- `SettingsForm` (after `scan_interval: String,`): `nag_interval: String,`
- `settings_page`: load `nag_interval` alongside `scan_interval`:
  ```rust
      let nag_interval = state.store.get_setting("nag_interval").await?.unwrap_or_default();
  ```
  and pass it into `SettingsTemplate`.
- `settings_save`: mirror the `scan_interval` persistence for `nag_interval`:
  ```rust
      let nag = form.nag_interval.trim();
      if nag.is_empty() {
          state.store.set_setting("nag_interval", "").await?;
      } else if nag.parse::<u64>().map(|v| v > 0).unwrap_or(false) {
          state.store.set_setting("nag_interval", nag).await?;
      }
  ```

- [ ] **Step 6: Update the templates**

`templates/check_form.html` — after the max runtime label:

```html
  <label>Nag interval secs (blank = inherit/off) <input name="nag_interval_secs" value="{{ nag_interval_secs }}"></label>
```

`templates/project_form.html` — after the scan interval label:

```html
  <label>Nag interval (secs, blank = inherit/off)
    <input name="nag_interval_secs" value="{{ nag_interval_secs }}"></label>
```

`templates/settings.html` — after the scan interval label:

```html
  <label>Global nag interval (secs, blank = off) <input name="nag_interval" value="{{ nag_interval }}"></label>
```

- [ ] **Step 7: Write a web test**

In `tests/auth_web.rs`, mirror the existing `create_check_persists_max_runtime` test to add `create_check_persists_nag_interval` (post the check form with `nag_interval_secs=120`, then assert the stored check's `nag_interval_secs == Some(120)`). Include `("nag_interval_secs", "120")` in the form body and add `("nag_interval_secs","")` to any OTHER existing form-post helpers in this file that would otherwise omit the now-required field (axum `Form` rejects a missing non-Option `String` field — mirror how `max_runtime_secs` was handled).

- [ ] **Step 8: Run tests**

Run: `cargo nextest run` (lib + integration)
Expected: PASS, including `create_check_persists_nag_interval` and all existing `auth_web` tests. `cargo clippy --all-targets -- -D warnings` clean, `cargo fmt --check` clean.

- [ ] **Step 9: Commit**

```bash
git add src/web.rs src/store.rs templates/check_form.html templates/project_form.html templates/settings.html tests/auth_web.rs
git commit -m "feat: nag interval config UI (check/project/global cascade)"
```

---

### Task 6: Acknowledge endpoint + button

**Files:**
- Modify: `src/web.rs` (`routes()` add `/checks/{id}/ack`; add `check_ack` handler)
- Modify: `templates/check.html` (Acknowledge button, shown when down)
- Test: `tests/auth_web.rs`

**Interfaces:**
- Consumes: `Store::acknowledge` (Task 3); `owned_check` authorization helper.

- [ ] **Step 1: Add the route**

In `src/web.rs` `routes()`, after the pause/resume routes (~line 44):

```rust
        .route("/checks/{id}/ack", post(check_ack))
```

- [ ] **Step 2: Add the handler**

Mirror `check_pause` (owner/admin authorization via `owned_check`):

```rust
async fn check_ack(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    owned_check(&state.store, id, user.id).await?;
    state.store.acknowledge(id).await?;
    Ok(Redirect::to(&format!("/checks/{id}")).into_response())
}
```

- [ ] **Step 3: Add the button to the check page**

In `templates/check.html`, inside the actions `<p>`, add an Acknowledge form shown only while the check is down and not already acknowledged:

```html
  {% if check.status.as_str() == "down" && !check.acknowledged %}
    <form class="inline" method="post" action="/checks/{{ check.id }}/ack"><button>Acknowledge</button></form>
  {% endif %}
  {% if check.acknowledged %}<span class="status-paused">acknowledged</span>{% endif %}
```

- [ ] **Step 4: Write tests**

In `tests/auth_web.rs`, add two tests mirroring existing pause/authorization tests:

- `acknowledge_persists`: create a check, set it down (via `store.set_status(id, CheckStatus::Down)` on the test's store handle, or a fail ping), POST `/checks/{id}/ack` as the owner, assert `SEE_OTHER` and that `store.find_check(id).acknowledged == true`.
- `non_owner_cannot_acknowledge`: a second user POSTing `/checks/{id}/ack` gets `AppError::NotFound`'s status (mirror the existing non-owner negative-path test for pause/other check mutations — match its exact asserted status).

- [ ] **Step 5: Run tests**

Run: `cargo nextest run`
Expected: PASS, including the two new ack tests.

- [ ] **Step 6: Commit**

```bash
git add src/web.rs templates/check.html tests/auth_web.rs
git commit -m "feat: acknowledge endpoint + button to silence nag per incident"
```

---

### Task 7: PostgreSQL parity test extension

**Files:**
- Modify: `tests/pg_store.rs`
- Test: the same file (runs only when `TEST_DATABASE_URL` is set)

**Interfaces:**
- Consumes: all nag store methods + `update_check_schedule`/`create_project` new params.

- [ ] **Step 1: Extend the round-trip**

In `tests/pg_store.rs` `postgres_full_round_trip`, after the existing check assertions, add a nag cycle so the new columns and constraint are exercised on Postgres:

```rust
    // nag: configure a per-check interval, down the check, stamp a baseline,
    // and confirm the reminder scan and acknowledge/clear cycle work on PG.
    store
        .update_check_schedule(cid, "web-check", ScheduleKind::Period, Some(60), 30, None, "UTC", None, None, Some(60))
        .await
        .unwrap();
    store.set_status(cid, pingward::models::CheckStatus::Down).await.unwrap();
    let t0 = now;
    store.begin_down_alert(cid, t0).await.unwrap();
    let due = t0 + chrono::Duration::seconds(90);
    let evs = pingward::scheduler::nag_once(&store, due).await.unwrap();
    assert!(evs.iter().any(|e| e.check_id == cid && e.event == pingward::notify::EventKind::Reminder));
    store.acknowledge(cid).await.unwrap();
    assert!(store.find_check(cid).await.unwrap().unwrap().acknowledged);
    // acknowledged → no further reminders
    assert!(pingward::scheduler::nag_once(&store, due + chrono::Duration::seconds(300))
        .await
        .unwrap()
        .into_iter()
        .all(|e| e.check_id != cid));
    store.clear_nag(cid).await.unwrap();
    assert_eq!(store.find_check(cid).await.unwrap().unwrap().last_alert_at, None);
```

Adjust the check name / `cid` variable to match what the existing test uses. Also update any `update_check_schedule` / `create_project` calls already present in this file to the new arities.

- [ ] **Step 2: Run against live Postgres**

Ensure the `apple/container` Postgres is running (see `docs`/memory), then:

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@<container-ip>:5432/postgres cargo nextest run --test pg_store`
Expected: PASS (not skipped) — confirm `postgres_full_round_trip` ran.

- [ ] **Step 3: Full gate**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: all clean/green (Postgres test may skip without `TEST_DATABASE_URL`; it was verified live in Step 2).

- [ ] **Step 4: Commit**

```bash
git add tests/pg_store.rs
git commit -m "test: postgres parity for nag columns + reminder cycle"
```

---

## Self-Review Notes

- **Spec coverage:** state model (Task 1), cascade resolver (Task 4), lifecycle transitions — down/reminder/recovery (Task 4), acknowledge (Task 3 method + Task 6 endpoint), Reminder event + rendering (Task 2), migration incl. widened CHECK (Task 1), all UI surfaces (Task 5/6), Postgres parity (Task 7). All spec sections map to a task.
- **Build-green ordering:** the only cross-task signature churn is `update_check_schedule` (introduced in Task 4 Step 10 with `None` placeholders at call sites, real value wired in Task 5) and `create_project`/`update_project` (Task 5). Each task compiles and tests on its own.
- **Type consistency:** `acknowledged` is stored as `INTEGER`/`BIGINT` 0/1 and mapped to `bool` (`row.get::<i64,_>(…) != 0`), matching the existing `is_admin` pattern. `last_alert_at` is `TEXT` RFC3339 via `parse_ts`, matching other timestamps. Nag intervals are `Option<i64>` throughout.
- **Opt-in invariant:** `effective_nag_interval` has no env fallback; `nag_once` skips checks with no resolved interval and checks with no `last_alert_at` baseline (so checks downed before this feature never nag until they re-enter a down incident).
