use crate::models::{Check, CheckStatus, Ping, PingKind};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayStatus {
    New,
    Up,
    Running,
    Late,
    Down,
    Paused,
}

impl DisplayStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            DisplayStatus::New => "new",
            DisplayStatus::Up => "up",
            DisplayStatus::Running => "running",
            DisplayStatus::Late => "late",
            DisplayStatus::Down => "down",
            DisplayStatus::Paused => "paused",
        }
    }
}

/// A `Start` ping has been recorded more recently than the last finish
/// (success/fail). `store::mark_ping` stamps `last_ping_at`/`last_start_at`
/// with `COALESCE`, and `ping::apply` only ever passes `last_start_at` for a
/// `Start` ping and `last_ping_at` for a success/fail — a `Log` ping calls
/// neither, so it cannot clear this. Rust's `Option` ordering makes
/// `Some(_) > None` true and `None > None` false, so this one comparison
/// covers both "started and never finished" and "started again after the
/// last finish", with no separate `is_some()` check needed.
fn is_running(check: &Check) -> bool {
    check.last_start_at > check.last_ping_at
}

/// `next_due_at` already includes grace, so `next_due_at - grace` is the expected
/// run time. A stored-Up check inside `(expected, due]` is "running late".
///
/// Precedence is `Paused > Down > Running > Late > Up`: `Running` only
/// applies to a stored `Up` or `New` check, and beats `Late` because a
/// long-running job naturally drifts past its expected time while it is
/// legitimately still executing — showing `late` there would be a false
/// alarm. `Down`/`Paused` are unaffected by `is_running`, so a job that
/// starts again after a failed run still shows `down`, and an in-flight run
/// never masks an alert.
pub fn display_status(check: &Check, now: DateTime<Utc>) -> DisplayStatus {
    match check.status {
        CheckStatus::Down => DisplayStatus::Down,
        CheckStatus::Paused => DisplayStatus::Paused,
        CheckStatus::New => {
            if is_running(check) {
                return DisplayStatus::Running;
            }
            DisplayStatus::New
        }
        CheckStatus::Up => {
            if is_running(check) {
                return DisplayStatus::Running;
            }
            if let Some(due) = check.next_due_at {
                let expected = due - Duration::seconds(check.grace_secs);
                if now > expected && now <= due {
                    return DisplayStatus::Late;
                }
            }
            DisplayStatus::Up
        }
    }
}

fn is_finish(k: PingKind) -> bool {
    matches!(k, PingKind::Success | PingKind::Fail)
}

