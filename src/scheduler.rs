use crate::config::effective_scan_interval;
use crate::models::{Check, CheckStatus, ScheduleKind};
use crate::notify::{deliver_event, EventKind, NotificationEvent, RetryPolicy};
use crate::store::Store;
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use std::str::FromStr;
use tokio::time::{sleep, Duration as TokioDuration};

/// Anchor for the next expected check-in: last successful ping, else creation.
fn anchor(check: &Check) -> DateTime<Utc> {
    check.last_ping_at.unwrap_or(check.created_at)
}

/// The instant at/after which `check` is overdue, or `None` if it cannot be
/// computed (e.g. a period check with no `period_secs`, or an invalid cron
/// expression).
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
            let tz: Tz = check.timezone.parse().unwrap_or_else(|_| {
                tracing::warn!(
                    check_id = check.id,
                    timezone = %check.timezone,
                    "invalid timezone on check, falling back to UTC"
                );
                chrono_tz::UTC
            });
            let anchor_local = anchor(check).with_timezone(&tz);
            let next = schedule.after(&anchor_local).next()?;
            Some(next.with_timezone(&Utc) + grace)
        }
    }
}

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
    let in_flight = check.last_ping_at.is_none_or(|done| start > done);
    if !in_flight {
        return None;
    }
    Some(start + Duration::seconds(max))
}

