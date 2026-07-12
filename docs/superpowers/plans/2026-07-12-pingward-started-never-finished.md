# Started-but-never-finished Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Detect a check that sent a `start` ping but never completed within a per-check maximum runtime, transition it to `down`, and notify (spec §12 "started but never finished" detection).

**Architecture:** Add an optional per-check `max_runtime_secs`. The `start` ping already records `last_start_at` without touching `last_ping_at` or `status`, so a run is "in flight" when `last_start_at` is newer than `last_ping_at`. The existing scan loop already downs overdue checks; extend `scan_once` so a check that is in-flight past `last_start_at + max_runtime_secs` is also downed and emits a `Down` event (reusing the existing per-check channel delivery + Down→success→Up recovery).

**Tech Stack:** Rust + tokio, sqlx `Any` (SQLite + PostgreSQL), askama templates.

## Global Constraints

- Runs on **both SQLite and PostgreSQL** via the sqlx `Any` driver — every schema change ships in BOTH `migrations/sqlite/` and `migrations/postgres/`, and every SQL statement uses `$N` placeholders (never `?`; the `Any` driver does not translate `?` for Postgres). Integer columns are `INTEGER` in SQLite / `BIGINT` in Postgres so `row.get::<i64,_>` is uniform.
- No behaviour change when `max_runtime_secs` is unset (NULL) — detection is opt-in per check. The full existing suite (`cargo nextest run`, currently 84 tests) must stay green.
- `max_runtime_secs` is stored via the existing `update_check_schedule` path (which `check_create` already calls right after `create_check`); `create_check`'s signature does NOT change.
- No new dependency. Tests via `cargo nextest run`; `cargo fmt` before commit; `cargo clippy --all-targets -- -D warnings` clean (CI-enforced). Commits GPG-signed; stage files explicitly.
- `scan_once` stays deterministic (takes `now`); `Utc::now()` is only called in `run_scan_loop`. Per-check failures are logged and skipped, never abort the round.

---

## File Structure

- `migrations/sqlite/0003_max_runtime.sql`, `migrations/postgres/0003_max_runtime.sql` (CREATE): add the `max_runtime_secs` column to `checks`.
- `src/models.rs` (MODIFY): add `max_runtime_secs: Option<i64>` to `Check`.
- `src/store.rs` (MODIFY): `row_to_check` reads the new column; `update_check_schedule` gains a `max_runtime_secs` param and sets it.
- `src/web.rs` (MODIFY): `CheckForm` + `CheckFormTemplate` gain a `max_runtime_secs` string field; `check_create`/`check_update` pass it; all `CheckFormTemplate` construction sites populate it.
- `templates/check_form.html` (MODIFY): add the input.
- `src/scheduler.rs` (MODIFY): add `overrun_time()` and extend `scan_once` to down in-flight checks past their max runtime + unit tests.
- `tests/auth_web.rs` (MODIFY): integration test that `max_runtime_secs` persists through the form.

---

### Task 1: Persist an optional per-check `max_runtime_secs`

**Files:**
- Create: `migrations/sqlite/0003_max_runtime.sql`, `migrations/postgres/0003_max_runtime.sql`
- Modify: `src/models.rs`, `src/store.rs`, `src/web.rs`, `templates/check_form.html`
- Test: `tests/auth_web.rs`

**Interfaces:**
- Consumes: existing `Check`, `update_check_schedule`, `CheckForm`, `parse_opt_i64`.
- Produces: `Check.max_runtime_secs: Option<i64>`; `update_check_schedule(..., scan_interval_secs, max_runtime_secs)` (one new trailing `Option<i64>` param); the check form exposes a "Max runtime seconds" input.

- [ ] **Step 1: Add the migration to BOTH backends**

Create `migrations/sqlite/0003_max_runtime.sql`:

```sql
ALTER TABLE checks ADD COLUMN max_runtime_secs INTEGER;
```

Create `migrations/postgres/0003_max_runtime.sql`:

```sql
ALTER TABLE checks ADD COLUMN max_runtime_secs BIGINT;
```

(Nullable, no default → NULL for existing rows, i.e. detection off.)

