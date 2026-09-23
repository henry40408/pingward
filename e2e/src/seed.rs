//! Deterministic demo data for the README screenshots: [`seed_sql`] builds one
//! SQL script for a *stopped* database holding only the `POST /setup` admin.
//!
//! `scheduler::scan_once` runs at boot, so every row must stay inside its
//! budget or the scan rewrites the status being shown:
//!
//! * `up`/`new` is downed once `last_ping_at + period + grace <= now`;
//! * an in-flight run is downed once `last_start_at + max_runtime <= now`.
//!
//! `next_due_at` is seeded too, since `view::display_status` reads it for `late`.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

/// The `POST /setup` admin; seeded users copy its password hash.
pub const ADMIN_USERNAME: &str = "demo";

pub const ADMIN_PASSWORD: &str = "screenshot-demo-password";

const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;

/// JavaScript's `mulberry32`, bit for bit (wrapping `u32` ops stand in for
/// `Math.imul`/`>>>`). It drives every jitter, duration and ping UUID, so any
/// change alters every committed PNG.
struct Mulberry32(u32);

impl Mulberry32 {
    const fn new(seed: u32) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let seed = self.0;
        let mut t = (seed ^ (seed >> 15)).wrapping_mul(1 | seed);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(0x3d | t))) ^ t;
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }

    /// A value in `[low, high)`.
    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.next()
    }

    /// `Math.floor(rand() * len)`.
    fn index(&mut self, len: usize) -> usize {
        // The modulo only guards the float rounding edge.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let raw = (self.next() * len as f64) as usize;
        raw % len
    }
}

/// SQL-quotes a value, or `NULL`.
fn q(value: Option<&str>) -> String {
    match value {
        None => "NULL".to_owned(),
        Some(value) => format!("'{}'", value.replace('\'', "''")),
    }
}

fn qs(value: &str) -> String {
    q(Some(value))
}

/// Epoch ms as RFC3339 with millisecond precision, like `toISOString`.
fn iso(ms: f64) -> String {
    let millis = ms.round() as i64;
    DateTime::from_timestamp_millis(millis)
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp_nanos(0))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn num(value: Option<i64>) -> String {
    value.map_or_else(|| "NULL".to_owned(), |value| value.to_string())
}

// ---- a minimal cron evaluator ----------------------------------------------
//
// Anchors a cron check's last ping exactly on its last fire; anchored off a
// fire, the boot scan sees an unserved fire and downs the check. Supports only
// `*`, `*/n`, plain numbers and `,` lists.

const MINUTE_MS: i64 = 60_000;

/// Over a year of minutes, so a yearly cron still resolves.
const SCAN_LIMIT_MINUTES: i64 = 400 * 24 * 60;

/// Cron fields of an instant in `tz`. `dow` must match the `cron` crate
/// (Sunday = 1): 0-based, a weekly check anchors a day out and the boot scan
/// downs it.
struct Fields {
    sec: u32,
    min: u32,
    hour: u32,
    dom: u32,
    mon: u32,
    dow: u32,
}

fn local_fields(ms: i64, tz: Tz) -> Result<Fields> {
    let instant = DateTime::from_timestamp_millis(ms).context("timestamp out of range")?;
    let local = tz.from_utc_datetime(&instant.naive_utc());
    Ok(Fields {
        sec: local.second(),
        min: local.minute(),
        hour: local.hour(),
        dom: local.day(),
        mon: local.month(),
        dow: local.weekday().number_from_sunday(),
    })
}

fn field_matches(spec: &str, value: u32) -> bool {
    spec.split(',').any(|part| {
        if part == "*" {
            return true;
        }
        if let Some(step) = part.strip_prefix("*/") {
            return step
                .parse::<u32>()
                .is_ok_and(|step| step != 0 && value.is_multiple_of(step));
        }
        part.parse::<u32>() == Ok(value)
    })
}

fn cron_matches(expr: &str, tz: Tz, ms: i64) -> Result<bool> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    let [sec, min, hour, dom, mon, dow] = parts[..] else {
        bail!("`{expr}` is not a 6-field cron expression");
    };
    let fields = local_fields(ms, tz)?;
    Ok(field_matches(sec, fields.sec)
        && field_matches(min, fields.min)
        && field_matches(hour, fields.hour)
        && field_matches(dom, fields.dom)
        && field_matches(mon, fields.mon)
        && field_matches(dow, fields.dow))
}

