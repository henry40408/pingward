# pingward nag (repeat down-notifications) — Design

**Status:** Approved (2026-07-12)
**Feature:** Repeat ("nag") notifications for checks that stay `down`, until they recover or are acknowledged.

## Goal

Today pingward notifies once on each state transition (`up`→`down`, `down`→`up`).
A check that stays down produces a single alert that is easy to miss. nag adds
**opt-in, interval-based repeat notifications** while a check remains down, with
a manual **acknowledge** to silence an ongoing incident without recovering it.

## Prior discussion / industry alignment

Interval-based repeat alerting is the canonical pattern: Nagios
`notification_interval`, Prometheus Alertmanager `repeat_interval` (default 4h),
Grafana repeat interval. Healthchecks.io keeps it opt-in (hourly/daily reminders,
Pushover "Emergency" re-alert). pingward follows the same shape: **opt-in,
configurable interval, stoppable** (by recovery, pause, or acknowledge).

## Decisions (locked)

1. **Interval configuration = scan cascade.** `nag_interval_secs` resolves
   per-check → per-project → global (`settings` key `nag_interval`), mirroring
   the existing `scan_interval_secs` cascade. Difference: **no env fallback** —
   nag is opt-in, so when every level is unset (or ≤ 0) nag is **off**.
2. **Stop conditions = recovery, pause, and manual acknowledge.** Recovery
   (`down`→`up`) and pause halt nag inherently (a paused/up check is not
   nagged). Additionally a per-incident **acknowledge** silences reminders
   while the check is still down.
3. **Repeat notifications are a distinct `Reminder` event.** A new
   `EventKind::Reminder` (`"reminder"`) separates repeats from the first `Down`
   alert in the `notifications` history and lets channels signal them
   distinctly.

## State model

New columns:

| Table    | Column               | Type (sqlite / postgres) | Meaning |
|----------|----------------------|--------------------------|---------|
| `checks` | `nag_interval_secs`  | `INTEGER` / `BIGINT`, NULL | per-check nag interval (innermost cascade level); NULL/≤0 = inherit/off |
| `checks` | `last_alert_at`      | `TEXT`, NULL             | timestamp of the last `down`/`reminder` alert scheduled; baseline for the next nag |
| `checks` | `acknowledged`       | `INTEGER`/`BIGINT` NOT NULL DEFAULT 0 | 1 = user silenced the current down incident |
| `projects` | `nag_interval_secs` | `INTEGER` / `BIGINT`, NULL | per-project nag interval (middle cascade level) |

**Why columns, not derived from `notifications`:** deriving next-nag time via
`SELECT MAX(created_at) ... WHERE event IN (...)` is polluted by delivery
failures (failed deliveries are also written to `notifications`) and couples
nag timing to delivery. `last_alert_at` is written by the scan loop at the
moment an alert is *scheduled*, keeping nag timing clean and delivery-decoupled.

## Lifecycle

| Trigger | Effect |
|---|---|
| Transition to `down` (scan `overdue`/`overrun`, or a `fail` ping from `up`/`new`) | `status=down`, `last_alert_at=now`, `acknowledged=0`; emit `Down` |
| `nag_once` each tick, per down check | if not acknowledged **and** `now ≥ last_alert_at + effective_interval` → `last_alert_at=now`; emit `Reminder` |
| User clicks **Acknowledge** (down check) | `acknowledged=1` |
| Recovery: `success` ping, `down`→`up` | `acknowledged=0`, `last_alert_at=NULL`; emit `Up` (unchanged behaviour) |

`acknowledged` cannot be manually cleared; it is reset automatically on the
next recovery and on every fresh down transition, so acknowledgement silences
exactly one incident. (Manual un-acknowledge is an explicit non-goal — optional
follow-up.)

## Cascade resolver

New pure function, parallel to `effective_scan_interval`:

```rust
/// Resolve the effective nag interval for a check from the cascade
/// (check → project → global). Returns None when nag is off at every level
/// or the resolved value is not positive. Unlike scan interval, there is no
/// env-default fallback: nag is opt-in.
pub fn effective_nag_interval(
    check: Option<i64>,
    project: Option<i64>,
    global: Option<i64>,
) -> Option<i64> {
    [check, project, global]
        .into_iter()
        .flatten()
        .find(|&v| v > 0)
}
```

Global value comes from `settings` key `nag_interval` (same mechanism as the
global `scan_interval`).

## Scan loop integration

New function symmetric to `scan_once`, with `now` injected for determinism:

```rust
/// Emit a Reminder event for every down, un-acknowledged check whose nag
/// interval has elapsed since its last alert. Updates last_alert_at for each
/// reminded check so the next reminder is one interval later.
pub async fn nag_once(
    store: &Store,
    now: DateTime<Utc>,
) -> Result<Vec<NotificationEvent>, sqlx::Error>;
```