- [ ] **Step 2: Add the model field**

In `src/models.rs`, add to `struct Check` (place it next to `scan_interval_secs`):

```rust
    pub max_runtime_secs: Option<i64>,
```

This will break every `Check { .. }` literal (the `base_check()` builder in `src/scheduler.rs` tests) — fix those in Step 6 / let the compiler guide you.

- [ ] **Step 3: Read the column in `row_to_check`**

In `src/store.rs` `row_to_check`, add alongside `scan_interval_secs`:

```rust
        max_runtime_secs: row.get("max_runtime_secs"),
```

- [ ] **Step 4: Extend `update_check_schedule`**

In `src/store.rs`, add a trailing `max_runtime_secs: Option<i64>` parameter and set the column. Update the SQL (renumber placeholders so the new bind is last, then `WHERE id` after it). Current statement sets `name=$1, schedule_kind=$2, period_secs=$3, grace_secs=$4, cron_expr=$5, timezone=$6, scan_interval_secs=$7 WHERE id=$8` — becomes:

```rust
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
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE checks SET name=$1, schedule_kind=$2, period_secs=$3, grace_secs=$4, \
             cron_expr=$5, timezone=$6, scan_interval_secs=$7, max_runtime_secs=$8 WHERE id=$9",
        )
        .bind(name)
        .bind(kind.as_str())
        .bind(period_secs)
        .bind(grace_secs)
        .bind(cron_expr)
        .bind(timezone)
        .bind(scan_interval_secs)
        .bind(max_runtime_secs)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
```

(Match the existing bind order/types exactly; only the two new binds — `max_runtime_secs` before `id` — and the renumbered `$8`/`$9` are added.)

- [ ] **Step 5: Wire the web form**

In `src/web.rs`:

1. Add `max_runtime_secs: String,` to both `struct CheckForm` and `struct CheckFormTemplate`.
2. Populate `max_runtime_secs` at every `CheckFormTemplate` construction site: `empty_check_form` (→ `String::new()`), `check_edit` (→ `check.max_runtime_secs.map(|v| v.to_string()).unwrap_or_default()`), and the error re-render paths in `check_create`/`check_update` (→ echo `form.max_runtime_secs.clone()`). Let the compiler list the sites.
3. In BOTH `update_check_schedule(...)` call sites (`check_create` and `check_update`), add a trailing `parse_opt_i64(&form.max_runtime_secs)` argument.

Then in `templates/check_form.html`, add an input after the scan-interval one:

```html
  <label>Max runtime secs (blank = off) <input name="max_runtime_secs" value="{{ max_runtime_secs }}"></label>
```

- [ ] **Step 6: Fix the `base_check()` test builder**

In `src/scheduler.rs` tests, add `max_runtime_secs: None,` to the `base_check()` `Check { .. }` literal (and any other `Check { .. }` literal the compiler flags).

- [ ] **Step 7: Write the persistence integration test**

Add to `tests/auth_web.rs` (mirror the existing `create_check_and_pause_resume` style; `server_with_project()` returns `(server, store, pid)`):

```rust
#[tokio::test]
async fn create_check_persists_max_runtime() {
    let (server, store, pid) = server_with_project().await;
    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("name", "job"),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("grace_secs", "300"),
            ("cron_expr", ""),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", "120"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let checks = store.list_checks_for_project(pid).await.unwrap();
    assert_eq!(checks[0].max_runtime_secs, Some(120));
}
```

- [ ] **Step 8: Run the suite + fmt + clippy**

Run: `cargo nextest run`
Expected: PASS — full suite green including the new test. Then `cargo fmt` and `cargo clippy --all-targets -- -D warnings` clean.

Also confirm the Postgres migration applies: a Postgres 17 instance runs via the `container` CLI (see the dispatch for the IP). Run
`TEST_DATABASE_URL="postgres://postgres:postgres@<PG_IP>:5432/postgres" cargo nextest run --test pg_store`
Expected: PASS — `postgres_full_round_trip` still green (it re-creates the schema, so the new migration must apply cleanly on Postgres).

- [ ] **Step 9: Commit**