/// Scans every active check (status `new`/`up`), transitioning any whose
/// `due_time` has passed, or whose in-flight run has exceeded
/// `max_runtime_secs`, to `down`. Per-check failures (e.g. a DB error on
/// `set_status`) are logged and skipped rather than aborting the round.
pub async fn scan_once(
    store: &Store,
    now: DateTime<Utc>,
) -> Result<Vec<NotificationEvent>, sqlx::Error> {
    let mut events = Vec::new();
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
    Ok(events)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Check, CheckStatus, ScheduleKind};
    use chrono::{TimeZone, Utc};

    fn base_check() -> Check {
        Check {
            id: 1,
            project_id: 1,
            name: "j".into(),
            ping_uuid: "u".into(),
            schedule_kind: ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            cron_expr: None,
            timezone: "UTC".into(),
            status: CheckStatus::Up,
            last_ping_at: Some(Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap()),
            last_start_at: None,
            next_due_at: None,
            scan_interval_secs: None,
            max_runtime_secs: None,
            created_at: Utc.with_ymd_and_hms(2026, 7, 12, 11, 0, 0).unwrap(),
        }
    }

    #[test]
    fn period_due_is_last_ping_plus_period_plus_grace() {
        let c = base_check();
        // 12:00 + 3600s + 300s = 13:05
        assert_eq!(
            due_time(&c).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 12, 13, 5, 0).unwrap()
        );
    }

    #[test]
    fn cron_due_is_next_trigger_plus_grace() {
        let mut c = base_check();
        c.schedule_kind = ScheduleKind::Cron;
        c.period_secs = None;
        c.cron_expr = Some("0 0 * * * *".into()); // top of every hour (sec min hour ...)
                                                  // last_ping 12:00 → next trigger 13:00 + 300s grace = 13:05
        assert_eq!(
            due_time(&c).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 12, 13, 5, 0).unwrap()
        );
    }

    #[test]
    fn period_without_period_secs_is_none() {
        let mut c = base_check();
        c.period_secs = None;
        assert!(due_time(&c).is_none());
    }

    #[test]
    fn first_run_anchor_is_created_at() {
        let mut c = base_check();
        c.last_ping_at = None;
        c.created_at = Utc.with_ymd_and_hms(2026, 7, 12, 9, 0, 0).unwrap();
        c.period_secs = Some(1800);
        c.grace_secs = 60;
        // created_at 09:00 + 1800s + 60s = 09:31:00
        assert_eq!(
            due_time(&c).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 12, 9, 31, 0).unwrap()
        );
    }

    #[test]
    fn cron_invalid_timezone_falls_back_to_utc() {
        let mut c = base_check();
        c.schedule_kind = ScheduleKind::Cron;
        c.period_secs = None;
        c.cron_expr = Some("0 0 * * * *".into());
        c.timezone = "Not/AZone".into();

        let mut expected = base_check();
        expected.schedule_kind = ScheduleKind::Cron;
        expected.period_secs = None;
        expected.cron_expr = Some("0 0 * * * *".into());
        expected.timezone = "UTC".into();

        assert_eq!(due_time(&c).unwrap(), due_time(&expected).unwrap());
        // last_ping 12:00 UTC → next trigger 13:00 UTC + 300s grace = 13:05 UTC
        assert_eq!(
            due_time(&c).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 12, 13, 5, 0).unwrap()
        );
    }

    #[test]
    fn cron_without_cron_expr_is_none() {
        let mut c = base_check();
        c.schedule_kind = ScheduleKind::Cron;
        c.period_secs = None;
        c.cron_expr = None;
        assert!(due_time(&c).is_none());
    }

    #[test]
    fn cron_with_malformed_expr_is_none() {
        let mut c = base_check();
        c.schedule_kind = ScheduleKind::Cron;
        c.period_secs = None;
        c.cron_expr = Some("not a cron".into());
        assert!(due_time(&c).is_none());
    }

    #[test]
    fn loop_interval_picks_min_effective_across_checks() {
        use std::collections::HashMap;
        // check 1 in project 1: no project override → its own 50 applies.
        let mut c1 = base_check();
        c1.id = 1;
        c1.project_id = 1;
        c1.scan_interval_secs = Some(50);
        // check 2 in project 2: no check override → project override 10 applies.
        let mut c2 = base_check();
        c2.id = 2;
        c2.project_id = 2;
        c2.scan_interval_secs = None;

        let mut intervals = HashMap::new();
        intervals.insert(1, None);
        intervals.insert(2, Some(10));

        // effective: check1=50, check2=10 → the loop ticks at the minimum, 10.
        assert_eq!(loop_interval_secs(&[c1, c2], &intervals, Some(30), 30), 10);
    }

    #[test]
    fn loop_interval_empty_falls_back_to_env_default() {
        use std::collections::HashMap;
        let intervals = HashMap::new();
        assert_eq!(loop_interval_secs(&[], &intervals, None, 30), 30);
        // env default of 0 is clamped to at least 1 so the timer stays valid.
        assert_eq!(loop_interval_secs(&[], &intervals, None, 0), 1);
    }

    // helper: a check that started at `start`, last completed at `last_ping`,
    // with an optional max runtime.
    fn running_check(
        max_runtime: Option<i64>,
        start: DateTime<Utc>,
        last_ping: Option<DateTime<Utc>>,
    ) -> Check {
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
        let c = running_check(
            Some(60),
            start,
            Some(Utc.with_ymd_and_hms(2026, 7, 12, 11, 0, 0).unwrap()),
        );
        // deadline = 12:00:60
        assert_eq!(overrun_time(&c), Some(start + Duration::seconds(60)));
    }

    #[test]
    fn no_overrun_when_completed_after_start() {
        let start = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();
        // a success ping landed AFTER the start → run finished, not in flight
        let c = running_check(
            Some(60),
            start,
            Some(Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 30).unwrap()),
        );
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
        store
            .create_user("u", Some("x"), false, Utc::now())
            .await
            .unwrap();
        store
            .create_project(1, "p", None, Utc::now())
            .await
            .unwrap();
        let start = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();
        let cid = store
            .create_check(
                1,
                "job",
                "u1",
                ScheduleKind::Period,
                Some(3_600_000),
                300,
                None,
                "UTC",
            )
            .await
            .unwrap();
        // long period so it is NOT overdue; set an in-flight start + short max runtime
        store
            .update_check_schedule(
                cid,
                "job",
                ScheduleKind::Period,
                Some(3_600_000),
                300,
                None,
                "UTC",
                None,
                Some(60),
            )
            .await
            .unwrap();
        store
            .mark_ping(cid, CheckStatus::Up, None, Some(start), None)
            .await
            .unwrap();

        // now = start + 61s → past the 60s max runtime
        let now = start + Duration::seconds(61);
        let events = scan_once(&store, now).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, EventKind::Down);
        assert_eq!(
            store.find_check(cid).await.unwrap().unwrap().status,
            CheckStatus::Down
        );
    }
}
