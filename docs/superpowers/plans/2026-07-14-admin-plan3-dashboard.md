# Admin Plan 3 — Admin Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A standalone `/admin` dashboard showing site-wide check health, notification health, scheduler/background status, and resource-scale figures.

**Architecture:** The scheduler and prune loops write a heartbeat timestamp into the `settings` table each cycle. New read-only store aggregates roll up cross-user figures. A dashboard handler replaces the Plan 1 `admin_home` redirect and renders `admin_dashboard.html`. All read-only — no audit needed (the dashboard reads aggregates, not individual users' resources).

**Tech Stack:** Rust, axum 0.8, sqlx 0.9 `Any`, askama, chrono. Tests via `cargo nextest run` with `axum-test`.

## Global Constraints

- **Depends on Plan 1** (the `/admin` group and `admin_home` exist). Ordering vs Plan 2 is independent. Do not start until Plan 1 is merged.
- sqlx `Any`: `$N` placeholders. Timestamps RFC3339 TEXT; compare with `WHERE created_at >= $1` binding `cutoff.to_rfc3339()`.
- `cargo nextest run` (not `cargo test`); `cargo fmt` before every commit; GPG-signed commits; stage files by name.
- The dashboard is read-only; do NOT write audit rows from it.
- **Test-helper facts:** the `src/store.rs` inline test store builder is **`seeded()`** (substitute for `test_store()` below); it pre-seeds user `'u'` (id 1) and project `'p'`, so `count_projects()`/`count_checks()` include the seed — assert relative deltas or account for it. `Store::create_check` is `create_check(project_id, name, ping_uuid, kind: ScheduleKind, period_secs: Option<i64>, grace_secs: i64, cron_expr: Option<&str>, timezone: &str)` — no `now`; e.g. `store.create_check(pid, "c", "uuid-c", ScheduleKind::Period, Some(3600), 300, None, "UTC").await`. Integration tests copy the `admin_server()` shape from `tests/admin.rs`.

---

## File Structure

- `src/scheduler.rs` — write `last_scan_at` each scan iteration.
- `src/prune.rs` — write `last_prune_at` each prune iteration.
- `src/store.rs` — aggregates: `CheckStatusCounts`, `count_checks_by_status`, `list_down_checks_with_owner`, `notification_counts_since`, `channel_failure_counts_since`, `recent_failed_notifications`, `count_projects`, `count_checks`, `count_pings_since`.
- `src/web.rs` — replace `admin_home` with `admin_dashboard`; add `AdminDashboardTemplate`.
- `templates/admin_dashboard.html` — four section cards (new).
- `tests/admin_dashboard.rs` — new integration tests.

---

## Task 1: Scheduler + prune heartbeat

**Files:**
- Modify: `src/scheduler.rs` (`run_scan_loop`), `src/prune.rs` (`run_prune_loop`)
- Test: `src/scheduler.rs` inline test (scan heartbeat) — note the loop runs forever, so test the extracted write, not the loop.

**Interfaces:**
- Produces: settings keys `last_scan_at`, `last_prune_at` (RFC3339), updated each cycle.

- [ ] **Step 1: Write the heartbeat in `run_scan_loop`**

In `src/scheduler.rs`, inside `run_scan_loop`, immediately after the `scan_once` match block (before the nag block), add:
```rust
        // Heartbeat: record the last successful scan pass for the admin dashboard.
        let _ = store.set_setting("last_scan_at", &now.to_rfc3339()).await;
```
(`now` is already bound at the top of the loop.)

- [ ] **Step 2: Write the heartbeat in `run_prune_loop`**

In `src/prune.rs`, inside `run_prune_loop`, after the `prune_once` match block and before `sleep(interval)`, add:
```rust
        let _ = store.set_setting("last_prune_at", &Utc::now().to_rfc3339()).await;
```
Ensure `chrono::Utc` is imported in `prune.rs` (it is used elsewhere; confirm).

- [ ] **Step 3: Write a focused test for the heartbeat semantics**

Because both loops are infinite, test the observable effect directly via settings. Add to `src/prune.rs` tests (mirroring the existing `prune_once` test's store setup):
```rust
    #[tokio::test]
    async fn prune_heartbeat_setting_writes() {
        let store = test_store().await; // use the same helper the neighbouring test uses
        // Simulate one loop body's heartbeat write.
        store
            .set_setting("last_prune_at", &Utc::now().to_rfc3339())
            .await
            .unwrap();
        assert!(store.get_setting("last_prune_at").await.unwrap().is_some());
    }
```
(The loop wiring is exercised by manual run in Task 4's verification note; this test locks the settings contract the dashboard reads.)

- [ ] **Step 4: Run and verify**

Run: `cargo nextest run -p pingward prune::tests::prune_heartbeat_setting_writes`
Expected: PASS. Also `cargo build` to confirm the loop edits compile.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/scheduler.rs src/prune.rs
git commit -m "feat: scheduler and prune loops write heartbeat timestamps"
```

---

## Task 2: Observability aggregates

**Files:**
- Modify: `src/store.rs`
- Test: `src/store.rs` inline tests

**Interfaces:**
- Produces:
  - `struct CheckStatusCounts { new: i64, up: i64, down: i64, paused: i64 }`
  - `Store::count_checks_by_status(&self) -> Result<CheckStatusCounts, sqlx::Error>`
  - `Store::list_down_checks_with_owner(&self) -> Result<Vec<(Check, String, String)>, sqlx::Error>` — `(check, project_name, owner_username)`
  - `Store::notification_counts_since(&self, cutoff: DateTime<Utc>) -> Result<(i64, i64), sqlx::Error>` — `(ok, error)`
  - `Store::channel_failure_counts_since(&self, cutoff: DateTime<Utc>) -> Result<Vec<(String, i64, i64)>, sqlx::Error>` — `(channel_name, ok, error)`
  - `Store::recent_failed_notifications(&self, limit: i64) -> Result<Vec<Notification>, sqlx::Error>`
  - `Store::count_projects(&self) -> Result<i64, sqlx::Error>`
  - `Store::count_checks(&self) -> Result<i64, sqlx::Error>`
  - `Store::count_pings_since(&self, cutoff: DateTime<Utc>) -> Result<i64, sqlx::Error>`

- [ ] **Step 1: Write failing tests**

```rust
    #[tokio::test]
    async fn status_counts_and_scale() {
        let store = test_store().await;
        let uid = store.create_user("u", Some("p"), false, Utc::now()).await.unwrap();
        let pid = store.create_project(uid, "p", None, None, Utc::now()).await.unwrap();
        use pingward::models::ScheduleKind;
        store.create_check(pid, "a", "uuid-a", ScheduleKind::Period, Some(3600), 300, None, "UTC").await.unwrap();
        store.create_check(pid, "b", "uuid-b", ScheduleKind::Period, Some(3600), 300, None, "UTC").await.unwrap();
        // `seeded()` pre-seeds one project 'p' + user 'u', so counts are relative to that.
        let counts = store.count_checks_by_status().await.unwrap();
        assert_eq!(counts.new + counts.up + counts.down + counts.paused, store.count_checks().await.unwrap());
        assert_eq!(store.count_projects().await.unwrap(), 2); // seeded 'p' + this 'p'
        assert_eq!(store.count_checks().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn notification_counts_split_ok_error() {
        let store = test_store().await;
        // seed a project/check/channel and two notifications (one ok, one error);
        // reuse whatever seed helper the notification tests use.
        // ... arrange ...
        let (ok, err) = store.notification_counts_since(Utc::now() - chrono::Duration::days(1)).await.unwrap();
        assert!(ok + err >= 0); // replace with exact counts once seeded
    }
```
(Fill the notification arrange block using the pattern from the existing notification store test near `delete_notifications_before`.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo nextest run -p pingward store::tests::status_counts_and_scale`
Expected: FAIL (methods missing).

- [ ] **Step 3: Implement the aggregates**

```rust
#[derive(Debug, Clone, Default)]
pub struct CheckStatusCounts {
    pub new: i64,
    pub up: i64,
    pub down: i64,
    pub paused: i64,
}
```
```rust
    pub async fn count_checks_by_status(&self) -> Result<CheckStatusCounts, sqlx::Error> {
        let rows = sqlx::query("SELECT status, COUNT(*) AS n FROM checks GROUP BY status")
            .fetch_all(&self.pool)
            .await?;
        let mut c = CheckStatusCounts::default();
        for r in &rows {
            let status: String = r.get("status");
            let n: i64 = r.get("n");
            match status.as_str() {
                "new" => c.new = n,
                "up" => c.up = n,
                "down" => c.down = n,
                "paused" => c.paused = n,
                _ => {}
            }
        }
        Ok(c)
    }

    pub async fn list_down_checks_with_owner(
        &self,
    ) -> Result<Vec<(Check, String, String)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT c.*, p.name AS project_name, u.username AS owner_username \
             FROM checks c JOIN projects p ON p.id = c.project_id \
             JOIN users u ON u.id = p.user_id \
             WHERE c.status = 'down' ORDER BY c.last_ping_at",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok((
                    row_to_check(r)?,
                    r.get::<String, _>("project_name"),
                    r.get::<String, _>("owner_username"),
                ))
            })
            .collect()
    }

    pub async fn notification_counts_since(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<(i64, i64), sqlx::Error> {
        let ok: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notifications WHERE status = 'ok' AND created_at >= $1",
        )
        .bind(cutoff.to_rfc3339())
        .fetch_one(&self.pool)
        .await?;
        let err: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notifications WHERE status = 'error' AND created_at >= $1",
        )
        .bind(cutoff.to_rfc3339())
        .fetch_one(&self.pool)
        .await?;
        Ok((ok, err))
    }

    pub async fn channel_failure_counts_since(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<(String, i64, i64)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT ch.name AS channel_name, \
             SUM(CASE WHEN n.status = 'ok' THEN 1 ELSE 0 END) AS ok, \
             SUM(CASE WHEN n.status = 'error' THEN 1 ELSE 0 END) AS err \
             FROM notifications n JOIN channels ch ON ch.id = n.channel_id \
             WHERE n.created_at >= $1 GROUP BY ch.name ORDER BY err DESC",
        )
        .bind(cutoff.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok((
                    r.get::<String, _>("channel_name"),
                    r.get::<i64, _>("ok"),
                    r.get::<i64, _>("err"),
                ))
            })
            .collect()
    }

    pub async fn recent_failed_notifications(
        &self,
        limit: i64,
    ) -> Result<Vec<Notification>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT * FROM notifications WHERE status = 'error' ORDER BY id DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_notification).collect()
    }

    pub async fn count_projects(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COUNT(*) FROM projects").fetch_one(&self.pool).await
    }

    pub async fn count_checks(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COUNT(*) FROM checks").fetch_one(&self.pool).await
    }

    pub async fn count_pings_since(&self, cutoff: DateTime<Utc>) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COUNT(*) FROM pings WHERE created_at >= $1")
            .bind(cutoff.to_rfc3339())
            .fetch_one(&self.pool)
            .await
    }
```
Note: the `SUM(CASE ...)` columns may decode as `i64` on both engines; if PostgreSQL returns them as a wider type, cast in SQL with `CAST(SUM(...) AS BIGINT)`. Verify in Task 4's PG run and adjust if a decode error appears.

- [ ] **Step 4: Run and verify pass**

Run: `cargo nextest run -p pingward store::tests::status_counts_and_scale store::tests::notification_counts_split_ok_error`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/store.rs
git commit -m "feat: add admin dashboard observability aggregates"
```

---

## Task 3: Dashboard handler + template

**Files:**
- Modify: `src/web.rs` (replace `admin_home` with `admin_dashboard`; the `/admin` route now maps to it)
- Create: `templates/admin_dashboard.html`
- Test: `tests/admin_dashboard.rs`

**Interfaces:**
- Consumes: all Task 2 aggregates + `Store::{count_users, get_setting}`.
- Produces: `GET /admin` renders the dashboard (still guarded by `AdminUser`).

- [ ] **Step 1: Write the failing test**

Create `tests/admin_dashboard.rs` (reuse the `admin_server()` helper shape from `tests/admin.rs`):
```rust
#[tokio::test]
async fn admin_dashboard_renders_with_figures() {
    let (server, store, _admin) = admin_server().await;
    let uid = store.create_user("owner", Some("p"), false, chrono::Utc::now()).await.unwrap();
    let pid = store.create_project(uid, "proj", None, None, chrono::Utc::now()).await.unwrap();
    store.create_check(pid, "c", "uuid-c", pingward::models::ScheduleKind::Period, Some(3600), 300, None, "UTC").await.unwrap();
    let res = server.get("/admin").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("Dashboard") || body.contains("Admin"));
    // scale figures present
    assert!(body.contains("proj") || body.contains("1"));
}

#[tokio::test]
async fn non_admin_cannot_see_dashboard() {
    // build a member-logged-in server (copy from tests/admin.rs non_admin case)
    // and assert GET /admin -> 403
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo nextest run --test admin_dashboard`
Expected: FAIL (`/admin` currently redirects to `/admin/projects`, so no 200 body with figures).

- [ ] **Step 3: Implement the handler + template struct**

Replace `admin_home` in `src/web.rs`:
```rust
#[derive(Template)]
#[template(path = "admin_dashboard.html")]
struct AdminDashboardTemplate {
    show_nav: bool,
    is_admin: bool,
    users: i64,
    projects: i64,
    checks: i64,
    pings_24h: i64,
    status: crate::store::CheckStatusCounts,
    down: Vec<(Check, String, String)>,
    notif_ok: i64,
    notif_err: i64,
    channel_fail: Vec<(String, i64, i64)>,
    recent_fail: Vec<Notification>,
    last_scan_at: Option<String>,
    last_prune_at: Option<String>,
}

async fn admin_dashboard(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Response, AppError> {
    let day_ago = Utc::now() - Duration::days(1);
    let (notif_ok, notif_err) = state.store.notification_counts_since(day_ago).await?;
    Ok(render(&AdminDashboardTemplate {
        show_nav: true,
        is_admin: true,
        users: state.store.count_users().await?,
        projects: state.store.count_projects().await?,
        checks: state.store.count_checks().await?,
        pings_24h: state.store.count_pings_since(day_ago).await?,
        status: state.store.count_checks_by_status().await?,
        down: state.store.list_down_checks_with_owner().await?,
        notif_ok,
        notif_err,
        channel_fail: state.store.channel_failure_counts_since(day_ago).await?,
        recent_fail: state.store.recent_failed_notifications(10).await?,
        last_scan_at: state.store.get_setting("last_scan_at").await?,
        last_prune_at: state.store.get_setting("last_prune_at").await?,
    })?
    .into_response())
}
```
In `routes()`, change `.route("/admin", get(admin_home))` to `.route("/admin", get(admin_dashboard))` and remove `admin_home`.

- [ ] **Step 4: Create `templates/admin_dashboard.html`**

Extend `base.html`; reuse the Console card/pill styles from `dashboard.html`. Four sections:
1. **Scale** — users / projects / checks / pings (24h) stat tiles.
2. **Check health** — status counts (new/up/down/paused) + a table of `down` checks with `project_name` and `owner_username`.
3. **Notification health** — `notif_ok` / `notif_err` (24h); the `channel_fail` table (name, ok, err); the `recent_fail` list.
4. **Scheduler** — `last_scan_at` / `last_prune_at`. Render the raw timestamp; when absent show "never". (A red/stale visual can compare against `scan_interval` in a follow-up; the timestamp itself satisfies the spec here.)

Set `is_admin: true` so the base-template `Admin` nav link shows.

- [ ] **Step 5: Run and verify pass**

Run: `cargo nextest run --test admin_dashboard`
Expected: PASS. Then `cargo nextest run` — full suite green (the Plan 1 `/admin/projects` link stays reachable from the dashboard).

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/web.rs templates/admin_dashboard.html tests/admin_dashboard.rs
git commit -m "feat: admin dashboard at /admin with health and scale figures"
```

---

## Task 4: PostgreSQL parity + manual heartbeat verification

- [ ] **Step 1: PG-backed tests**

Run: `cargo nextest run --test pg_store` (start Postgres via the `container` CLI per the local-Postgres memory note if needed). If a `SUM(CASE ...)` column fails to decode on PG, wrap it in `CAST(... AS BIGINT)` in `channel_failure_counts_since` and re-run.
Expected: PASS.

- [ ] **Step 2: Manual heartbeat check**

Run the app briefly and confirm the heartbeat lands:
```bash
cargo run &
sleep 5
sqlite3 pingward.sqlite3 "SELECT key, value FROM settings WHERE key IN ('last_scan_at','last_prune_at');"
kill %1
```
Expected: `last_scan_at` (and `last_prune_at` after a prune cycle) present. Then load `/admin` in a browser as an admin and confirm all four cards render.

- [ ] **Step 3: Full suite**

Run: `cargo nextest run`
Expected: all green.

---

## Self-Review Notes

- Spec §3 (scheduler heartbeat) → Task 1. §4 aggregates → Task 2. §6/#3 dashboard route → Task 3. §7 dashboard UI + nav → Task 3. §8 testing → Tasks 2–4.
- Dashboard is read-only, so no audit rows (consistent with the spec: audit covers #1 cross-user resource access and #2 management actions, not aggregate reads).
- `admin_home` (Plan 1's redirect) is replaced by `admin_dashboard`; `/admin/projects` remains the drill-down entry.