```bash
git add migrations/sqlite/0003_max_runtime.sql migrations/postgres/0003_max_runtime.sql src/models.rs src/store.rs src/web.rs templates/check_form.html src/scheduler.rs tests/auth_web.rs
git commit -m "feat: add optional per-check max_runtime_secs"
```

---

### Task 2: Detect and down in-flight checks past their max runtime

**Files:**
- Modify: `src/scheduler.rs` (add `overrun_time`, extend `scan_once`, add unit tests)

**Interfaces:**
- Consumes: `Check.max_runtime_secs`, `Check.last_start_at`, `Check.last_ping_at`, `due_time`, `store.set_status`, `NotificationEvent`, `EventKind::Down`.
- Produces: `overrun_time(&Check) -> Option<DateTime<Utc>>`; `scan_once` also downs in-flight-overrun checks.

- [ ] **Step 1: Write the failing unit tests**

Add to `src/scheduler.rs` tests (they use `base_check()` from Task 1, `chrono::{TimeZone, Utc}`):

```rust
    // helper: a check that started at `start`, last completed at `last_ping`,
    // with an optional max runtime.
    fn running_check(max_runtime: Option<i64>, start: DateTime<Utc>, last_ping: Option<DateTime<Utc>>) -> Check {
        let mut c = base_check();
        c.max_runtime_secs = max_runtime;
        c.last_start_at = Some(start);
        c.last_ping_at = last_ping;
        c
    }

    #[test]
    fn overrun_when_in_flight_past_max_runtime() {
        let start = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();
        // started, never completed (last_ping older than start), max 60s
        let c = running_check(Some(60), start, Some(Utc.with_ymd_and_hms(2026, 7, 12, 11, 0, 0).unwrap()));
        // deadline = 12:00:60
        assert_eq!(overrun_time(&c), Some(start + Duration::seconds(60)));
    }

    #[test]
    fn no_overrun_when_completed_after_start() {
        let start = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();
        // a success ping landed AFTER the start → run finished, not in flight
        let c = running_check(Some(60), start, Some(Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 30).unwrap()));
        assert_eq!(overrun_time(&c), None);
    }

    #[test]
    fn no_overrun_without_max_runtime_or_start() {
        let start = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();
        assert_eq!(overrun_time(&running_check(None, start, None)), None);
        assert_eq!(overrun_time(&running_check(Some(0), start, None)), None); // non-positive off
        let mut no_start = base_check();
        no_start.max_runtime_secs = Some(60);
        no_start.last_start_at = None;
        assert_eq!(overrun_time(&no_start), None);
    }

    #[tokio::test]
    async fn scan_once_downs_overrun_check() {
        use crate::db;
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool, "sqlite::memory:").await.unwrap();
        let store = Store::new(pool);
        // user+project+check
        store.create_user("u", Some("x"), false, Utc::now()).await.unwrap();
        store.create_project(1, "p", None, Utc::now()).await.unwrap();
        let start = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();
        let cid = store
            .create_check(1, "job", "u1", ScheduleKind::Period, Some(3_600_000), 300, None, "UTC")
            .await
            .unwrap();
        // long period so it is NOT overdue; set an in-flight start + short max runtime
        store.update_check_schedule(cid, "job", ScheduleKind::Period, Some(3_600_000), 300, None, "UTC", None, Some(60)).await.unwrap();
        store.mark_ping(cid, CheckStatus::Up, None, Some(start), None).await.unwrap();

        // now = start + 61s → past the 60s max runtime
        let now = start + Duration::seconds(61);
        let events = scan_once(&store, now).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, EventKind::Down);
        assert_eq!(store.find_check(cid).await.unwrap().unwrap().status, CheckStatus::Down);
    }
```