`nag_once` loads `list_down_checks()`, `all_project_nag_intervals()`, and the
global `nag_interval` setting; for each down check it resolves the cascade,
skips acknowledged / off / no-baseline checks, and for due checks writes
`last_alert_at=now` and pushes a `Reminder` event.

`run_scan_loop` calls `nag_once` after `scan_once` each tick and spawns
deliveries identically. Reminder cadence is therefore bounded below by the loop
tick interval (the scan cascade minimum); a `nag_interval` shorter than the tick
cannot fire faster than the tick. This is documented behaviour, not a bug.

## Notification rendering

`EventKind::Reminder`:
- `as_str()` → `"reminder"`, `FromStr` accepts `"reminder"`.
- `event_text` → `"🔴 {name} is STILL DOWN (as of {at})"` (red, like `Down`).
- ntfy: priority `high`, tag `red_circle` (same as `Down`).
- pushover: priority `1` (same as `Down`).
- `event_title` → `"pingward: {name} reminder"`.

## Store methods

- `list_down_checks() -> Vec<Check>` — checks with `status='down'`.
- `all_project_nag_intervals() -> HashMap<i64, Option<i64>>` — mirrors
  `all_project_scan_intervals`.
- `begin_down_alert(check_id, at)` — `UPDATE checks SET last_alert_at=$at,
  acknowledged=0 WHERE id=$id`; called right after a Down event is emitted
  (scan_once and the ping `fail` branch).
- `record_reminder(check_id, at)` — `UPDATE checks SET last_alert_at=$at WHERE
  id=$id`; called by `nag_once` per reminder.
- `clear_nag(check_id)` — `UPDATE checks SET acknowledged=0, last_alert_at=NULL
  WHERE id=$id`; called on recovery.
- `acknowledge(check_id)` — `UPDATE checks SET acknowledged=1 WHERE id=$id`;
  called by the ack endpoint.
- `update_check_schedule` gains a trailing `nag_interval_secs: Option<i64>`
  parameter; the project create/update path gains `nag_interval_secs`.
- `row_to_check` / `row_to_project` read the new columns.

## Web surface

- **Check form** (`templates/check_form.html`, `CheckForm`,
  `CheckFormTemplate`): add `nag_interval_secs` input (mirrors
  `max_runtime_secs`; empty = inherit). Both `update_check_schedule` call sites
  pass `parse_opt_i64(&form.nag_interval_secs)`.
- **Project form**: add `nag_interval_secs` input.
- **Settings page**: add global `nag_interval` field (mirrors `scan_interval`).
- **Check detail page**: when `status == down`, show an **Acknowledge** button
  → `POST /checks/:id/ack`, owner/admin authorization (mirrors pause/unpause).
  Acknowledged incidents may show an "acknowledged" marker.

## Migration `0004_nag.sql` (both backends)

1. `ALTER TABLE checks ADD COLUMN nag_interval_secs INTEGER|BIGINT`
2. `ALTER TABLE checks ADD COLUMN last_alert_at TEXT`
3. `ALTER TABLE checks ADD COLUMN acknowledged INTEGER|BIGINT NOT NULL DEFAULT 0`
4. `ALTER TABLE projects ADD COLUMN nag_interval_secs INTEGER|BIGINT`
5. Widen `notifications.event` CHECK to `IN ('down','up','reminder')`:
   - **SQLite:** rebuild the `notifications` table (create `notifications_new`
     with the new CHECK, `INSERT ... SELECT`, `DROP TABLE notifications`,
     `ALTER TABLE notifications_new RENAME TO notifications`). Nothing
     references `notifications`, so the drop is safe.
   - **Postgres:** `ALTER TABLE notifications DROP CONSTRAINT
     notifications_event_check;` then `ALTER TABLE notifications ADD CONSTRAINT
     notifications_event_check CHECK (event IN ('down','up','reminder'));`

## Testing

- Unit: `effective_nag_interval` cascade (check/project/global precedence, off
  when all unset/≤0).
- Unit/integration (sqlite store): `nag_once` — reminds a due down check, skips
  acknowledged, skips off (no interval), skips not-yet-due, advances
  `last_alert_at`; recovery clears nag state; fresh down resets acknowledged.
- Web: acknowledge endpoint persists + authorization negative path; check/project
  forms persist `nag_interval_secs`; settings global persists.
- Postgres parity: extend `tests/pg_store.rs` round-trip to cover the new columns
  and a nag cycle.

## Non-goals (optional follow-ups)

- Manual un-acknowledge.
- Per-channel nag opt-out.
- Escalation (different channel after N reminders).
- Bounding maximum number of reminders per incident.
