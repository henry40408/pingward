// Deterministic demo data for the README screenshot pipeline.
//
// Pure data module: `generateSeedSql(nowMs)` returns one SQL script for the
// sqlite3 CLI, run against a *stopped* pingward whose only existing row is the
// admin created through POST /setup (so the argon2 hash is the product's own).
//
// Timestamps are RFC3339 text, matching every `*_at` column (`store::parse_ts`
// parses them with `DateTime::parse_from_rfc3339`).
//
// Two invariants the seeded rows must respect, because the scan loop runs a
// pass immediately at boot (`scheduler::scan_once`) and would otherwise rewrite
// the very statuses this seed exists to show:
//   * an `up`/`new` check is downed when `last_ping_at + period + grace <= now`
//     — so every non-down check's last finish stays inside its budget;
//   * an in-flight run is downed when `last_start_at + max_runtime <= now` —
//     so the "running" check's start is well under its max runtime.
// `next_due_at` is seeded to the same `last_ping_at + period + grace` the
// scheduler computes, since `view::display_status` reads that column to decide
// `late`.

export const ADMIN_USERNAME = "demo";
export const ADMIN_PASSWORD = "screenshot-demo-password";

const HOUR = 3600;
const DAY = 24 * HOUR;

function mulberry32(seed) {
  return function () {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const q = (v) => (v == null ? "NULL" : `'${String(v).replace(/'/g, "''")}'`);
const iso = (ms) => new Date(Math.round(ms)).toISOString();

// ---- minimal cron evaluator --------------------------------------------
// A cron check's due time is `next_fire_after(last_ping) + grace`, so anchoring
// its last ping anywhere other than an actual fire time can leave a fire
// between the anchor and now — which the boot scan would read as overdue and
// turn the check down. These two helpers put the anchor exactly on the last
// fire. Only the subset of the 6-field syntax this seed uses is supported:
// `*`, `*/n`, and plain numbers.
const WEEKDAYS = { Sun: 0, Mon: 1, Tue: 2, Wed: 3, Thu: 4, Fri: 5, Sat: 6 };
const formatters = new Map();

function localFields(ms, tz) {
  if (!formatters.has(tz)) {
    formatters.set(
      tz,
      new Intl.DateTimeFormat("en-US", {
        timeZone: tz,
        hour12: false,
        year: "numeric",
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        weekday: "short",
      })
    );
  }
  const parts = Object.fromEntries(
    formatters.get(tz).formatToParts(new Date(ms)).map((p) => [p.type, p.value])
  );
  return {
    sec: Number(parts.second),
    min: Number(parts.minute),
    // "24" is how en-US hour12:false renders midnight.
    hour: Number(parts.hour) % 24,
    dom: Number(parts.day),
    mon: Number(parts.month),
    dow: WEEKDAYS[parts.weekday],
  };
}

function fieldMatches(spec, value) {
  return spec.split(",").some((part) => {
    if (part === "*") return true;
    if (part.startsWith("*/")) return value % Number(part.slice(2)) === 0;
    return Number(part) === value;
  });
}

function cronMatches(expr, tz, ms) {
  const [sec, min, hour, dom, mon, dow] = expr.trim().split(/\s+/);
  const f = localFields(ms, tz);
  return (
    fieldMatches(sec, f.sec) &&
    fieldMatches(min, f.min) &&
    fieldMatches(hour, f.hour) &&
    fieldMatches(dom, f.dom) &&
    fieldMatches(mon, f.mon) &&
    fieldMatches(dow, f.dow)
  );
}

const MINUTE_MS = 60_000;
const SCAN_LIMIT_MINUTES = 400 * 24 * 60; // > a year, so a yearly cron still resolves

/** The most recent fire at or before `fromMs`. Every expression here has a
 *  zero seconds field, so stepping a minute at a time is exact. */
function lastFireAtOrBefore(expr, tz, fromMs) {
  let t = Math.floor(fromMs / MINUTE_MS) * MINUTE_MS;
  for (let i = 0; i < SCAN_LIMIT_MINUTES; i++, t -= MINUTE_MS) {
    if (cronMatches(expr, tz, t)) return t;
  }
  throw new Error(`cron "${expr}" (${tz}) has no fire in the past year`);
}

/** The first fire strictly after `fromMs` — what `scheduler::due_time` computes. */
function nextFireAfter(expr, tz, fromMs) {
  let t = Math.floor(fromMs / MINUTE_MS) * MINUTE_MS + MINUTE_MS;
  for (let i = 0; i < SCAN_LIMIT_MINUTES; i++, t += MINUTE_MS) {
    if (cronMatches(expr, tz, t)) return t;
  }
  throw new Error(`cron "${expr}" (${tz}) has no fire in the next year`);
}

// Owners. `demo` is the admin created by POST /setup; the other two exist so
// /admin's "All users" and cross-user "All projects" cards have something to
// show. They reuse the admin's argon2 hash — this database is throwaway.
const EXTRA_USERS = [
  { username: "maya", is_admin: true },
  { username: "sam", is_admin: false },
];

// `owner` indexes into [demo, maya, sam].
const PROJECTS = [
  {
    key: "backups",
    owner: 0,
    name: "Backups",
    description:
      "Nightly database dumps and offsite sync. **Paging** goes to the on-call rotation.",
    scan_interval_secs: null,
    nag_interval_secs: 1800,
  },
  {
    key: "pipeline",
    owner: 0,
    name: "Data pipeline",
    description: "Hourly ETL plus the downstream dbt models.",
    scan_interval_secs: 60,
    nag_interval_secs: null,
  },
  {
    key: "web",
    owner: 0,
    name: "Website",
    description: "Certificate renewal and the housekeeping crons for the public site.",
    scan_interval_secs: null,
    nag_interval_secs: null,
  },
  {
    key: "staging",
    owner: 2,
    name: "Staging (sam)",
    description: "Another user's project — only reachable from /admin.",
    scan_interval_secs: null,
    nag_interval_secs: null,
  },
];

// `cadence` is how often the job actually runs, in seconds; it drives both the
// synthetic ping history and (for period checks) the schedule itself. `state`
// picks where the last run sits relative to now — see the header note.
const CHECKS = [
  {
    key: "pg-dump",
    project: "backups",
    name: "postgres-nightly-dump",
    description: "`pg_dump` of the primary, streamed straight to object storage.",
    kind: "cron",
    cron: "0 30 2 * * *",
    timezone: "Europe/Berlin",
    cadence: DAY,
    grace: 15 * 60,
    max_runtime: 45 * 60,
    state: "up",
    runtime: [900, 1500],
    channels: ["ops-slack", "oncall-pushover"],
  },
  {
    key: "s3-sync",
    project: "backups",
    name: "s3-offsite-sync",
    description: "Mirrors last night's dump to a second region.",
    kind: "period",
    cadence: HOUR,
    grace: 5 * 60,
    max_runtime: 20 * 60,
    state: "up",
    runtime: [120, 900],
    channels: ["ops-slack"],
  },
  {
    key: "nas-snapshot",
    project: "backups",
    name: "home-nas-snapshot",
    description: "ZFS snapshot + scrub on the NAS. Missed its window last night.",
    kind: "period",
    cadence: 6 * HOUR,
    grace: 30 * 60,
    max_runtime: 90 * 60,
    state: "down",
    runtime: [1400, 3200],
    channels: ["ops-slack", "oncall-pushover", "alerts-email"],
  },
  {
    key: "archive-verify",
    project: "backups",
    name: "photo-archive-verify",
    description: "Weekly checksum sweep over the cold archive. Paused during the migration.",
    kind: "period",
    cadence: 7 * DAY,
    grace: DAY,
    max_runtime: null,
    state: "paused",
    runtime: [4000, 9000],
    channels: ["alerts-email"],
  },
  {
    key: "etl",
    project: "pipeline",
    name: "etl-hourly",
    description: "Extracts yesterday's events into the warehouse.",
    kind: "period",
    cadence: HOUR,
    grace: 10 * 60,
    max_runtime: 25 * 60,
    state: "running",
    runtime: [400, 1100],
    channels: ["pipeline-ntfy"],
  },
  {
    key: "dbt",
    project: "pipeline",
    name: "dbt-run",
    description: "Rebuilds the marts once the ETL lands.",
    kind: "cron",
    cron: "0 15 */4 * * *",
    timezone: "UTC",
    cadence: 4 * HOUR,
    grace: 10 * 60,
    max_runtime: 30 * 60,
    state: "up",
    runtime: [600, 1500],
    channels: ["pipeline-ntfy"],
  },
  {
    key: "feature-export",
    project: "pipeline",
    name: "feature-export",
    description: "Pushes the feature table to the serving store. Running behind.",
    kind: "period",
    cadence: 30 * 60,
    grace: 10 * 60,
    max_runtime: 15 * 60,
    state: "late",
    runtime: [200, 700],
    channels: ["pipeline-ntfy"],
  },
  {
    key: "certbot",
    project: "web",
    name: "certbot-renew",
    description: "Weekly ACME renewal for `www` and the wildcard.",
    kind: "cron",
    cron: "0 0 4 * * 1",
    timezone: "UTC",
    cadence: 7 * DAY,
    grace: 6 * HOUR,
    max_runtime: 4 * 60,
    // Runtimes are quoted as a band inside `max_runtime`: the heartbeat scales
    // bar height by `duration / max_runtime`, so a job that never uses more
    // than a few percent of its budget renders as a row of stubs.
    state: "up",
    runtime: [40, 170],
    channels: ["ops-slack"],
  },
  {
    key: "sitemap",
    project: "web",
    name: "sitemap-rebuild",
    description: "",
    kind: "period",
    cadence: DAY,
    grace: 2 * HOUR,
    max_runtime: 15 * 60,
    state: "up",
    runtime: [180, 640],
    channels: [],
  },
  {
    key: "linkcheck",
    project: "web",
    name: "broken-link-sweep",
    description: "Crawls the docs for dead links.",
    kind: "period",
    cadence: 12 * HOUR,
    grace: HOUR,
    max_runtime: 40 * 60,
    state: "up",
    runtime: [800, 2100],
    channels: ["ops-slack"],
  },
  {
    key: "staging-seed",
    project: "staging",
    name: "staging-db-reseed",
    description: "",
    kind: "period",
    cadence: DAY,
    grace: HOUR,
    max_runtime: null,
    state: "up",
    runtime: [300, 800],
    channels: [],
  },
];

const CHANNELS = [
  {
    key: "ops-slack",
    project: "backups",
    kind: "slack",
    name: "#ops-alerts",
    config: { url: "https://hooks.slack.com/services/T000/B000/xxxxxxxxxxxx" },
  },
  {
    key: "oncall-pushover",
    project: "backups",
    kind: "pushover",
    name: "on-call phone",
    config: { token: "axxxxxxxxxxxxxxxxxxxxxxxxxxxxx", user: "uxxxxxxxxxxxxxxxxxxxxxxxxxxxxx" },
  },
  {
    key: "alerts-email",
    project: "backups",
    kind: "email",
    name: "alerts@example.com",
    config: { to: "alerts@example.com" },
  },
  {
    key: "pipeline-ntfy",
    project: "pipeline",
    kind: "ntfy",
    name: "ntfy · data-pipeline",
    config: { base_url: "https://ntfy.sh", topic: "data-pipeline" },
  },
];

const FAIL_BODIES = [
  "rsync: connection unexpectedly closed (0 bytes received so far)\nrsync error: error in rsync protocol data stream (code 12) at io.c(228)",
  "pg_dump: error: connection to server failed: FATAL:  the database system is in recovery mode",
  "zpool status: one or more devices has experienced an unrecoverable error\n  scrub repaired 0B with 3 errors",
];

const SOURCE_IPS = ["10.4.2.15", "10.4.2.31", "192.168.20.8"];

// Where the most recent finished run sits, relative to `now`, for each state.
// Expressed as a fraction of the check's cadence so a 30-minute job and a
// weekly one both look plausible.
function lastFinishOffset(check, rand) {
  const { cadence, grace } = check;
  switch (check.state) {
    case "down":
      // Comfortably past due — this is why the check is down.
      return cadence * 2 + grace + 900;
    case "late":
      // Past the expected time but still inside the grace window, so the scan
      // loop leaves it alone and `display_status` reports `late`.
      return cadence + grace * 0.5;
    case "running":
      // Finished one cadence ago; a fresh `start` (added below) is in flight.
      return cadence * 0.95;
    case "paused":
      return cadence * 0.4;
    default:
      return cadence * (0.15 + 0.35 * rand());
  }
}

/**
 * Build the whole demo dataset as one SQL script.
 * @param {number} nowMs epoch millis the data is anchored to ("now").
 */
export function generateSeedSql(nowMs) {
  const rand = mulberry32(20260722);
  const stmts = ["BEGIN;"];

  // ---- users -------------------------------------------------------------
  for (const [i, u] of EXTRA_USERS.entries()) {
    stmts.push(
      `INSERT INTO users (username, password_hash, is_admin, created_at)
         SELECT ${q(u.username)}, password_hash, ${u.is_admin ? 1 : 0}, ${q(iso(nowMs - (40 - i * 6) * DAY * 1000))}
         FROM users WHERE username = ${q(ADMIN_USERNAME)};`
    );
  }
  const ownerSql = (idx) =>
    `(SELECT id FROM users WHERE username = ${q([ADMIN_USERNAME, ...EXTRA_USERS.map((u) => u.username)][idx])})`;

  // ---- projects ----------------------------------------------------------
  for (const [i, p] of PROJECTS.entries()) {
    stmts.push(
      `INSERT INTO projects (user_id, name, description, scan_interval_secs, nag_interval_secs, created_at)
         VALUES (${ownerSql(p.owner)}, ${q(p.name)}, ${q(p.description)},
                 ${p.scan_interval_secs ?? "NULL"}, ${p.nag_interval_secs ?? "NULL"},
                 ${q(iso(nowMs - (60 - i * 5) * DAY * 1000))});`
    );
  }
  const projectSql = (key) =>
    `(SELECT id FROM projects WHERE name = ${q(PROJECTS.find((p) => p.key === key).name)})`;

  // ---- channels ----------------------------------------------------------
  for (const ch of CHANNELS) {
    stmts.push(
      `INSERT INTO channels (project_id, kind, name, config_json, created_at)
         VALUES (${projectSql(ch.project)}, ${q(ch.kind)}, ${q(ch.name)},
                 ${q(JSON.stringify(ch.config))}, ${q(iso(nowMs - 50 * DAY * 1000))});`
    );
  }
  const channelSql = (key) =>
    `(SELECT id FROM channels WHERE name = ${q(CHANNELS.find((c) => c.key === key).name)})`;

  // ---- checks, their ping history, and channel bindings ------------------
  // ping ids are assigned by AUTOINCREMENT in insertion order; the heartbeat
  // pairs each finish with the preceding `start` by timestamp, so inserting
  // each check's runs oldest-first is enough.
  const notifications = [];

  for (const c of CHECKS) {
    const tz = c.timezone ?? "UTC";
    // Cron checks anchor on a real fire time (see the evaluator above); period
    // checks anchor wherever their state wants the last run to sit.
    const finishAt =
      c.kind === "cron"
        ? lastFireAtOrBefore(c.cron, tz, nowMs)
        : nowMs - lastFinishOffset(c, rand) * 1000;
    const period = c.kind === "period" ? c.cadence : null;
    const nextDue =
      c.kind === "cron"
        ? nextFireAfter(c.cron, tz, finishAt) + c.grace * 1000
        : finishAt + (c.cadence + c.grace) * 1000;

    const status =
      c.state === "down" ? "down" : c.state === "paused" ? "paused" : "up";
    // `running` is a display status: an in-flight `start` newer than the last
    // finish, kept well inside max_runtime so the scan loop doesn't down it.
    const runningStart = c.state === "running" ? nowMs - 6 * 60 * 1000 : null;
    const lastStart = runningStart ?? finishAt - 60 * 1000;

    stmts.push(
      `INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs,
                           grace_secs, cron_expr, timezone, status, last_ping_at, last_start_at,
                           next_due_at, max_runtime_secs, last_alert_at, acknowledged, created_at)
         VALUES (${projectSql(c.project)}, ${q(c.name)}, ${q(c.description)}, ${q(uuidFor(c.key, rand))},
                 ${q(c.kind)}, ${period ?? "NULL"}, ${c.grace}, ${q(c.cron ?? null)}, ${q(tz)},
                 ${q(status)}, ${q(iso(finishAt))}, ${q(iso(lastStart))}, ${q(iso(nextDue))},
                 ${c.max_runtime ?? "NULL"},
                 ${c.state === "down" ? q(iso(nowMs - 10 * 60 * 1000)) : "NULL"}, 0,
                 ${q(iso(nowMs - 45 * DAY * 1000))});`
    );

    const checkSql = `(SELECT id FROM checks WHERE name = ${q(c.name)})`;
    for (const key of c.channels) {
      stmts.push(
        `INSERT INTO check_channels (check_id, channel_id) VALUES (${checkSql}, ${channelSql(key)});`
      );
    }

    // 34 runs, oldest first: enough to fill the 30-bar heartbeat strip.
    const RUNS = 34;
    const [lo, hi] = c.runtime;
    for (let i = RUNS - 1; i >= 0; i--) {
      const jitter = (rand() - 0.5) * c.cadence * 0.06;
      const end = finishAt - i * c.cadence * 1000 + jitter * 1000;
      // Every ~9th run is slow enough to paint an amber bar; runs 5 and 17 of
      // the down check failed outright, which is the red bar plus the captured
      // output row on the check page.
      const slow = i % 9 === 4;
      const failed = c.state === "down" && (i === 0 || i === 12);
      let dur = lo + (hi - lo) * rand();
      if (slow && c.max_runtime) dur = c.max_runtime * (0.82 + 0.1 * rand());
      const start = end - dur * 1000;
      const ip = SOURCE_IPS[Math.floor(rand() * SOURCE_IPS.length)];
      stmts.push(pingSql(checkSql, "start", start, "", ip));
      stmts.push(
        pingSql(
          checkSql,
          failed ? "fail" : "success",
          end,
          failed ? FAIL_BODIES[i % FAIL_BODIES.length] : "",
          ip
        )
      );
    }
    if (runningStart != null) {
      stmts.push(pingSql(checkSql, "start", runningStart, "", SOURCE_IPS[0]));
    }

    // Notification history: the down check's alert chain, and one earlier
    // recovered incident on the hourly sync so the table shows a mixed log.
    if (c.state === "down") {
      const chain = [
        ["down", "ops-slack", "ok", null, 62 * 60],
        ["down", "oncall-pushover", "ok", null, 62 * 60],
        ["down", "alerts-email", "error", "smtp: connection refused", 62 * 60],
        ["reminder", "ops-slack", "ok", null, 32 * 60],
        ["reminder", "oncall-pushover", "ok", null, 32 * 60],
      ];
      for (const [event, channel, st, err, agoSecs] of chain) {
        notifications.push([checkSql, channelSql(channel), event, st, err, nowMs - agoSecs * 1000]);
      }
    }
    if (c.key === "s3-sync") {
      notifications.push([checkSql, channelSql("ops-slack"), "down", "ok", null, nowMs - 26 * HOUR * 1000]);
      notifications.push([checkSql, channelSql("ops-slack"), "up", "ok", null, nowMs - 25.4 * HOUR * 1000]);
    }
  }

  for (const [check, channel, event, status, error, at] of notifications) {
    stmts.push(
      `INSERT INTO notifications (check_id, channel_id, event, status, error, created_at)
         VALUES (${check}, ${channel}, ${q(event)}, ${q(status)}, ${q(error)}, ${q(iso(at))});`
    );
  }

  // ---- global settings ---------------------------------------------------
  // Retention comfortably exceeds the backdated history: `prune_once` runs a
  // pass the moment the server boots.
  for (const [key, value] of [
    ["scan_interval", "30"],
    ["nag_interval", "1800"],
    ["pings_retention_days", "90"],
    ["notifications_retention_days", "90"],
  ]) {
    stmts.push(
      `INSERT INTO settings (key, value) VALUES (${q(key)}, ${q(value)})
         ON CONFLICT(key) DO UPDATE SET value = excluded.value;`
    );
  }

  stmts.push("COMMIT;");
  return stmts.join("\n");
}

function pingSql(checkSql, kind, atMs, body, ip) {
  return `INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at)
     VALUES (${checkSql}, ${q(kind)}, NULL, ${q(body)}, ${q(ip)}, ${q(iso(atMs))});`;
}

// Stable-looking but deterministic ping UUIDs: the URLs are rendered verbatim
// on the check page, so a fresh random one per run would make otherwise
// identical screenshots differ.
function uuidFor(key, rand) {
  const hex = (n) =>
    Array.from({ length: n }, () => "0123456789abcdef"[Math.floor(rand() * 16)]).join("");
  return `${hex(8)}-${hex(4)}-4${hex(3)}-a${hex(3)}-${hex(12)}`;
}