Note: adjust `create_check`/`update_check_schedule`/`mark_ping` argument lists to the real signatures (Task 1 added `max_runtime_secs` to `update_check_schedule`). The period `3_600_000` seconds keeps the check from being overdue so the test isolates the overrun path.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run --lib scheduler::tests::overrun scheduler::tests::scan_once_downs_overrun scheduler::tests::no_overrun`
Expected: FAIL — `overrun_time` does not exist yet.

- [ ] **Step 3: Implement `overrun_time` + extend `scan_once`**

In `src/scheduler.rs`, add the helper (near `due_time`):

```rust
/// The instant at/after which an in-flight run is considered overrun, or
/// `None` if overrun detection does not apply. A run is in flight when the
/// check has a `max_runtime_secs > 0`, a `last_start_at`, and that start is
/// newer than the last completion (`last_ping_at`) — i.e. a `start` ping
/// arrived without a subsequent success/fail. The deadline is
/// `last_start_at + max_runtime_secs`.
pub fn overrun_time(check: &Check) -> Option<DateTime<Utc>> {
    let max = check.max_runtime_secs?;
    if max <= 0 {
        return None;
    }
    let start = check.last_start_at?;
    let in_flight = check.last_ping_at.map_or(true, |done| start > done);
    if !in_flight {
        return None;
    }
    Some(start + Duration::seconds(max))
}
```

Then extend `scan_once` so a check is downed when it is either overdue OR overrun (compute both, down once, emit one `Down` event). Replace the per-check body:

```rust
    for check in store.list_active_checks().await? {
        let overdue = due_time(&check).is_some_and(|due| now >= due);
        let overrun = overrun_time(&check).is_some_and(|deadline| now >= deadline);
        if !(overdue || overrun) {
            continue;
        }
        if let Err(e) = store.set_status(check.id, CheckStatus::Down).await {
            tracing::error!("failed to down check {}: {e}", check.id);
            continue;
        }
        events.push(NotificationEvent {
            check_id: check.id,
            check_name: check.name.clone(),
            event: EventKind::Down,
            at: now,
            project_id: check.project_id,
        });
    }
```

(Downing is idempotent: `list_active_checks` returns only `new`/`up` checks, so once downed a check leaves the scan set and will not re-emit — the same guarantee the overdue path already relies on. Recovery is unchanged: a later success ping fires the existing Down→Up event.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --lib scheduler::tests`
Expected: PASS — the new overrun tests plus all existing scheduler tests (overdue behaviour unchanged).

- [ ] **Step 5: Full suite + fmt + clippy + commit**

Run: `cargo nextest run` → full suite green. Then `cargo fmt` and `cargo clippy --all-targets -- -D warnings` clean.

```bash
git add src/scheduler.rs
git commit -m "feat: down in-flight checks that exceed max_runtime_secs"
```

---

## Self-Review

**1. Spec coverage (spec §12 "Started but never finished detection using `start` pings + max-runtime"):**
- Uses the existing `start` ping (`last_start_at`, already recorded, previously unused for detection). ✅
- Per-check max runtime — `max_runtime_secs`, opt-in, NULL = off. ✅ (Task 1)
- Detects and notifies — `scan_once` downs the in-flight-overrun check and emits a `Down` event through the existing per-check channel delivery; recovery (Down→success→Up) is automatic. ✅ (Task 2)

**2. Placeholder scan:** every code step is complete — both migrations, the model field, the `row_to_check` line, the full `update_check_schedule`, the form field, the full `overrun_time`, the `scan_once` body, and all tests. The only rule-based steps are "populate `max_runtime_secs` at every `CheckFormTemplate` site" and "add `max_runtime_secs: None` to every `Check` literal" — both are compiler-enumerated (the build fails until each site is handled), and the specific values to use are given.

**3. Type consistency:**
- `max_runtime_secs: Option<i64>` in the model ↔ `INTEGER`/`BIGINT` column ↔ `row.get::<i64,_>` (via `row.get("max_runtime_secs")` inferring `Option<i64>`). ✅
- `update_check_schedule`'s new trailing `Option<i64>` param ↔ both web call sites pass `parse_opt_i64(&form.max_runtime_secs)` (returns `Option<i64>`). ✅
- `overrun_time(&Check) -> Option<DateTime<Utc>>` produced in Task 2, consumed by `scan_once` in the same task. ✅
- `$N` placeholders renumbered correctly in `update_check_schedule` (8 binds → 9, `id` last as `$9`). ✅
- Detection reuses `EventKind::Down` + `NotificationEvent` — no new event type, so channel delivery and recovery are unchanged. ✅
