use crate::models::{Check, CheckStatus, PingKind, PingSummary};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

/// Build version stamped in by `build.rs` from `git describe --tags --always
/// --dirty`: a release tag, a tag plus distance, or — before the first tag, or
/// from a shallow CI checkout — a bare short SHA.
pub fn version() -> &'static str {
    env!("GIT_VERSION")
}

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
/// (success/fail). A `Log` ping stamps neither column, so it cannot clear
/// this. `Option`'s ordering (`Some(_) > None`, `None > None` false) makes
/// this one comparison cover both "started and never finished" and "started
/// again after the last finish".
fn is_running(check: &Check) -> bool {
    check.last_start_at > check.last_ping_at
}

/// `next_due_at` already includes grace, so `next_due_at - grace` is the
/// expected run time. A stored-Up check inside `(expected, due]` is "late".
///
/// Precedence is `Paused > Down > Running > Late > Up`. `Running` applies only
/// to a stored `Up`/`New` check and beats `Late`, since a long-running job
/// legitimately drifts past its expected time. `Down`/`Paused` ignore
/// `is_running`, so an in-flight run never masks an alert.
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
pub fn run_durations(pings: &[PingSummary]) -> HashMap<i64, i64> {
    let mut ordered: Vec<&PingSummary> = pings.iter().collect();
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
    pings: &[PingSummary],
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
    let mut runs: Vec<&PingSummary> = pings.iter().filter(|p| is_finish(p.kind)).collect();
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
                        // Duration measured, but no ceiling to render it as a
                        // fraction of.
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

/// Every IANA timezone, for the `<datalist>` behind the timezone fields.
/// Called straight from the templates so no template struct has to carry a
/// 597-entry constant.
pub fn timezones() -> &'static [chrono_tz::Tz] {
    &chrono_tz::TZ_VARIANTS
}

/// The suggestions behind the `<datalist>` on every interval-shaped duration
/// field (check/project overrides, `/admin`'s global intervals). Called
/// straight from the templates, like [`timezones`].
///
/// A hint, not a constraint: the handlers still parse and validate whatever is
/// typed. The point is to make the unit suffixes discoverable — a field
/// holding `3600` reads as one that wants seconds. One list spans every use
/// (scan intervals in seconds, nag intervals in hours) so they read as one
/// kind of field, and every entry must round-trip through `parse_duration`
/// (`durations_all_parse`).
pub fn durations() -> &'static [&'static str] {
    &[
        "30s", "1m", "5m", "15m", "30m", "1h", "6h", "12h", "1d", "7d",
    ]
}

