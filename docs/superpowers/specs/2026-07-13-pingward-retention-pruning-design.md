# pingward retention / pruning — Design

**Status:** Approved (2026-07-13)
**Feature:** Age-based background pruning of `pings` and `notifications` history.

## Goal

`pings` and `notifications` grow without bound — a row per ping and per delivery
attempt — degrading the check-detail page and the database over time (the nag
feature added more notification rows via reminders). Add opt-in, age-based
retention: a background task deletes rows older than a configurable number of
days.

## Safety analysis

Monitoring state is denormalized onto `checks` (`last_ping_at`, `last_start_at`,
`status`, `next_due_at`); the scheduler and ping handler read those columns, not
the `pings`/`notifications` rows. Those two tables are consumed only by the
check-detail page (`list_recent_pings` / `list_recent_notifications`, 20 rows
each). Deleting old history rows therefore cannot affect monitoring correctness —
it only shortens the displayed history.

## Decisions (locked)

1. **Global-only configuration** via the `settings` table (pruning is an
   admin/operational concern, consistent with the admin-only settings page). No
   per-project/per-check cascade.
2. **Age-based (days).** Delete rows whose `created_at` is older than N days.
3. **Separate retention per table:** `pings_retention_days` and
   `notifications_retention_days` are independent settings.
4. **Dedicated periodic task** at a fixed interval (`PINGWARD_PRUNE_INTERVAL_SECS`,
   default 3600), decoupled from the scan loop.

Unset / blank / non-positive retention = **off** (keep forever) for that table.

**No migration / no schema change:** retention values live in the existing
`settings` key/value table; pruning uses the existing `created_at` columns.

## Timestamp comparison

All timestamps are stored as `DateTime<Utc>.to_rfc3339()` — i.e. UTC with a `Z`
(or `+00:00`) offset and fixed field widths — so lexicographic TEXT ordering
equals chronological ordering. `DELETE ... WHERE created_at < $cutoff` with
`cutoff = (now - Duration::days(N)).to_rfc3339()` is correct on both SQLite and
PostgreSQL as a TEXT comparison, matching the existing `find_session_user`
pattern (`expires_at > $2`). (All app writes use `to_rfc3339()` with UTC; there
is no mixed-offset data.)

## Components

### Config (`src/config.rs`)
Add `prune_interval_secs: u64` to `Config`, parsed from
`PINGWARD_PRUNE_INTERVAL_SECS` (default 3600, mirroring `PINGWARD_SCAN_INTERVAL`).

### Prune logic (`src/prune.rs`, new)
- `prune_once(store: &Store, now: DateTime<Utc>) -> Result<(u64, u64), sqlx::Error>`
  — `now` injected for determinism (mirrors `scan_once`). Reads the two retention
  settings; for each that parses to a positive integer, computes
  `cutoff = (now - Duration::days(n)).to_rfc3339()` and deletes older rows,
  returning `(pings_deleted, notifications_deleted)`. A table with unset/blank/≤0
  retention is skipped (its count is 0).
- `run_prune_loop(store: Store, interval_secs: u64)` —
  `loop { match prune_once(&store, Utc::now()).await { Ok((p,n)) => log if >0, Err(e) => log }; sleep(interval.max(1)) }`. Runs once at startup, then every interval.

### Store methods (`src/store.rs`)
- `delete_pings_before(&self, cutoff: &str) -> Result<u64, sqlx::Error>` —
  `DELETE FROM pings WHERE created_at < $1`, returns `rows_affected()`.
- `delete_notifications_before(&self, cutoff: &str) -> Result<u64, sqlx::Error>` —
  `DELETE FROM notifications WHERE created_at < $1`, returns `rows_affected()`.

### Wiring (`src/main.rs`)
`tokio::spawn(prune::run_prune_loop(store.clone(), config.prune_interval_secs));`
alongside the existing scan-loop spawn.

### Settings UI (`src/web.rs`, `templates/settings.html`)
Add `pings_retention_days` and `notifications_retention_days` fields to
`SettingsTemplate` / `SettingsForm`; `settings_page` loads them; `settings_save`
persists each with the same blank-clears / positive-int-only logic used for
`scan_interval` and `nag_interval`.

## Testing

- **`prune_once`** (sqlite in-memory): insert old + recent pings/notifications,
  set retention, assert old rows deleted / recent rows kept / returned counts
  correct; assert unset retention deletes nothing (returns 0); assert a `0`/blank
  value is treated as off.
- **Config:** `prune_interval_secs` default (3600) and env override.
- **Settings UI** (`tests/auth_web.rs`): the two retention fields persist; add the
  new field keys to existing settings-form POSTs (axum `Form` rejects missing
  non-Option fields).
- **Postgres parity** (`tests/pg_store.rs`): a prune round-trip on live PG.

## lib.rs

Register the new `pub mod prune;` (mirror the existing module declarations).

## Non-goals (optional follow-ups)

- Per-check "keep last N" retention.
- An index on `created_at` to speed the delete (hourly bulk delete on a
  self-hosted scale does not need it; note it if tables grow very large).
- Pruning `sessions` (already self-expiring) or `pings` body truncation.