/// The last fire at or before `from_ms`. Minute steps are exact because every
/// expression here has a zero seconds field.
fn last_fire_at_or_before(expr: &str, tz: Tz, from_ms: i64) -> Result<i64> {
    let mut t = from_ms.div_euclid(MINUTE_MS) * MINUTE_MS;
    for _ in 0..SCAN_LIMIT_MINUTES {
        if cron_matches(expr, tz, t)? {
            return Ok(t);
        }
        t -= MINUTE_MS;
    }
    bail!("cron `{expr}` ({tz}) has no fire in the past year")
}

/// The first fire strictly after `from_ms`, as `scheduler::due_time` computes.
fn next_fire_after(expr: &str, tz: Tz, from_ms: i64) -> Result<i64> {
    let mut t = from_ms.div_euclid(MINUTE_MS) * MINUTE_MS + MINUTE_MS;
    for _ in 0..SCAN_LIMIT_MINUTES {
        if cron_matches(expr, tz, t)? {
            return Ok(t);
        }
        t += MINUTE_MS;
    }
    bail!("cron `{expr}` ({tz}) has no fire in the next year")
}

// ---- the dataset -----------------------------------------------------------

/// Extra owners so `/admin`'s cross-user cards have content.
const EXTRA_USERS: [(&str, bool); 2] = [("maya", true), ("sam", false)];

struct Project {
    key: &'static str,
    /// Indexes into `[demo, maya, sam]`.
    owner: usize,
    name: &'static str,
    description: &'static str,
    scan_interval_secs: Option<i64>,
    nag_interval_secs: Option<i64>,
}

const PROJECTS: [Project; 4] = [
    Project {
        key: "backups",
        owner: 0,
        name: "Backups",
        description: "Nightly database dumps and offsite sync. **Paging** goes to the on-call rotation.",
        scan_interval_secs: None,
        nag_interval_secs: Some(1800),
    },
    Project {
        key: "pipeline",
        owner: 0,
        name: "Data pipeline",
        description: "Hourly ETL plus the downstream dbt models.",
        scan_interval_secs: Some(60),
        nag_interval_secs: None,
    },
    Project {
        key: "web",
        owner: 0,
        name: "Website",
        description: "Certificate renewal and the housekeeping crons for the public site.",
        scan_interval_secs: None,
        nag_interval_secs: None,
    },
    Project {
        key: "staging",
        owner: 2,
        name: "Staging (sam)",
        description: "Another user's project — only reachable from /admin.",
        scan_interval_secs: None,
        nag_interval_secs: None,
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Up,
    Down,
    Late,
    Running,
    Paused,
}

struct Check {
    key: &'static str,
    project: &'static str,
    name: &'static str,
    description: &'static str,
    cron: Option<&'static str>,
    timezone: &'static str,
    /// Run interval in seconds: spaces the ping history and, for period
    /// checks, is the schedule.
    cadence: i64,
    grace: i64,
    max_runtime: Option<i64>,
    state: State,
    /// Duration band (secs); keep it a visible share of `max_runtime`, which
    /// the heartbeat scales bar height by.
    runtime: (f64, f64),
    channels: &'static [&'static str],
}

