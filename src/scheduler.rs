use crate::models::{Check, ScheduleKind};
use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use std::str::FromStr;

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
            let tz: Tz = check.timezone.parse().unwrap_or(chrono_tz::UTC);
            let anchor_local = anchor(check).with_timezone(&tz);
            let next = schedule.after(&anchor_local).next()?;
            Some(next.with_timezone(&Utc) + grace)
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
}