/// Pair each finish (success/fail) ping with the most recent preceding `start`.
/// Input may be newest- or oldest-first; normalized to chronological internally.
pub fn run_durations(pings: &[Ping]) -> HashMap<i64, i64> {
    let mut ordered: Vec<&Ping> = pings.iter().collect();
    ordered.sort_by_key(|p| (p.created_at, p.id));
    let mut out = HashMap::new();
    let mut pending_start: Option<DateTime<Utc>> = None;
    for p in ordered {
        match p.kind {
            PingKind::Start => pending_start = Some(p.created_at),
            k if is_finish(k) => {
                if let Some(s) = pending_start.take() {
                    let secs = (p.created_at - s).num_seconds();
                    if secs >= 0 {
                        out.insert(p.id, secs);
                    }
                }
            }
            _ => {} // log / exitcode-as-recorded: ignore
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub height: u32,
    pub class: &'static str,
    pub title: String,
}

const MAX_H: u32 = 26;
const MIN_H: u32 = 5;
const NONE_H: u32 = 16;
const HOT_FRACTION: f64 = 0.80;

/// Build the heartbeat strip: the last `n` runs (success/fail pings), height by
/// fraction of runtime budget used, colour by outcome. See spec §7.
#[allow(
    clippy::cast_sign_loss,
    reason = "`frac` is clamped to [0.0, 1.0] and MAX_H > 0, so the scaled height is non-negative"
)]
pub fn heartbeat(
    pings: &[Ping],
    max_runtime_secs: Option<i64>,
    paused: bool,
    n: usize,
) -> Vec<Bar> {
    if paused {
        return (0..n)
            .map(|_| Bar {
                height: MIN_H,
                class: "pausedbar",
                title: "paused".into(),
            })
            .collect();
    }
    let durations = run_durations(pings);
    // chronological runs = finish pings, oldest→newest, keep last n
    let mut runs: Vec<&Ping> = pings.iter().filter(|p| is_finish(p.kind)).collect();
    runs.sort_by_key(|p| (p.created_at, p.id));
    let start = runs.len().saturating_sub(n);
    let runs = &runs[start..];

    let measured: Vec<i64> = runs
        .iter()
        .filter_map(|p| durations.get(&p.id).copied())
        .collect();
    // Ceiling: explicit max_runtime, else window max — but the window fallback
    // needs >= 2 measured durations to be meaningful.
    let ceiling: Option<i64> = match max_runtime_secs {
        Some(m) if m > 0 => Some(m),
        _ => {
            if measured.len() >= 2 {
                measured.iter().copied().max()
            } else {
                None
            }
        }
    };

    runs.iter()
        .map(|p| {
            let dur = durations.get(&p.id).copied();
            let failed = p.kind == PingKind::Fail;
            match (dur, ceiling) {
                (Some(d), Some(c)) if c > 0 => {
                    let frac = (d as f64 / c as f64).clamp(0.0, 1.0);
                    let h = ((MAX_H as f64) * frac).round() as u32;
                    let height = h.clamp(MIN_H, MAX_H);
                    let class = if failed {
                        "bad"
                    } else if matches!(max_runtime_secs, Some(m) if m > 0 && (d as f64) >= HOT_FRACTION * m as f64) {
                        "hot"
                    } else {
                        ""
                    };
                    Bar {
                        height,
                        class,
                        title: format!("{} / {}", fmt_secs(d), fmt_secs(c)),
                    }
                }
                _ => {
                    let class = if failed { "bad" } else { "none" };
                    let height = if failed { MAX_H } else { NONE_H };
                    let title = if failed {
                        "failed".into()
                    } else if dur.is_some() {
                        // Duration was measured, but there's no ceiling (no
                        // explicit max_runtime_secs and <2 measured runs in
                        // the window) to render height as a fraction of.
                        "no runtime limit set".into()
                    } else {
                        "duration unknown".into()
                    };
                    Bar { height, class, title }
                }
            }
        })
        .collect()
}

pub fn fmt_secs(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    }
}