const CHECKS: [Check; 11] = [
    Check {
        key: "pg-dump",
        project: "backups",
        name: "postgres-nightly-dump",
        description: "`pg_dump` of the primary, streamed straight to object storage.",
        cron: Some("0 30 2 * * *"),
        timezone: "Europe/Berlin",
        cadence: DAY,
        grace: 15 * 60,
        max_runtime: Some(45 * 60),
        state: State::Up,
        runtime: (900.0, 1500.0),
        channels: &["ops-slack", "oncall-pushover"],
    },
    Check {
        key: "s3-sync",
        project: "backups",
        name: "s3-offsite-sync",
        description: "Mirrors last night's dump to a second region.",
        cron: None,
        timezone: "UTC",
        cadence: HOUR,
        grace: 5 * 60,
        max_runtime: Some(20 * 60),
        state: State::Up,
        runtime: (120.0, 900.0),
        channels: &["ops-slack"],
    },
    Check {
        key: "nas-snapshot",
        project: "backups",
        name: "home-nas-snapshot",
        description: "ZFS snapshot + scrub on the NAS. Missed its window last night.",
        cron: None,
        timezone: "UTC",
        cadence: 6 * HOUR,
        grace: 30 * 60,
        max_runtime: Some(90 * 60),
        state: State::Down,
        runtime: (1400.0, 3200.0),
        channels: &["ops-slack", "oncall-pushover", "alerts-email"],
    },
    Check {
        key: "archive-verify",
        project: "backups",
        name: "photo-archive-verify",
        description: "Weekly checksum sweep over the cold archive. Paused during the migration.",
        cron: None,
        timezone: "UTC",
        cadence: 7 * DAY,
        grace: DAY,
        max_runtime: None,
        state: State::Paused,
        runtime: (4000.0, 9000.0),
        channels: &["alerts-email"],
    },
    Check {
        key: "etl",
        project: "pipeline",
        name: "etl-hourly",
        description: "Extracts yesterday's events into the warehouse.",
        cron: None,
        timezone: "UTC",
        cadence: HOUR,
        grace: 10 * 60,
        max_runtime: Some(25 * 60),
        state: State::Running,
        runtime: (400.0, 1100.0),
        channels: &["pipeline-ntfy"],
    },
    Check {
        key: "dbt",
        project: "pipeline",
        name: "dbt-run",
        description: "Rebuilds the marts once the ETL lands.",
        cron: Some("0 15 */4 * * *"),
        timezone: "UTC",
        cadence: 4 * HOUR,
        grace: 10 * 60,
        max_runtime: Some(30 * 60),
        state: State::Up,
        runtime: (600.0, 1500.0),
        channels: &["pipeline-ntfy"],
    },
    Check {
        key: "feature-export",
        project: "pipeline",
        name: "feature-export",
        description: "Pushes the feature table to the serving store. Running behind.",
        cron: None,
        timezone: "UTC",
        cadence: 30 * 60,
        grace: 10 * 60,
        max_runtime: Some(15 * 60),
        state: State::Late,
        runtime: (200.0, 700.0),
        channels: &["pipeline-ntfy"],
    },
    Check {
        key: "certbot",
        project: "web",
        name: "certbot-renew",
        description: "Weekly ACME renewal for `www` and the wildcard.",
        cron: Some("0 0 4 * * 1"),
        timezone: "UTC",
        cadence: 7 * DAY,
        grace: 6 * HOUR,
        max_runtime: Some(4 * 60),
        state: State::Up,
        runtime: (40.0, 170.0),
        channels: &["ops-slack"],
    },
    Check {
        key: "sitemap",
        project: "web",
        name: "sitemap-rebuild",
        description: "",
        cron: None,
        timezone: "UTC",
        cadence: DAY,
        grace: 2 * HOUR,
        max_runtime: Some(15 * 60),
        state: State::Up,
        runtime: (180.0, 640.0),
        channels: &[],
    },
    Check {
        key: "linkcheck",
        project: "web",
        name: "broken-link-sweep",
        description: "Crawls the docs for dead links.",
        cron: None,
        timezone: "UTC",
        cadence: 12 * HOUR,
        grace: HOUR,
        max_runtime: Some(40 * 60),
        state: State::Up,
        runtime: (800.0, 2100.0),
        channels: &["ops-slack"],
    },
    Check {
        key: "staging-seed",
        project: "staging",
        name: "staging-db-reseed",
        description: "",
        cron: None,
        timezone: "UTC",
        cadence: DAY,
        grace: HOUR,
        max_runtime: None,
        state: State::Up,
        runtime: (300.0, 800.0),
        channels: &[],
    },
];

struct Channel {
    key: &'static str,
    project: &'static str,
    kind: &'static str,
    name: &'static str,
    config: &'static str,
}

