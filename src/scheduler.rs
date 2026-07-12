use crate::models::{Check, CheckStatus, ScheduleKind};
use crate::notify::{dispatch, EventKind, NotificationEvent, Notifier};
use crate::store::Store;
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::{interval, Duration as TokioDuration};

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

/// Scans every active check (status `new`/`up`), transitioning any whose
/// `due_time` has passed to `down`. Per-check failures (e.g. a DB error on
/// `set_status`) are logged and skipped rather than aborting the round.
pub async fn scan_once(
    store: &Store,
    now: DateTime<Utc>,
) -> Result<Vec<NotificationEvent>, sqlx::Error> {
    let mut events = Vec::new();
    for check in store.list_active_checks().await? {
        let Some(due) = due_time(&check) else {
            continue;
        };
        if now >= due {
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

/// Runs the scan loop forever: on every tick, scans for overdue checks and
/// dispatches any resulting events to `notifiers`. `Utc::now()` is called
/// here (and only here) so `scan_once` itself stays deterministic and
/// testable with an injected `now`.
///
/// Plan 1 bound: `notifiers` is a single, global set loaded once at startup
/// (see `PINGWARD_WEBHOOK_URL` in `main.rs`); per-check channel binding
/// arrives in Plan 2.
pub async fn run_scan_loop(
    store: Store,
    interval_secs: u64,
    notifiers: Arc<Vec<Box<dyn Notifier>>>,
) {
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
}