/// The same for `/account`'s API key expiry: same parser, different scale, so
/// kept separate from [`durations`] rather than carrying entries that are
/// noise in the other list.
pub fn expiries() -> &'static [&'static str] {
    &["7d", "30d", "90d", "365d"]
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

/// An absolute timestamp as the UI's plain-text fallback: what stays on the
/// page when `app.js` does not rewrite a `.localtime[data-ts]` span into the
/// viewer's zone. Human-readable rather than machine-readable — no sub-second
/// digits, no `T`, and the zone spelled out, since a bare number that might be
/// local or might be UTC is worse than one that says which.
///
/// Takes a reference so Askama can call it on a field or a `Some(t)` binding
/// alike.
pub fn fmt_utc(at: &DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M:%S UTC").to_string()
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

/// Forward-looking mirror of [`fmt_relative`]: how far `then` still is from
/// `now`, at the same granularity ("in 45s", "in 12m", "in 3h", "in 2d").
pub fn fmt_until(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let s = (then - now).num_seconds().max(0);
    if s < 60 {
        format!("in {s}s")
    } else if s < 3600 {
        format!("in {}m", s / 60)
    } else if s < 86400 {
        format!("in {}h", s / 3600)
    } else {
        format!("in {}d", s / 86400)
    }
}

/// The "when is the next ping expected" line on the check page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextDue {
    /// Visible text, e.g. `due in 57m`, `overdue by 12m`.
    pub label: String,
    /// The deadline as RFC 3339, carried in the element's `title`. `None`
    /// whenever `label` names a state rather than an instant (paused,
    /// unschedulable), so the template renders no tooltip.
    pub iso: Option<String>,
}

/// Describe a check's next deadline for display.
///
/// The source is [`crate::scheduler::due_time`], not the stored
/// `checks.next_due_at` column: that column is only stamped by `ping::apply`,
/// so it is `NULL` for a never-pinged check and for one downed by a `fail`
/// ping. `due_time` is what `scan_once` evaluates, so the rendered deadline
/// predicts when the check will actually be marked down.
///
/// The deadline includes grace (see [`display_status`]) — hence "due" rather
/// than "expected", which is `grace_secs` earlier.
pub fn next_due(check: &Check, now: DateTime<Utc>) -> NextDue {
    let unlabelled = |label: &str| NextDue {
        label: label.into(),
        iso: None,
    };
    // Spec §6: a paused check is excluded from monitoring, so a countdown
    // here would be a deadline nothing enforces.
    if check.status == CheckStatus::Paused {
        return unlabelled("not scheduled while paused");
    }
    let Some(due) = crate::scheduler::due_time(check) else {
        // Period check with no period, or an uninterpretable cron expression.
        return unlabelled("next due unknown");
    };
    let label = if now >= due {
        format!("overdue by {}", fmt_secs((now - due).num_seconds()))
    } else if check.last_ping_at.is_none() {
        // Never pinged: the deadline is anchored on creation, so say "first
        // ping" rather than let "due in 1h" imply a run already happened.
        format!("first ping due {}", fmt_until(due, now))
    } else {
        format!("due {}", fmt_until(due, now))
    };
    NextDue {
        label,
        iso: Some(due.to_rfc3339()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Check, CheckStatus, PingKind, PingSummary, ScheduleKind};
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
    fn ping(id: i64, kind: PingKind, at: chrono::DateTime<Utc>) -> PingSummary {
        PingSummary {
            id,
            check_id: 1,
            kind,
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
        assert!(bars.iter().all(|b| b.title == "duration unknown"));
    }

    #[test]
    fn heartbeat_known_duration_without_ceiling_has_distinct_title() {
        // One measured run and no max_runtime_secs leaves ceiling None, but
        // the duration is known — the tooltip must not say "duration unknown".
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

    #[test]
    fn next_due_counts_down_from_the_last_ping() {
        let now = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let mut c = base_check();
        // 1h period + 5m grace, pinged 30m ago → 35m left on the deadline.
        c.last_ping_at = Some(now - Duration::minutes(30));
        let d = next_due(&c, now);
        assert_eq!(d.label, "due in 35m");
        assert_eq!(
            d.iso,
            Some((now + Duration::minutes(35)).to_rfc3339()),
            "the tooltip carries the exact deadline"
        );
    }

    #[test]
    fn next_due_names_the_first_ping_when_none_has_arrived() {
        let now = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let mut c = base_check();
        c.status = CheckStatus::New;
        c.last_ping_at = None;
        c.created_at = now - Duration::minutes(5);
        // Anchored on creation: the deadline is real (scan_once will down it)
        // but says nothing about a run that happened.
        assert_eq!(next_due(&c, now).label, "first ping due in 1h");
    }

    #[test]
    fn next_due_reports_how_far_past_the_deadline_a_check_is() {
        let now = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let mut c = base_check();
        c.status = CheckStatus::Down;
        c.last_ping_at = Some(now - Duration::minutes(75)); // deadline was 10m ago
        let d = next_due(&c, now);
        assert_eq!(d.label, "overdue by 10m 00s");
        assert!(d.iso.is_some());
    }

    #[test]
    fn next_due_on_a_paused_check_names_the_state_and_offers_no_tooltip() {
        let now = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let mut c = base_check();
        c.status = CheckStatus::Paused;
        c.last_ping_at = Some(now - Duration::hours(9)); // long overdue, if it counted
        let d = next_due(&c, now);
        assert_eq!(d.label, "not scheduled while paused");
        assert_eq!(d.iso, None);
    }

    #[test]
    fn next_due_is_unknown_when_the_schedule_cannot_be_evaluated() {
        let now = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let mut c = base_check();
        c.schedule_kind = ScheduleKind::Cron;
        c.cron_expr = Some("not a cron".into());
        let d = next_due(&c, now);
        assert_eq!(d.label, "next due unknown");
        assert_eq!(d.iso, None);
    }

    #[test]
    fn fmt_until_mirrors_fmt_relative_granularity_and_floors_at_zero() {
        let now = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        assert_eq!(fmt_until(now + Duration::seconds(45), now), "in 45s");
        assert_eq!(fmt_until(now + Duration::seconds(750), now), "in 12m");
        assert_eq!(fmt_until(now + Duration::hours(3), now), "in 3h");
        assert_eq!(fmt_until(now + Duration::days(2), now), "in 2d");
        assert_eq!(fmt_until(now - Duration::hours(1), now), "in 0s");
    }

    /// A suggestion the field would reject is worse than no suggestion. Both
    /// lists are held to the handlers' parser and to `> 0` — the strictest
    /// bound any field carrying them applies (only grace is looser, at
    /// `>= 0`), so one check covers every field.
    #[test]
    fn every_suggested_duration_is_one_the_forms_accept() {
        for raw in durations().iter().chain(expiries().iter()) {
            let parsed = crate::duration::parse_duration(raw);
            assert!(parsed.is_some(), "{raw:?} does not parse as a duration");
            assert!(
                parsed.unwrap() > 0,
                "{raw:?} parses but is not the positive duration the forms require"
            );
        }
    }

    /// A browser renders a datalist in document order, so an unsorted list
    /// reads as arbitrary rather than as a scale.
    #[test]
    fn suggested_durations_are_sorted_and_distinct() {
        for list in [durations(), expiries()] {
            let secs: Vec<i64> = list
                .iter()
                .map(|raw| crate::duration::parse_duration(raw).unwrap())
                .collect();
            let mut sorted = secs.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(
                secs, sorted,
                "{list:?} is not sorted shortest-first, or repeats"
            );
        }
    }
}