const CHANNELS: [Channel; 4] = [
    Channel {
        key: "ops-slack",
        project: "backups",
        kind: "slack",
        name: "#ops-alerts",
        config: r#"{"url":"https://hooks.slack.com/services/T000/B000/xxxxxxxxxxxx"}"#,
    },
    Channel {
        key: "oncall-pushover",
        project: "backups",
        kind: "pushover",
        name: "on-call phone",
        config: r#"{"token":"axxxxxxxxxxxxxxxxxxxxxxxxxxxxx","user":"uxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"}"#,
    },
    Channel {
        key: "alerts-email",
        project: "backups",
        kind: "email",
        name: "alerts@example.com",
        config: r#"{"to":"alerts@example.com"}"#,
    },
    Channel {
        key: "pipeline-ntfy",
        project: "pipeline",
        kind: "ntfy",
        name: "ntfy · data-pipeline",
        config: r#"{"base_url":"https://ntfy.sh","topic":"data-pipeline"}"#,
    },
];

const FAIL_BODIES: [&str; 3] = [
    "rsync: connection unexpectedly closed (0 bytes received so far)\nrsync error: error in rsync protocol data stream (code 12) at io.c(228)",
    "pg_dump: error: connection to server failed: FATAL:  the database system is in recovery mode",
    "zpool status: one or more devices has experienced an unrecoverable error\n  scrub repaired 0B with 3 errors",
];

const SOURCE_IPS: [&str; 3] = ["10.4.2.15", "10.4.2.31", "192.168.20.8"];

/// `check`/`channel` hold `SELECT` sub-queries by name: ids are DB-assigned.
struct Notification {
    check: String,
    channel: String,
    event: &'static str,
    status: &'static str,
    error: Option<&'static str>,
    at: f64,
}

struct Audit {
    actor: &'static str,
    action: &'static str,
    target_type: &'static str,
    target_id: i64,
    method: &'static str,
    path: &'static str,
    detail: Option<&'static str>,
    /// Seconds before now.
    ago: f64,
}

/// Enough to fill the heartbeat strip.
const RUNS: i64 = 34;

/// Seconds before now of the last finished run, scaled by cadence.
fn last_finish_offset(check: &Check, rand: &mut Mulberry32) -> f64 {
    let (cadence, grace) = (check.cadence as f64, check.grace as f64);
    match check.state {
        State::Down => cadence * 2.0 + grace + 900.0,
        // Inside grace: the scan leaves it, `display_status` says `late`.
        State::Late => cadence + grace * 0.5,
        State::Running => cadence * 0.95,
        State::Paused => cadence * 0.4,
        State::Up => cadence * (0.15 + 0.35 * rand.next()),
    }
}

/// Deterministic, since the ping URL is in the check-page screenshot.
fn uuid_for(rand: &mut Mulberry32) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = |count: usize| -> String {
        (0..count)
            .map(|_| DIGITS[rand.index(DIGITS.len())] as char)
            .collect()
    };
    format!("{}-{}-4{}-a{}-{}", hex(8), hex(4), hex(3), hex(3), hex(12))
}

fn ping_sql(check: &str, kind: &str, at_ms: f64, body: &str, ip: &str) -> String {
    format!(
        "INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)\n  \
         VALUES ({check}, {}, NULL, {}, {}, {});",
        qs(kind),
        qs(body),
        qs(ip),
        qs(&iso(at_ms))
    )
}