pub fn fmt_relative(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let s = (now - then).num_seconds().max(0);
    if s < 60 {
        format!("{s}s ago")
    } else if s < 3600 {
        format!("{}m ago", s / 60)
    } else if s < 86400 {
        format!("{}h ago", s / 3600)
    } else {
        format!("{}d ago", s / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Check, CheckStatus, Ping, PingKind, ScheduleKind};
    use chrono::{Duration, TimeZone, Utc};

    fn base_check() -> Check {
        Check {
            id: 1,
            project_id: 1,
            name: "c".into(),
            description: String::new(),
            ping_uuid: "u".into(),
            schedule_kind: ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            cron_expr: None,
            timezone: "UTC".into(),
            status: CheckStatus::Up,
            last_ping_at: None,
            last_start_at: None,
            next_due_at: None,
            scan_interval_secs: None,
            max_runtime_secs: None,
            nag_interval_secs: None,
            last_alert_at: None,
            acknowledged: false,
            created_at: Utc::now(),
        }
    }
    fn ping(id: i64, kind: PingKind, at: chrono::DateTime<Utc>) -> Ping {
        Ping {
            id,
            check_id: 1,
            kind,
            exit_code: None,
            body: String::new(),
            source_ip: None,
            created_at: at,
        }
    }

    #[test]
    fn up_in_grace_window_is_late() {
        let now = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let mut c = base_check();
        c.status = CheckStatus::Up;
        c.next_due_at = Some(now + Duration::seconds(120)); // due in 2m, grace 300 → expected was 3m ago
        assert_eq!(display_status(&c, now), DisplayStatus::Late);
    }

    #[test]
    fn up_before_expected_is_up() {
        let now = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let mut c = base_check();
        c.next_due_at = Some(now + Duration::seconds(3000)); // expected well in the future
        assert_eq!(display_status(&c, now), DisplayStatus::Up);
    }

    #[test]
    fn running_beats_late() {
        let now = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let mut c = base_check();
        c.status = CheckStatus::Up;
        c.next_due_at = Some(now + Duration::seconds(120)); // due in 2m, grace 300 → would be "late"
        c.last_ping_at = Some(now - Duration::seconds(4000));
        c.last_start_at = Some(now - Duration::seconds(10)); // started after the last finish
        assert_eq!(display_status(&c, now), DisplayStatus::Running);
    }

    #[test]
    fn running_from_new() {
        let now = Utc::now();
        let mut c = base_check();
        c.status = CheckStatus::New;
        c.last_start_at = Some(now); // started, never finished
        assert_eq!(display_status(&c, now), DisplayStatus::Running);
    }

    #[test]
    fn down_and_paused_unaffected_by_running() {
        let now = Utc::now();
        let mut c = base_check();
        c.last_ping_at = Some(now - Duration::seconds(100));
        c.last_start_at = Some(now); // started again after a failed/paused run
        for s in [CheckStatus::Down, CheckStatus::Paused] {
            c.status = s;
            assert_eq!(
                display_status(&c, now),
                if s == CheckStatus::Down {
                    DisplayStatus::Down
                } else {
                    DisplayStatus::Paused
                }
            );
        }
    }

    #[test]
    fn running_cleared_by_a_later_success() {
        let now = Utc::now();
        let mut c = base_check();
        c.status = CheckStatus::Up;
        c.last_start_at = Some(now - Duration::seconds(50));
        c.last_ping_at = Some(now); // success landed after the start
        assert_eq!(display_status(&c, now), DisplayStatus::Up);
    }

    #[test]
    fn both_timestamps_none_is_not_running() {
        let now = Utc::now();
        let mut c = base_check();
        c.status = CheckStatus::New;
        c.last_start_at = None;
        c.last_ping_at = None;
        assert_eq!(display_status(&c, now), DisplayStatus::New);
    }

    #[test]
    fn stored_states_pass_through() {
        let now = Utc::now();
        let mut c = base_check();
        for (s, d) in [
            (CheckStatus::New, DisplayStatus::New),
            (CheckStatus::Down, DisplayStatus::Down),
            (CheckStatus::Paused, DisplayStatus::Paused),
        ] {
            c.status = s;
            assert_eq!(display_status(&c, now), d);
        }
    }

    #[test]
    fn duration_pairs_start_with_next_finish() {
        let t0 = Utc.with_ymd_and_hms(2026, 7, 14, 8, 0, 0).unwrap();
        let pings = vec![
            ping(1, PingKind::Start, t0),
            ping(2, PingKind::Success, t0 + Duration::seconds(242)),
        ];
        let d = run_durations(&pings);
        assert_eq!(d.get(&2), Some(&242));
    }

    #[test]
    fn heartbeat_no_duration_is_hollow() {
        let t0 = Utc.with_ymd_and_hms(2026, 7, 14, 8, 0, 0).unwrap();
        let pings = vec![
            ping(1, PingKind::Success, t0),
            ping(2, PingKind::Success, t0 + Duration::seconds(60)),
        ];
        let bars = heartbeat(&pings, None, false, 6);
        assert!(bars.iter().all(|b| b.class == "none"));
        // No start pings at all → duration is genuinely unknown for every bar.
        assert!(bars.iter().all(|b| b.title == "duration unknown"));
    }

    #[test]
    fn heartbeat_known_duration_without_ceiling_has_distinct_title() {
        // A single measured run (start→success), no max_runtime_secs configured:
        // ceiling stays None (window fallback needs >=2 measured durations), but
        // the duration itself IS known — the tooltip must say so, not claim
        // "duration unknown".
        let t0 = Utc.with_ymd_and_hms(2026, 7, 14, 8, 0, 0).unwrap();
        let pings = vec![
            ping(1, PingKind::Start, t0),
            ping(2, PingKind::Success, t0 + Duration::seconds(42)),
        ];
        let bars = heartbeat(&pings, None, false, 6);
        let bar = bars.last().unwrap();
        assert_eq!(bar.class, "none"); // height/class logic unchanged
        assert_eq!(bar.title, "no runtime limit set");
    }

    #[test]
    fn heartbeat_hot_when_over_80pct_of_max_runtime() {
        let t0 = Utc.with_ymd_and_hms(2026, 7, 14, 8, 0, 0).unwrap();
        let pings = vec![
            ping(1, PingKind::Start, t0),
            ping(2, PingKind::Success, t0 + Duration::seconds(90)), // 90/100 = 90%
        ];
        let bars = heartbeat(&pings, Some(100), false, 6);
        assert_eq!(bars.last().unwrap().class, "hot");
    }

    #[test]
    fn heartbeat_paused_is_flatline() {
        let bars = heartbeat(&[], None, true, 6);
        assert_eq!(bars.len(), 6);
        assert!(bars.iter().all(|b| b.class == "pausedbar"));
    }
}