/// Builds the demo dataset as one SQL transaction, anchored at `now_ms`.
pub fn seed_sql(now_ms: i64) -> Result<String> {
    let now = now_ms as f64;
    let mut rand = Mulberry32::new(20_260_722);
    let mut stmts = vec!["BEGIN;".to_owned()];

    // ---- users -------------------------------------------------------------
    for (index, (username, is_admin)) in EXTRA_USERS.iter().enumerate() {
        let created = now - (40.0 - index as f64 * 6.0) * DAY as f64 * 1000.0;
        stmts.push(format!(
            "INSERT INTO users (username, password_hash, is_admin, created_at)\n  \
             SELECT {}, password_hash, {}, {}\n  FROM users WHERE username = {};",
            qs(username),
            i32::from(*is_admin),
            qs(&iso(created)),
            qs(ADMIN_USERNAME),
        ));
    }
    let owner_sql = |index: usize| {
        let username = if index == 0 {
            ADMIN_USERNAME
        } else {
            EXTRA_USERS[index - 1].0
        };
        format!("(SELECT id FROM users WHERE username = {})", qs(username))
    };

    // ---- projects ----------------------------------------------------------
    for (index, project) in PROJECTS.iter().enumerate() {
        let created = now - (60.0 - index as f64 * 5.0) * DAY as f64 * 1000.0;
        stmts.push(format!(
            "INSERT INTO projects (user_id, name, description, scan_interval_secs, \
             nag_interval_secs, created_at)\n  VALUES ({}, {}, {}, {}, {}, {});",
            owner_sql(project.owner),
            qs(project.name),
            qs(project.description),
            num(project.scan_interval_secs),
            num(project.nag_interval_secs),
            qs(&iso(created)),
        ));
    }
    let project_sql = |key: &str| -> Result<String> {
        let project = PROJECTS
            .iter()
            .find(|project| project.key == key)
            .with_context(|| format!("no seeded project keyed `{key}`"))?;
        Ok(format!(
            "(SELECT id FROM projects WHERE name = {})",
            qs(project.name)
        ))
    };

    // ---- channels ----------------------------------------------------------
    for channel in &CHANNELS {
        stmts.push(format!(
            "INSERT INTO channels (project_id, kind, name, config_json, created_at)\n  \
             VALUES ({}, {}, {}, {}, {});",
            project_sql(channel.project)?,
            qs(channel.kind),
            qs(channel.name),
            qs(channel.config),
            qs(&iso(now - 50.0 * DAY as f64 * 1000.0)),
        ));
    }
    let channel_sql = |key: &str| -> Result<String> {
        let channel = CHANNELS
            .iter()
            .find(|channel| channel.key == key)
            .with_context(|| format!("no seeded channel keyed `{key}`"))?;
        Ok(format!(
            "(SELECT id FROM channels WHERE name = {})",
            qs(channel.name)
        ))
    };

    // ---- checks, their ping history, and channel bindings ------------------
    let mut notifications: Vec<Notification> = Vec::new();

    for check in &CHECKS {
        let tz: Tz = check
            .timezone
            .parse()
            .map_err(|_ignored| anyhow::anyhow!("unknown timezone `{}`", check.timezone))?;
        // Cron checks anchor on a real fire; period checks per their state.
        let finish_at = match check.cron {
            Some(expr) => last_fire_at_or_before(expr, tz, now_ms)? as f64,
            None => now - last_finish_offset(check, &mut rand) * 1000.0,
        };
        let period = check.cron.is_none().then_some(check.cadence);
        let next_due = match check.cron {
            Some(expr) => {
                (next_fire_after(expr, tz, finish_at as i64)? + check.grace * 1000) as f64
            }
            None => finish_at + (check.cadence + check.grace) as f64 * 1000.0,
        };

        let status = match check.state {
            State::Down => "down",
            State::Paused => "paused",
            _ => "up",
        };
        // `running` = a `start` newer than the last finish, kept inside
        // `max_runtime` so the scan does not down it.
        let running_start = (check.state == State::Running).then_some(now - 6.0 * 60.0 * 1000.0);
        let last_start = running_start.unwrap_or(finish_at - 60.0 * 1000.0);

        stmts.push(format!(
            "INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, \
             period_secs, grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at, \
             next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)\n  \
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, 0, {});",
            project_sql(check.project)?,
            qs(check.name),
            qs(check.description),
            qs(&uuid_for(&mut rand)),
            qs(if check.cron.is_some() {
                "cron"
            } else {
                "period"
            }),
            num(period),
            check.grace,
            q(check.cron),
            qs(check.timezone),
            qs(status),
            qs(&iso(finish_at)),
            qs(&iso(last_start)),
            qs(&iso(next_due)),
            num(check.max_runtime),
            if check.state == State::Down {
                qs(&iso(now - 10.0 * 60.0 * 1000.0))
            } else {
                "NULL".to_owned()
            },
            qs(&iso(now - 45.0 * DAY as f64 * 1000.0)),
        ));

        let check_sql = format!("(SELECT id FROM checks WHERE name = {})", qs(check.name));
        for key in check.channels {
            stmts.push(format!(
                "INSERT INTO check_channels (check_id, channel_id) VALUES ({check_sql}, {});",
                channel_sql(key)?
            ));
        }

        let (low, high) = check.runtime;
        for index in (0..RUNS).rev() {
            let jitter = (rand.next() - 0.5) * check.cadence as f64 * 0.06;
            let end = finish_at - (index * check.cadence) as f64 * 1000.0 + jitter * 1000.0;
            // Every ninth run is slow (amber bar, if `max_runtime` is set);
            // two runs of the down check fail with captured output.
            let slow = index % 9 == 4;
            let failed = check.state == State::Down && (index == 0 || index == 12);
            let mut duration = rand.range(low, high);
            if let Some(max_runtime) = check.max_runtime
                && slow
            {
                duration = max_runtime as f64 * (0.82 + 0.1 * rand.next());
            }
            let start = end - duration * 1000.0;
            let ip = SOURCE_IPS[rand.index(SOURCE_IPS.len())];
            stmts.push(ping_sql(&check_sql, "start", start, "", ip));
            stmts.push(ping_sql(
                &check_sql,
                if failed { "fail" } else { "success" },
                end,
                if failed {
                    FAIL_BODIES[usize::try_from(index).unwrap_or_default() % FAIL_BODIES.len()]
                } else {
                    ""
                },
                ip,
            ));
        }
        if let Some(running_start) = running_start {
            stmts.push(ping_sql(
                &check_sql,
                "start",
                running_start,
                "",
                SOURCE_IPS[0],
            ));
        }

        // The down check's alert chain plus one recovered incident on the
        // hourly sync, for a mixed log.
        if check.state == State::Down {
            let chain: [(&str, &str, &str, Option<&str>, f64); 5] = [
                ("down", "ops-slack", "ok", None, 62.0 * 60.0),
                ("down", "oncall-pushover", "ok", None, 62.0 * 60.0),
                (
                    "down",
                    "alerts-email",
                    "error",
                    Some("smtp: connection refused"),
                    62.0 * 60.0,
                ),
                ("reminder", "ops-slack", "ok", None, 32.0 * 60.0),
                ("reminder", "oncall-pushover", "ok", None, 32.0 * 60.0),
            ];
            for (event, channel, status, error, ago) in chain {
                notifications.push(Notification {
                    check: check_sql.clone(),
                    channel: channel_sql(channel)?,
                    event,
                    status,
                    error,
                    at: now - ago * 1000.0,
                });
            }
        }
        if check.key == "s3-sync" {
            for (event, ago) in [("down", 26.0), ("up", 25.4)] {
                notifications.push(Notification {
                    check: check_sql.clone(),
                    channel: channel_sql("ops-slack")?,
                    event,
                    status: "ok",
                    error: None,
                    at: now - ago * HOUR as f64 * 1000.0,
                });
            }
        }
    }

    for notification in notifications {
        stmts.push(format!(
            "INSERT INTO notifications (check_id, channel_id, event, status, error, created_at)\n  \
             VALUES ({}, {}, {}, {}, {}, {});",
            notification.check,
            notification.channel,
            qs(notification.event),
            qs(notification.status),
            q(notification.error),
            qs(&iso(notification.at)),
        ));
    }

    // ---- audit trail -------------------------------------------------------
    //
    // Shaped like real `record_audit` rows. Oldest first: the card orders by
    // `id`, which must agree with the timestamps.
    let audits: [Audit; 5] = [
        Audit {
            actor: "demo",
            action: "user.create",
            target_type: "user",
            target_id: 3,
            method: "POST",
            path: "/admin/users",
            detail: Some("username=sam is_admin=false"),
            ago: 30.0 * DAY as f64,
        },
        Audit {
            actor: "demo",
            action: "user.set_admin",
            target_type: "user",
            target_id: 2,
            method: "POST",
            path: "/admin/users/2/admin",
            detail: Some("is_admin=true"),
            ago: 26.0 * DAY as f64,
        },
        Audit {
            actor: "maya",
            action: "user.password_reset",
            target_type: "user",
            target_id: 3,
            method: "POST",
            path: "/admin/users/3/password",
            detail: None,
            ago: 6.0 * DAY as f64,
        },
        Audit {
            actor: "maya",
            action: "admin.access",
            target_type: "project",
            target_id: 1,
            method: "GET",
            path: "/admin/projects/1",
            detail: None,
            ago: 2.5 * HOUR as f64,
        },
        Audit {
            actor: "maya",
            action: "admin.access",
            target_type: "check",
            target_id: 3,
            method: "GET",
            path: "/admin/checks/3",
            detail: None,
            ago: 2.4 * HOUR as f64,
        },
    ];
    for Audit {
        actor,
        action,
        target_type,
        target_id,
        method,
        path,
        detail,
        ago,
    } in audits
    {
        stmts.push(format!(
            "INSERT INTO audit_log (actor_user_id, actor_username, action, target_type, \
             target_id, target_owner_id, method, path, detail, created_at)\n  \
             VALUES ((SELECT id FROM users WHERE username = {0}), {0}, {1}, {2}, {3}, {3}, \
             {4}, {5}, {6}, {7});",
            qs(actor),
            qs(action),
            qs(target_type),
            target_id,
            qs(method),
            qs(path),
            q(detail),
            qs(&iso(now - ago * 1000.0)),
        ));
    }

    // ---- global settings ---------------------------------------------------
    //
    // `prune_once` runs at boot. 90 days keeps all history except the weekly
    // checks' oldest runs (34 weeks), which the screenshots never show.
    for (key, value) in [
        ("scan_interval", "30"),
        ("nag_interval", "1800"),
        ("pings_retention_days", "90"),
        ("notifications_retention_days", "90"),
    ] {
        stmts.push(format!(
            "INSERT INTO settings (key, value) VALUES ({}, {})\n  \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value;",
            qs(key),
            qs(value),
        ));
    }

    stmts.push("COMMIT;".to_owned());
    Ok(stmts.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expected values come from the reference JavaScript `mulberry32` under Node.
    #[test]
    fn mulberry32_matches_the_javascript_sequence() {
        let mut rand = Mulberry32::new(20_260_722);
        let drawn: Vec<f64> = (0..4).map(|_| rand.next()).collect();
        assert_eq!(
            drawn,
            [
                0.238_021_608_209_237_46,
                0.324_671_987_909_823_66,
                0.291_686_902_754_008_77,
                0.640_540_145_337_581_6,
            ]
        );
    }

    #[test]
    fn weekdays_are_numbered_from_sunday_at_one() {
        // 2026-08-16 was a Sunday.
        let sunday = chrono::NaiveDate::from_ymd_opt(2026, 8, 16)
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .expect("a valid date")
            .and_utc()
            .timestamp_millis();
        assert_eq!(
            local_fields(sunday, chrono_tz::UTC).expect("evaluable").dow,
            1,
            "Sunday must be 1, as `cron` numbers it"
        );
        assert_eq!(
            local_fields(sunday + 86_400_000, chrono_tz::UTC)
                .expect("evaluable")
                .dow,
            2,
            "Monday must be 2"
        );
    }

    #[test]
    fn cron_anchors_land_on_a_real_fire() {
        let tz: Tz = "Europe/Berlin".parse().expect("a known zone");
        let now = 1_800_000_000_000;
        let last = last_fire_at_or_before("0 30 2 * * *", tz, now).expect("a fire in the past");
        assert!(last <= now);
        assert!(cron_matches("0 30 2 * * *", tz, last).expect("evaluable"));
        let next = next_fire_after("0 30 2 * * *", tz, last).expect("a fire in the future");
        assert!(next > last);
        assert!(cron_matches("0 30 2 * * *", tz, next).expect("evaluable"));
    }

    #[test]
    fn the_seed_script_is_one_transaction() {
        let sql = seed_sql(1_800_000_000_000).expect("the seed builds");
        assert!(sql.starts_with("BEGIN;"));
        assert!(sql.trim_end().ends_with("COMMIT;"));
        // Every check contributes 34 runs of two pings each.
        assert!(sql.matches("INSERT INTO pings").count() >= CHECKS.len() * 68);
    }
}
