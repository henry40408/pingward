use crate::db::Pool;
use crate::models::{
    ApiKey, AuditLog, Channel, ChannelKind, Check, CheckStatus, Notification, NotifyStatus, Ping,
    PingKind, Project, ScheduleKind, Session, User,
};
use crate::notify::EventKind;
use chrono::{DateTime, Utc};
use sqlx::Row;
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Clone)]
pub struct Store {
    pub pool: Pool,
}

/// Cross-user rollup of check statuses for the admin dashboard.
#[derive(Debug, Clone, Default)]
pub struct CheckStatusCounts {
    pub new: i64,
    pub up: i64,
    pub down: i64,
    pub paused: i64,
    /// Stored `up`/`new` checks with an in-flight `start` (mirrors
    /// `view::DisplayStatus::Running`). Not part of the `GROUP BY status`
    /// aggregate below — it's a display-status derivation, not a stored
    /// status — so it comes from a second, portable query instead of a
    /// `SUM(CASE ...)` folded into the first (whose result type differs
    /// across `SQLite`/`PostgreSQL` on the `Any` driver).
    pub running: i64,
}

/// A keyset cursor for paging the check-detail "recent pings"/"recent
/// notifications" tables. `id` ordering is used (not `created_at`) since it is
/// monotonic and covered by the `(check_id, id)` index — stable under
/// concurrent inserts, unlike offset pagination.
#[derive(Debug, Clone, Copy)]
pub enum PageCursor {
    /// The newest page — no cursor.
    Latest,
    /// Rows older than this id (paging "older").
    Before(i64),
    /// Rows newer than this id (paging back toward "newer").
    After(i64),
}

/// One page of keyset-paginated rows, always newest-first (`id DESC`) for
/// display regardless of which direction was queried.
#[derive(Debug)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// A row exists with `id` newer than the newest item on this page.
    pub has_newer: bool,
    /// A row exists with `id` older than the oldest item on this page.
    pub has_older: bool,
}

/// Filters for the check-detail "recent pings" table. Empty `kinds` and `None`
/// bounds mean "no constraint". The date bounds are compared against the
/// RFC3339 `created_at` text; the caller supplies them as UTC `DateTime`s and
/// they are re-serialized with `to_rfc3339()` here, so the lexicographic
/// comparison stays chronological (same basis as [`Store::delete_pings_before`]).
#[derive(Debug, Clone, Default)]
pub struct PingFilter {
    pub kinds: Vec<PingKind>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

impl PingFilter {
    /// True when no constraint is set — the default (unfiltered) page.
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty() && self.from.is_none() && self.to.is_none()
    }

    fn predicates(&self) -> Vec<Predicate> {
        let mut p = Vec::new();
        if !self.kinds.is_empty() {
            p.push(Predicate::TextIn(
                "kind",
                self.kinds.iter().map(|k| k.as_str().to_string()).collect(),
            ));
        }
        if let Some(f) = self.from {
            p.push(Predicate::TextCmp("created_at", ">=", f.to_rfc3339()));
        }
        if let Some(t) = self.to {
            p.push(Predicate::TextCmp("created_at", "<=", t.to_rfc3339()));
        }
        p
    }
}

/// Filters for the check-detail "recent notifications" table. `events` filters
/// the notify event (up/down/reminder) and `statuses` the delivery result
/// (ok/error); both empty means "no constraint". Date bounds behave as in
/// [`PingFilter`].
#[derive(Debug, Clone, Default)]
pub struct NotifFilter {
    pub events: Vec<EventKind>,
    pub statuses: Vec<NotifyStatus>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

impl NotifFilter {
    /// True when no constraint is set — the default (unfiltered) page.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
            && self.statuses.is_empty()
            && self.from.is_none()
            && self.to.is_none()
    }

    fn predicates(&self) -> Vec<Predicate> {
        let mut p = Vec::new();
        if !self.events.is_empty() {
            p.push(Predicate::TextIn(
                "event",
                self.events.iter().map(|e| e.as_str().to_string()).collect(),
            ));
        }
        if !self.statuses.is_empty() {
            p.push(Predicate::TextIn(
                "status",
                self.statuses
                    .iter()
                    .map(|s| s.as_str().to_string())
                    .collect(),
            ));
        }
        if let Some(f) = self.from {
            p.push(Predicate::TextCmp("created_at", ">=", f.to_rfc3339()));
        }
        if let Some(t) = self.to {
            p.push(Predicate::TextCmp("created_at", "<=", t.to_rfc3339()));
        }
        p
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewAudit<'a> {
    pub actor_user_id: i64,
    pub actor_username: &'a str,
    pub action: &'a str,
    pub target_type: Option<&'a str>,
    pub target_id: Option<i64>,
    pub target_owner_id: Option<i64>,
    pub method: Option<&'a str>,
    pub path: Option<&'a str>,
    pub detail: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct NewCheck<'a> {
    pub project_id: i64,
    pub name: &'a str,
    pub description: &'a str,
    pub ping_uuid: &'a str,
    pub kind: ScheduleKind,
    pub period_secs: Option<i64>,
    pub grace_secs: i64,
    pub cron_expr: Option<&'a str>,
    pub timezone: &'a str,
    pub scan_interval_secs: Option<i64>,
    pub max_runtime_secs: Option<i64>,
    pub nag_interval_secs: Option<i64>,
}

/// Not `#[derive(Default)]` like `NewAudit`: `ScheduleKind` is `str_enum!`-generated
/// and has no `Default`. `Period` matches the new-check form's own default.
impl Default for NewCheck<'_> {
    fn default() -> Self {
        Self {
            project_id: 0,
            name: "",
            description: "",
            ping_uuid: "",
            kind: ScheduleKind::Period,
            period_secs: None,
            grace_secs: 0,
            cron_expr: None,
            timezone: "",
            scan_interval_secs: None,
            max_runtime_secs: None,
            nag_interval_secs: None,
        }
    }
}

/// The new values for a check's schedule. Unlike [`NewCheck`], this has no
/// `Default`: on an INSERT an unset field just means "no value", but on an
/// UPDATE it would write the default over whatever is stored — defaulting
/// `name` would blank the check's name. Every caller must spell out all of it.
#[derive(Debug, Clone)]
pub struct UpdateCheck<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub kind: ScheduleKind,
    pub period_secs: Option<i64>,
    pub grace_secs: i64,
    pub cron_expr: Option<&'a str>,
    pub timezone: &'a str,
    pub scan_interval_secs: Option<i64>,
    pub max_runtime_secs: Option<i64>,
    pub nag_interval_secs: Option<i64>,
}

fn parse_ts(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|v| {
        DateTime::parse_from_rfc3339(&v)
            .ok()
            .map(|d| d.with_timezone(&Utc))
    })
}

fn decode_err(msg: impl Into<String>) -> sqlx::Error {
    sqlx::Error::Decode(Box::<dyn std::error::Error + Send + Sync>::from(msg.into()))
}

/// Fallible row mapping: a corrupt/unparsable enum or timestamp must surface
/// as an `Err` rather than panic, since a panic here (e.g. via
/// `list_active_checks` in the scan loop) would unwind and permanently kill
/// the spawned scan task.
fn row_to_check(row: &sqlx::any::AnyRow) -> Result<Check, sqlx::Error> {
    let schedule_kind_raw: String = row.get("schedule_kind");
    let schedule_kind = ScheduleKind::from_str(&schedule_kind_raw)
        .map_err(|e| decode_err(format!("invalid schedule_kind {schedule_kind_raw:?}: {e}")))?;

    let status_raw: String = row.get("status");
    let status = CheckStatus::from_str(&status_raw)
        .map_err(|e| decode_err(format!("invalid status {status_raw:?}: {e}")))?;

    let created_at = parse_ts(row.get("created_at"))
        .ok_or_else(|| decode_err("created_at must be valid RFC3339"))?;

    Ok(Check {
        id: row.get("id"),
        project_id: row.get("project_id"),
        name: row.get("name"),
        description: row.get("description"),
        ping_uuid: row.get("ping_uuid"),
        schedule_kind,
        period_secs: row.get("period_secs"),
        grace_secs: row.get("grace_secs"),
        cron_expr: row.get("cron_expr"),
        timezone: row.get("timezone"),
        status,
        last_ping_at: parse_ts(row.get("last_ping_at")),
        last_start_at: parse_ts(row.get("last_start_at")),
        next_due_at: parse_ts(row.get("next_due_at")),
        scan_interval_secs: row.get("scan_interval_secs"),
        max_runtime_secs: row.get("max_runtime_secs"),
        nag_interval_secs: row.get("nag_interval_secs"),
        last_alert_at: parse_ts(row.get("last_alert_at")),
        acknowledged: row.get::<i64, _>("acknowledged") != 0,
        created_at,
    })
}

fn row_to_user(row: &sqlx::any::AnyRow) -> Result<User, sqlx::Error> {
    Ok(User {
        id: row.get("id"),
        username: row.get("username"),
        password_hash: row.get("password_hash"),
        is_admin: row.get::<i64, _>("is_admin") != 0,
        disabled: row.get::<i64, _>("disabled") != 0,
        created_at: parse_ts(row.get("created_at"))
            .ok_or_else(|| decode_err("users.created_at must be RFC3339"))?,
    })
}

fn row_to_api_key(row: &sqlx::any::AnyRow) -> Result<ApiKey, sqlx::Error> {
    Ok(ApiKey {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        prefix: row.get("prefix"),
        created_at: parse_ts(row.get("created_at"))
            .ok_or_else(|| decode_err("api_keys.created_at must be RFC3339"))?,
        last_used_at: parse_ts(row.get("last_used_at")),
        expires_at: parse_ts(row.get("expires_at")),
    })
}

fn row_to_session(row: &sqlx::any::AnyRow) -> Result<Session, sqlx::Error> {
    Ok(Session {
        id: row.get("id"),
        user_id: row.get("user_id"),
        created_at: parse_ts(row.get("created_at")),
        last_seen_at: parse_ts(row.get("last_seen_at")),
        expires_at: parse_ts(row.get("expires_at"))
            .ok_or_else(|| decode_err("sessions.expires_at must be RFC3339"))?,
        user_agent: row.get("user_agent"),
        ip: row.get("ip"),
    })
}

fn row_to_project(row: &sqlx::any::AnyRow) -> Result<Project, sqlx::Error> {
    Ok(Project {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        description: row.get("description"),
        scan_interval_secs: row.get("scan_interval_secs"),
        nag_interval_secs: row.get("nag_interval_secs"),
        created_at: parse_ts(row.get("created_at"))
            .ok_or_else(|| decode_err("projects.created_at must be RFC3339"))?,
    })
}

fn row_to_channel(row: &sqlx::any::AnyRow) -> Result<Channel, sqlx::Error> {
    let kind_raw: String = row.get("kind");
    let kind = ChannelKind::from_str(&kind_raw)
        .map_err(|e| decode_err(format!("invalid channel kind {kind_raw:?}: {e}")))?;
    Ok(Channel {
        id: row.get("id"),
        project_id: row.get("project_id"),
        kind,
        name: row.get("name"),
        config_json: row.get("config_json"),
        created_at: parse_ts(row.get("created_at"))
            .ok_or_else(|| decode_err("channels.created_at must be RFC3339"))?,
    })
}

fn row_to_ping(row: &sqlx::any::AnyRow) -> Result<Ping, sqlx::Error> {
    let kind_raw: String = row.get("kind");
    let kind = PingKind::from_str(&kind_raw)
        .map_err(|e| decode_err(format!("invalid ping kind {kind_raw:?}: {e}")))?;
    Ok(Ping {
        id: row.get("id"),
        check_id: row.get("check_id"),
        kind,
        exit_code: row.get("exit_code"),
        body: row.get("body"),
        source_ip: row.get("source_ip"),
        created_at: parse_ts(row.get("created_at"))
            .ok_or_else(|| decode_err("pings.created_at must be RFC3339"))?,
    })
}

fn row_to_notification(row: &sqlx::any::AnyRow) -> Result<Notification, sqlx::Error> {
    let event_raw: String = row.get("event");
    let event = EventKind::from_str(&event_raw)
        .map_err(|e| decode_err(format!("invalid notification event {event_raw:?}: {e}")))?;
    let status_raw: String = row.get("status");
    let status = NotifyStatus::from_str(&status_raw)
        .map_err(|e| decode_err(format!("invalid notification status {status_raw:?}: {e}")))?;
    Ok(Notification {
        id: row.get("id"),
        check_id: row.get("check_id"),
        channel_id: row.get("channel_id"),
        event,
        status,
        error: row.get("error"),
        created_at: parse_ts(row.get("created_at"))
            .ok_or_else(|| decode_err("notifications.created_at must be RFC3339"))?,
    })
}

fn row_to_audit(row: &sqlx::any::AnyRow) -> Result<AuditLog, sqlx::Error> {
    Ok(AuditLog {
        id: row.get("id"),
        actor_user_id: row.get("actor_user_id"),
        actor_username: row.get("actor_username"),
        action: row.get("action"),
        target_type: row.get("target_type"),
        target_id: row.get("target_id"),
        target_owner_id: row.get("target_owner_id"),
        method: row.get("method"),
        path: row.get("path"),
        detail: row.get("detail"),
        created_at: parse_ts(row.get("created_at"))
            .ok_or_else(|| decode_err("audit_log.created_at must be RFC3339"))?,
    })
}

/// A value bound into a filtered keyset query. Columns, operators, and
/// placeholders are always self-generated literals; only these values cross the
/// query boundary as bound parameters, so the assembled SQL has no injection
/// surface.
enum QueryBind {
    Int(i64),
    Text(String),
}

/// One extra `WHERE` predicate layered onto a keyset page (a filter). The
/// column name and operator are fixed caller literals; values are always bound.
enum Predicate {
    /// `col op $n` with a text value (e.g. `created_at >= $n`).
    TextCmp(&'static str, &'static str, String),
    /// `col IN ($a,$b,…)` over text values. An empty `vals` is skipped by the
    /// filter builders, so this never renders as `IN ()`.
    TextIn(&'static str, Vec<String>),
}

/// Shared keyset-pagination core for `pings`/`notifications`: both tables are
/// paged the same way (by `id`, scoped to `check_id`), so the SQL shape and
/// `has_newer/has_older` bookkeeping live here once. `table` and every predicate
/// column/operator are fixed caller literals (never user input); all values —
/// including filter values — are bound, so interpolating the assembled clause
/// into the query text is safe. Uses a limit+1 fetch to detect another page in
/// the queried direction rather than a separate `COUNT(*)`.
#[allow(
    clippy::cast_sign_loss,
    reason = "`limit` is a small positive page size supplied by callers, never negative"
)]
async fn keyset_page<T>(
    pool: &crate::db::Pool,
    table: &'static str,
    check_id: i64,
    cursor: PageCursor,
    limit: i64,
    filters: &[Predicate],
    row_to: fn(&sqlx::any::AnyRow) -> Result<T, sqlx::Error>,
) -> Result<Page<T>, sqlx::Error> {
    let fetch_limit = limit + 1;
    let mut conds: Vec<String> = Vec::new();
    let mut binds: Vec<QueryBind> = Vec::new();

    // $1 is always the check scope.
    conds.push("check_id = $1".to_string());
    binds.push(QueryBind::Int(check_id));

    // Filter predicates, each allocating fresh placeholders in bind order.
    for f in filters {
        match f {
            Predicate::TextCmp(col, op, v) => {
                binds.push(QueryBind::Text(v.clone()));
                conds.push(format!("{col} {op} ${}", binds.len()));
            }
            Predicate::TextIn(col, vals) => {
                let phs: Vec<String> = vals
                    .iter()
                    .map(|v| {
                        binds.push(QueryBind::Text(v.clone()));
                        format!("${}", binds.len())
                    })
                    .collect();
                conds.push(format!("{col} IN ({})", phs.join(",")));
            }
        }
    }

    // The cursor predicate + scan direction. Latest carries no cursor bound.
    let order = match cursor {
        PageCursor::Latest => "DESC",
        PageCursor::Before(id) => {
            binds.push(QueryBind::Int(id));
            conds.push(format!("id < ${}", binds.len()));
            "DESC"
        }
        PageCursor::After(id) => {
            binds.push(QueryBind::Int(id));
            conds.push(format!("id > ${}", binds.len()));
            "ASC"
        }
    };

    binds.push(QueryBind::Int(fetch_limit));
    let sql = format!(
        "SELECT * FROM {table} WHERE {} ORDER BY id {order} LIMIT ${}",
        conds.join(" AND "),
        binds.len()
    );
    // Safe: `table`, every column, operator, and placeholder are self-generated
    // literals; all values (check_id, filter values, cursor, limit) are bound.
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
    for b in &binds {
        q = match b {
            QueryBind::Int(i) => q.bind(*i),
            QueryBind::Text(s) => q.bind(s.clone()),
        };
    }
    let mut rows = q.fetch_all(pool).await?;

    // Interpret the limit+1 overflow row per direction. Before/After always
    // came from an existing adjacent page, so the opposite-direction flag is
    // known true regardless of the filter set.
    match cursor {
        PageCursor::Latest => {
            let has_older = rows.len() as i64 > limit;
            let items = rows
                .iter()
                .take(limit as usize)
                .map(row_to)
                .collect::<Result<Vec<T>, _>>()?;
            Ok(Page {
                items,
                has_newer: false,
                has_older,
            })
        }
        PageCursor::Before(_) => {
            let has_older = rows.len() as i64 > limit;
            let items = rows
                .iter()
                .take(limit as usize)
                .map(row_to)
                .collect::<Result<Vec<T>, _>>()?;
            Ok(Page {
                items,
                has_newer: true,
                has_older,
            })
        }
        PageCursor::After(_) => {
            let has_newer = rows.len() as i64 > limit;
            if has_newer {
                // Rows are ASC by id; the last row is the farthest-from-cursor
                // (newest) overflow row — drop it, keeping the `limit` rows
                // closest to the cursor.
                rows.pop();
            }
            let mut items = rows.iter().map(row_to).collect::<Result<Vec<T>, _>>()?;
            items.reverse(); // ASC -> newest-first (id DESC) for display
            Ok(Page {
                items,
                has_newer,
                has_older: true,
            })
        }
    }
}

impl Store {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    pub async fn find_check_by_uuid(&self, uuid: &str) -> Result<Option<Check>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM checks WHERE ping_uuid = $1")
            .bind(uuid)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(row_to_check).transpose()
    }

    /// One corrupt/unparsable row must never abort the whole scan: rows that
    /// fail to decode are logged and skipped rather than propagated as an
    /// error or allowed to panic.
    pub async fn list_active_checks(&self) -> Result<Vec<Check>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM checks WHERE status IN ('new','up')")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            match row_to_check(row) {
                Ok(check) => out.push(check),
                Err(e) => {
                    let id: i64 = row.get("id");
                    tracing::error!("skipping corrupt checks row id={id}: {e}");
                }
            }
        }
        Ok(out)
    }

    /// Checks currently in `down` status — the candidates for nag reminders.
    /// Corrupt rows are logged and skipped, mirroring `list_active_checks`.
    pub async fn list_down_checks(&self) -> Result<Vec<Check>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM checks WHERE status = 'down'")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            match row_to_check(row) {
                Ok(check) => out.push(check),
                Err(e) => {
                    let id: i64 = row.get("id");
                    tracing::error!("skipping corrupt checks row id={id}: {e}");
                }
            }
        }
        Ok(out)
    }

    pub async fn insert_ping(
        &self,
        check_id: i64,
        kind: PingKind,
        exit_code: Option<i64>,
        body: &str,
        source_ip: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at) VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(check_id).bind(kind.as_str()).bind(exit_code)
        .bind(body).bind(source_ip).bind(now.to_rfc3339())
        .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn mark_ping(
        &self,
        check_id: i64,
        status: CheckStatus,
        last_ping_at: Option<DateTime<Utc>>,
        last_start_at: Option<DateTime<Utc>>,
        next_due_at: Option<DateTime<Utc>>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE checks SET status=$1, last_ping_at=COALESCE($2, last_ping_at), \
             last_start_at=COALESCE($3, last_start_at), next_due_at=$4 WHERE id=$5",
        )
        .bind(status.as_str())
        .bind(last_ping_at.map(|d| d.to_rfc3339()))
        .bind(last_start_at.map(|d| d.to_rfc3339()))
        .bind(next_due_at.map(|d| d.to_rfc3339()))
        .bind(check_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_status(&self, check_id: i64, status: CheckStatus) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE checks SET status=$1 WHERE id=$2")
            .bind(status.as_str())
            .bind(check_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Mark the start of a down incident's alerting: stamp the alert baseline
    /// and clear any prior acknowledgement so a fresh incident is never silent.
    pub async fn begin_down_alert(
        &self,
        check_id: i64,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE checks SET last_alert_at=$1, acknowledged=0 WHERE id=$2")
            .bind(at.to_rfc3339())
            .bind(check_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Advance the alert baseline after emitting a reminder.
    pub async fn record_reminder(
        &self,
        check_id: i64,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE checks SET last_alert_at=$1 WHERE id=$2")
            .bind(at.to_rfc3339())
            .bind(check_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Clear nag state on recovery: no acknowledgement, no alert baseline.
    pub async fn clear_nag(&self, check_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE checks SET acknowledged=0, last_alert_at=NULL WHERE id=$1")
            .bind(check_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Silence reminders for the current down incident.
    pub async fn acknowledge(&self, check_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE checks SET acknowledged=1 WHERE id=$1")
            .bind(check_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn create_check(&self, c: &NewCheck<'_>) -> Result<i64, sqlx::Error> {
        let row = sqlx::query(
            "INSERT INTO checks (project_id, name, description, ping_uuid, schedule_kind, period_secs, \
             grace_secs, cron_expr, timezone, scan_interval_secs, max_runtime_secs, \
             nag_interval_secs, status, created_at) VALUES \
             ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'new',$13) RETURNING id",
        )
        .bind(c.project_id)
        .bind(c.name)
        .bind(c.description)
        .bind(c.ping_uuid)
        .bind(c.kind.as_str())
        .bind(c.period_secs)
        .bind(c.grace_secs)
        .bind(c.cron_expr)
        .bind(c.timezone)
        .bind(c.scan_interval_secs)
        .bind(c.max_runtime_secs)
        .bind(c.nag_interval_secs)
        .bind(Utc::now().to_rfc3339())
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get::<i64, _>("id"))
    }

    pub async fn count_users(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await
    }

    pub async fn create_user(
        &self,
        username: &str,
        password_hash: Option<&str>,
        is_admin: bool,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query(
            "INSERT INTO users (username, password_hash, is_admin, created_at) VALUES ($1,$2,$3,$4) RETURNING id",
        )
        .bind(username)
        .bind(password_hash)
        .bind(is_admin as i64)
        .bind(now.to_rfc3339())
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get::<i64, _>("id"))
    }

    pub async fn find_user_by_username(&self, username: &str) -> Result<Option<User>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM users WHERE username = $1")
            .bind(username)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(row_to_user).transpose()
    }

    pub async fn find_user_by_id(&self, id: i64) -> Result<Option<User>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(row_to_user).transpose()
    }

    pub async fn list_users(&self) -> Result<Vec<User>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM users ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_user).collect()
    }

    pub async fn delete_user(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_user_disabled(&self, id: i64, disabled: bool) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE users SET disabled = $1 WHERE id = $2")
            .bind(disabled as i64)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_user_password(&self, id: i64, password_hash: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
            .bind(password_hash)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_user_admin(&self, id: i64, is_admin: bool) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE users SET is_admin = $1 WHERE id = $2")
            .bind(is_admin as i64)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn count_enabled_admins(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE is_admin <> 0 AND disabled = 0")
            .fetch_one(&self.pool)
            .await
    }

    /// Persist a new API key. Only the `token_hash` and non-secret `prefix` are
    /// stored — the plaintext is never seen here.
    pub async fn insert_api_key(
        &self,
        user_id: i64,
        name: &str,
        token_hash: &str,
        prefix: &str,
        expires_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query(
            "INSERT INTO api_keys (user_id, name, token_hash, prefix, created_at, expires_at) \
             VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
        )
        .bind(user_id)
        .bind(name)
        .bind(token_hash)
        .bind(prefix)
        .bind(now.to_rfc3339())
        .bind(expires_at.map(|t| t.to_rfc3339()))
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get::<i64, _>("id"))
    }

    /// List a user's API keys (metadata only), newest first.
    pub async fn list_api_keys_for_user(&self, user_id: i64) -> Result<Vec<ApiKey>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM api_keys WHERE user_id = $1 ORDER BY id DESC")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_api_key).collect()
    }

    /// Delete an API key, scoped to its owner. Returns `true` if a row was
    /// removed — `false` means the key does not exist or belongs to another
    /// user (existence is not distinguished, mirroring the 404-hiding model).
    pub async fn delete_api_key(&self, id: i64, user_id: i64) -> Result<bool, sqlx::Error> {
        let res = sqlx::query("DELETE FROM api_keys WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Resolve a token hash to its owning user id, honoring expiry. Returns
    /// `None` for an unknown or expired key. On a successful match the key's
    /// `last_used_at` is refreshed, but writes are throttled to at most once per
    /// 60s so a hot key doesn't cause a write per request.
    pub async fn validate_api_key(
        &self,
        token_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<i64>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, user_id, expires_at, last_used_at FROM api_keys WHERE token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        if let Some(exp) = parse_ts(row.get("expires_at"))
            && exp <= now
        {
            return Ok(None);
        }
        let id: i64 = row.get("id");
        let user_id: i64 = row.get("user_id");
        let stale = parse_ts(row.get("last_used_at"))
            .is_none_or(|t| now - t >= chrono::Duration::seconds(60));
        if stale {
            sqlx::query("UPDATE api_keys SET last_used_at = $1 WHERE id = $2")
                .bind(now.to_rfc3339())
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        Ok(Some(user_id))
    }

    #[allow(clippy::too_many_arguments, reason = "mirrors the sessions row shape")]
    pub async fn create_session(
        &self,
        id: &str,
        user_id: i64,
        csrf_token: &str,
        expires_at: DateTime<Utc>,
        user_agent: Option<&str>,
        ip: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO sessions (id, user_id, csrf_token, expires_at, created_at, user_agent, ip) \
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(id)
        .bind(user_id)
        .bind(csrf_token)
        .bind(expires_at.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(user_agent)
        .bind(ip)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Look up the CSRF synchronizer token stored alongside a session row.
    /// Returns `None` when the session does not exist.
    pub async fn session_csrf_token(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT csrf_token FROM sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
    }

    /// Resolve a session to its owning user, honoring expiry. On a match the
    /// session's `last_seen_at` is refreshed, but writes are throttled to at
    /// most once per 60s (mirroring [`Store::validate_api_key`]'s
    /// `last_used_at` throttle) so a hot session doesn't cause a write per
    /// request.
    pub async fn find_session_user(
        &self,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<User>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT u.*, s.last_seen_at AS session_last_seen_at FROM sessions s \
             JOIN users u ON u.id = s.user_id \
             WHERE s.id = $1 AND s.expires_at > $2",
        )
        .bind(session_id)
        .bind(now.to_rfc3339())
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let stale = parse_ts(row.get("session_last_seen_at"))
            .is_none_or(|t| now - t >= chrono::Duration::seconds(60));
        if stale {
            sqlx::query("UPDATE sessions SET last_seen_at = $1 WHERE id = $2")
                .bind(now.to_rfc3339())
                .bind(session_id)
                .execute(&self.pool)
                .await?;
        }
        Ok(Some(row_to_user(&row)?))
    }

    pub async fn delete_session(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM sessions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// List a user's currently-valid sessions (not expired), newest-created
    /// first, for the `/account` management page.
    pub async fn list_sessions_for_user(
        &self,
        user_id: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<Session>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT * FROM sessions WHERE user_id = $1 AND expires_at > $2 \
             ORDER BY created_at DESC, id",
        )
        .bind(user_id)
        .bind(now.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_session).collect()
    }

    /// Delete a session, scoped to its owner. Returns `true` if a row was
    /// removed — `false` means the session does not exist or belongs to
    /// another user (existence is not distinguished, mirroring
    /// [`Store::delete_api_key`]'s 404-hiding model).
    pub async fn delete_session_owned(&self, id: &str, user_id: i64) -> Result<bool, sqlx::Error> {
        let res = sqlx::query("DELETE FROM sessions WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Delete every session for `user_id` except `keep_id` ("revoke all other
    /// sessions"). Returns the number of sessions removed.
    pub async fn delete_other_sessions_for_user(
        &self,
        user_id: i64,
        keep_id: &str,
    ) -> Result<u64, sqlx::Error> {
        let res = sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND id <> $2")
            .bind(user_id)
            .bind(keep_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }

    // --- projects ---
    pub async fn create_project(
        &self,
        user_id: i64,
        name: &str,
        description: &str,
        scan_interval_secs: Option<i64>,
        nag_interval_secs: Option<i64>,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query(
            "INSERT INTO projects (user_id, name, description, scan_interval_secs, nag_interval_secs, created_at) \
             VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
        )
        .bind(user_id)
        .bind(name)
        .bind(description)
        .bind(scan_interval_secs)
        .bind(nag_interval_secs)
        .bind(now.to_rfc3339())
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get::<i64, _>("id"))
    }

    pub async fn find_project(&self, id: i64) -> Result<Option<Project>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM projects WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(row_to_project).transpose()
    }

    /// Every project paired with its owner's username, for the admin
    /// cross-user projects list. Ordered by project id.
    pub async fn list_all_projects_with_owner(
        &self,
    ) -> Result<Vec<(Project, String)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT p.*, u.username AS owner_username \
             FROM projects p JOIN users u ON u.id = p.user_id ORDER BY p.id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| Ok((row_to_project(r)?, r.get::<String, _>("owner_username"))))
            .collect()
    }

    pub async fn list_projects_for_user(&self, user_id: i64) -> Result<Vec<Project>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM projects WHERE user_id = $1 ORDER BY id")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_project).collect()
    }

    pub async fn update_project(
        &self,
        id: i64,
        name: &str,
        description: &str,
        scan_interval_secs: Option<i64>,
        nag_interval_secs: Option<i64>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE projects SET name = $1, description = $2, scan_interval_secs = $3, nag_interval_secs = $4 WHERE id = $5",
        )
        .bind(name)
        .bind(description)
        .bind(scan_interval_secs)
        .bind(nag_interval_secs)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_project(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM projects WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn all_project_scan_intervals(
        &self,
    ) -> Result<HashMap<i64, Option<i64>>, sqlx::Error> {
        let rows = sqlx::query("SELECT id, scan_interval_secs FROM projects")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get::<i64, _>("id"),
                    r.get::<Option<i64>, _>("scan_interval_secs"),
                )
            })
            .collect())
    }

    pub async fn all_project_nag_intervals(
        &self,
    ) -> Result<HashMap<i64, Option<i64>>, sqlx::Error> {
        let rows = sqlx::query("SELECT id, nag_interval_secs FROM projects")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get::<i64, _>("id"),
                    r.get::<Option<i64>, _>("nag_interval_secs"),
                )
            })
            .collect())
    }

    // --- channels ---
    pub async fn create_channel(
        &self,
        project_id: i64,
        kind: ChannelKind,
        name: &str,
        config_json: &str,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query(
            "INSERT INTO channels (project_id, kind, name, config_json, created_at) VALUES ($1,$2,$3,$4,$5) \
             RETURNING id",
        )
        .bind(project_id).bind(kind.as_str()).bind(name).bind(config_json).bind(now.to_rfc3339())
        .fetch_one(&self.pool).await?;
        Ok(row.get::<i64, _>("id"))
    }

    pub async fn find_channel(&self, id: i64) -> Result<Option<Channel>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM channels WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(row_to_channel).transpose()
    }

    pub async fn list_channels_for_project(
        &self,
        project_id: i64,
    ) -> Result<Vec<Channel>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM channels WHERE project_id = $1 ORDER BY id")
            .bind(project_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_channel).collect()
    }

    pub async fn delete_channel(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM channels WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // --- bindings ---
    pub async fn bind_channel(&self, check_id: i64, channel_id: i64) -> Result<(), sqlx::Error> {
        // `INSERT OR IGNORE` is SQLite-only syntax and is a parse error on
        // Postgres; `ON CONFLICT DO NOTHING` is portable to both backends and
        // relies on the `(check_id, channel_id)` primary key as the conflict
        // target.
        sqlx::query(
            "INSERT INTO check_channels (check_id, channel_id) VALUES ($1,$2) \
             ON CONFLICT DO NOTHING",
        )
        .bind(check_id)
        .bind(channel_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn unbind_channel(&self, check_id: i64, channel_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM check_channels WHERE check_id = $1 AND channel_id = $2")
            .bind(check_id)
            .bind(channel_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn bound_channel_ids(&self, check_id: i64) -> Result<Vec<i64>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT channel_id FROM check_channels WHERE check_id = $1 ORDER BY channel_id",
        )
        .bind(check_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| r.get::<i64, _>("channel_id")).collect())
    }

    pub async fn channels_for_check(&self, check_id: i64) -> Result<Vec<Channel>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT c.* FROM channels c JOIN check_channels cc ON cc.channel_id = c.id \
             WHERE cc.check_id = $1 ORDER BY c.id",
        )
        .bind(check_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_channel).collect()
    }

    // --- checks (web) ---
    pub async fn find_check(&self, id: i64) -> Result<Option<Check>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM checks WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(row_to_check).transpose()
    }

    pub async fn list_checks_for_project(
        &self,
        project_id: i64,
    ) -> Result<Vec<Check>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM checks WHERE project_id = $1 ORDER BY id")
            .bind(project_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_check).collect()
    }

    pub async fn update_check_schedule(
        &self,
        id: i64,
        c: &UpdateCheck<'_>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE checks SET name=$1, description=$2, schedule_kind=$3, period_secs=$4, grace_secs=$5, \
             cron_expr=$6, timezone=$7, scan_interval_secs=$8, max_runtime_secs=$9, \
             nag_interval_secs=$10 WHERE id=$11",
        )
        .bind(c.name)
        .bind(c.description)
        .bind(c.kind.as_str())
        .bind(c.period_secs)
        .bind(c.grace_secs)
        .bind(c.cron_expr)
        .bind(c.timezone)
        .bind(c.scan_interval_secs)
        .bind(c.max_runtime_secs)
        .bind(c.nag_interval_secs)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn regenerate_uuid(&self, id: i64, new_uuid: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE checks SET ping_uuid = $1 WHERE id = $2")
            .bind(new_uuid)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_check(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM checks WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // --- pings / notifications ---
    pub async fn list_recent_pings(
        &self,
        check_id: i64,
        limit: i64,
    ) -> Result<Vec<Ping>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM pings WHERE check_id = $1 ORDER BY id DESC LIMIT $2")
            .bind(check_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_ping).collect()
    }

    /// Batched form of [`list_recent_pings`]: fetch the most recent
    /// `per_check_limit` pings (newest id first) for each of `check_ids` in a
    /// single round-trip, keyed by `check_id`. Avoids the per-check N+1 the
    /// dashboard would otherwise incur. Checks with no pings are simply absent
    /// from the map. Uses a `ROW_NUMBER()` window (`SQLite` >= 3.25 / `PostgreSQL`).
    pub async fn list_recent_pings_for_checks(
        &self,
        check_ids: &[i64],
        per_check_limit: i64,
    ) -> Result<HashMap<i64, Vec<Ping>>, sqlx::Error> {
        if check_ids.is_empty() {
            return Ok(HashMap::new());
        }
        // Placeholders `$1..$N` for the IN list; the limit is the final param.
        let placeholders = (1..=check_ids.len())
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT * FROM ( \
               SELECT p.*, ROW_NUMBER() OVER (PARTITION BY p.check_id ORDER BY p.id DESC) AS rn \
               FROM pings p WHERE p.check_id IN ({placeholders}) \
             ) sub WHERE rn <= ${} ORDER BY check_id, id DESC",
            check_ids.len() + 1
        );
        // Safe: `sql` interpolates only self-generated `$N` placeholders and a
        // count — every value is bound below, so there is no injection surface.
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
        for id in check_ids {
            q = q.bind(*id);
        }
        q = q.bind(per_check_limit);
        let rows = q.fetch_all(&self.pool).await?;
        let mut map: HashMap<i64, Vec<Ping>> = HashMap::new();
        for row in &rows {
            let ping = row_to_ping(row)?;
            map.entry(ping.check_id).or_default().push(ping);
        }
        Ok(map)
    }

    /// Keyset-paginated page of a check's pings for the check-detail table
    /// (newest-first), narrowed by `filter`. See [`PageCursor`]/[`Page`]/
    /// [`PingFilter`]. Independent of [`Store::list_recent_pings`], which the
    /// heartbeat strip uses and which must never be affected by table paging.
    pub async fn list_pings_page(
        &self,
        check_id: i64,
        cursor: PageCursor,
        limit: i64,
        filter: &PingFilter,
    ) -> Result<Page<Ping>, sqlx::Error> {
        keyset_page(
            &self.pool,
            "pings",
            check_id,
            cursor,
            limit,
            &filter.predicates(),
            row_to_ping,
        )
        .await
    }

    /// Keyset-paginated page of a check's notifications for the check-detail
    /// table (newest-first), narrowed by `filter`. See [`PageCursor`]/[`Page`]/
    /// [`NotifFilter`].
    pub async fn list_notifications_page(
        &self,
        check_id: i64,
        cursor: PageCursor,
        limit: i64,
        filter: &NotifFilter,
    ) -> Result<Page<Notification>, sqlx::Error> {
        keyset_page(
            &self.pool,
            "notifications",
            check_id,
            cursor,
            limit,
            &filter.predicates(),
            row_to_notification,
        )
        .await
    }

    pub async fn record_notification(
        &self,
        check_id: i64,
        channel_id: i64,
        event: EventKind,
        status: NotifyStatus,
        error: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO notifications (check_id, channel_id, event, status, error, created_at) \
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(check_id)
        .bind(channel_id)
        .bind(event.as_str())
        .bind(status.as_str())
        .bind(error)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_recent_notifications(
        &self,
        check_id: i64,
        limit: i64,
    ) -> Result<Vec<Notification>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT * FROM notifications WHERE check_id = $1 ORDER BY id DESC LIMIT $2",
        )
        .bind(check_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_notification).collect()
    }

    /// Delete pings older than `cutoff` (an RFC3339 timestamp). Returns the
    /// number of rows removed. `created_at` is TEXT RFC3339 (UTC), so the
    /// lexicographic `<` comparison is chronological on both backends.
    pub async fn delete_pings_before(&self, cutoff: &str) -> Result<u64, sqlx::Error> {
        let r = sqlx::query("DELETE FROM pings WHERE created_at < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    /// Delete notifications older than `cutoff` (an RFC3339 timestamp). Returns
    /// the number of rows removed.
    pub async fn delete_notifications_before(&self, cutoff: &str) -> Result<u64, sqlx::Error> {
        let r = sqlx::query("DELETE FROM notifications WHERE created_at < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    /// Delete sessions whose `expires_at` has passed. Expired sessions are already
    /// unusable (`list_sessions_for_user` and session lookup both require
    /// `expires_at > now`), so this is unconditional rather than retention-driven.
    /// Returns the number of rows removed.
    pub async fn delete_expired_sessions(&self, now: &str) -> Result<u64, sqlx::Error> {
        let r = sqlx::query("DELETE FROM sessions WHERE expires_at <= $1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    // --- settings ---
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT value FROM settings WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ($1,$2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // --- audit log ---
    pub async fn record_audit(
        &self,
        e: &NewAudit<'_>,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query(
            "INSERT INTO audit_log \
             (actor_user_id, actor_username, action, target_type, target_id, \
              target_owner_id, method, path, detail, created_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING id",
        )
        .bind(e.actor_user_id)
        .bind(e.actor_username)
        .bind(e.action)
        .bind(e.target_type)
        .bind(e.target_id)
        .bind(e.target_owner_id)
        .bind(e.method)
        .bind(e.path)
        .bind(e.detail)
        .bind(now.to_rfc3339())
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get::<i64, _>("id"))
    }

    pub async fn list_audit(&self, limit: i64) -> Result<Vec<AuditLog>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM audit_log ORDER BY id DESC LIMIT $1")
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_audit).collect()
    }

    // --- admin dashboard aggregates ---

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
        // `status IN ('up','new')` is what keeps this consistent with
        // `view::display_status`'s precedence: Running only ever applies on
        // top of a stored up/new check.
        c.running = sqlx::query_scalar(
            "SELECT COUNT(*) FROM checks \
             WHERE status IN ('up','new') AND last_start_at IS NOT NULL \
             AND (last_ping_at IS NULL OR last_start_at > last_ping_at)",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(c)
    }

    /// Every check currently `down`, paired with its project name and owner
    /// username, for the admin cross-user incidents view. Ordered by
    /// `last_ping_at` (oldest first, i.e. longest-down first), with
    /// never-pinged checks (`NULL`) sorted last on both `SQLite` and
    /// `PostgreSQL`, and `id` as a deterministic final tiebreaker.
    pub async fn list_down_checks_with_owner(
        &self,
    ) -> Result<Vec<(Check, String, String)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT c.*, p.name AS project_name, u.username AS owner_username \
             FROM checks c JOIN projects p ON p.id = c.project_id \
             JOIN users u ON u.id = p.user_id \
             WHERE c.status = 'down' ORDER BY c.last_ping_at IS NULL, c.last_ping_at, c.id",
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

    /// `(ok, error)` notification counts across all users since `cutoff`.
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

    /// Per-channel `(channel_name, ok, error)` notification counts since
    /// `cutoff`, ordered by most failures first. The conditional sums are
    /// cast to `BIGINT` so they decode as `i64` on both `SQLite` and
    /// `PostgreSQL` (a bare `SUM()` can come back as a wider/different type
    /// on `PostgreSQL` and fail to decode as `i64`).
    pub async fn channel_failure_counts_since(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<(String, i64, i64)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT ch.name AS channel_name, \
             CAST(SUM(CASE WHEN n.status = 'ok' THEN 1 ELSE 0 END) AS BIGINT) AS ok, \
             CAST(SUM(CASE WHEN n.status = 'error' THEN 1 ELSE 0 END) AS BIGINT) AS err \
             FROM notifications n JOIN channels ch ON ch.id = n.channel_id \
             WHERE n.created_at >= $1 GROUP BY ch.id, ch.name ORDER BY err DESC, ch.id",
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
        sqlx::query_scalar("SELECT COUNT(*) FROM projects")
            .fetch_one(&self.pool)
            .await
    }

    pub async fn count_checks(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COUNT(*) FROM checks")
            .fetch_one(&self.pool)
            .await
    }

    pub async fn count_pings_since(&self, cutoff: DateTime<Utc>) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COUNT(*) FROM pings WHERE created_at >= $1")
            .bind(cutoff.to_rfc3339())
            .fetch_one(&self.pool)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db,
        models::{CheckStatus, PingKind, ScheduleKind},
    };
    use chrono::{Duration, TimeZone, Utc};

    async fn seeded() -> Store {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool, "sqlite::memory:").await.unwrap();
        sqlx::query("INSERT INTO users (username, is_admin, created_at) VALUES ('u', 0, $1)")
            .bind(Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO projects (user_id, name, created_at) VALUES (1, 'p', $1)")
            .bind(Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        Store::new(pool)
    }

    #[tokio::test]
    async fn find_by_uuid_roundtrip() {
        let store = seeded().await;
        let id = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "job",
                ping_uuid: "uuid-1",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let found = store.find_check_by_uuid("uuid-1").await.unwrap().unwrap();
        assert_eq!(found.id, id);
        assert_eq!(found.status, CheckStatus::New);
        assert!(store.find_check_by_uuid("nope").await.unwrap().is_none());
    }

    /// A project's and a check's `description` persist on insert and change on
    /// update, on both the freshly-created row and the reloaded one.
    #[tokio::test]
    async fn project_and_check_description_persist_and_update() {
        let store = seeded().await;

        let pid = store
            .create_project(1, "described", "hello **world**", None, None, Utc::now())
            .await
            .unwrap();
        let p = store.find_project(pid).await.unwrap().unwrap();
        assert_eq!(p.description, "hello **world**");

        store
            .update_project(pid, "described", "updated *desc*", None, None)
            .await
            .unwrap();
        let p = store.find_project(pid).await.unwrap().unwrap();
        assert_eq!(p.description, "updated *desc*");

        let cid = store
            .create_check(&NewCheck {
                project_id: pid,
                name: "job",
                description: "runs `nightly`",
                ping_uuid: "uuid-desc",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let c = store.find_check(cid).await.unwrap().unwrap();
        assert_eq!(c.description, "runs `nightly`");

        store
            .update_check_schedule(
                cid,
                &UpdateCheck {
                    name: "job",
                    description: "updated check desc",
                    kind: ScheduleKind::Period,
                    period_secs: Some(60),
                    grace_secs: 30,
                    cron_expr: None,
                    timezone: "UTC",
                    scan_interval_secs: None,
                    max_runtime_secs: None,
                    nag_interval_secs: None,
                },
            )
            .await
            .unwrap();
        let c = store.find_check(cid).await.unwrap().unwrap();
        assert_eq!(c.description, "updated check desc");
    }

    #[tokio::test]
    async fn insert_ping_and_list_active() {
        let store = seeded().await;
        let id = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "job",
                ping_uuid: "u",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let ping_time = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        store
            .insert_ping(
                id,
                PingKind::Success,
                Some(0),
                "hello",
                Some("1.2.3.4"),
                ping_time,
            )
            .await
            .unwrap();

        let row = sqlx::query("SELECT * FROM pings WHERE check_id = $1")
            .bind(id)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("kind"), PingKind::Success.as_str());
        assert_eq!(row.get::<Option<i64>, _>("exit_code"), Some(0));
        assert_eq!(row.get::<String, _>("body"), "hello");
        assert_eq!(
            row.get::<Option<String>, _>("source_ip"),
            Some("1.2.3.4".to_string())
        );

        assert_eq!(store.list_active_checks().await.unwrap().len(), 1);
        store.set_status(id, CheckStatus::Paused).await.unwrap();
        assert_eq!(store.list_active_checks().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn list_active_checks_includes_up_status() {
        let store = seeded().await;
        let id = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "job",
                ping_uuid: "up-uuid",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        store.set_status(id, CheckStatus::Up).await.unwrap();
        let active = store.list_active_checks().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].status, CheckStatus::Up);
    }

    #[tokio::test]
    async fn mark_ping_updates_status_and_coalesces_timestamps() {
        let store = seeded().await;
        let id = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "job",
                ping_uuid: "mark-uuid",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();

        let t1 = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
        let s1 = Utc.with_ymd_and_hms(2026, 1, 1, 9, 59, 0).unwrap();
        let due1 = Utc.with_ymd_and_hms(2026, 1, 1, 11, 0, 0).unwrap();

        store
            .mark_ping(id, CheckStatus::Up, Some(t1), Some(s1), Some(due1))
            .await
            .unwrap();

        let found = store
            .find_check_by_uuid("mark-uuid")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, CheckStatus::Up);
        assert_eq!(found.last_ping_at, Some(t1));
        assert_eq!(found.last_start_at, Some(s1));
        assert_eq!(found.next_due_at, Some(due1));

        // Second call with None for ping/start timestamps: COALESCE preserves
        // the prior values, but next_due_at is unconditionally overwritten to NULL.
        store
            .mark_ping(id, CheckStatus::Up, None, None, None)
            .await
            .unwrap();

        let found = store
            .find_check_by_uuid("mark-uuid")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, CheckStatus::Up);
        assert_eq!(found.last_ping_at, Some(t1));
        assert_eq!(found.last_start_at, Some(s1));
        assert_eq!(found.next_due_at, None);
    }

    /// Proves the DB-level CHECK constraint on `checks.status` (source-level
    /// prevention added in migration 0001): an out-of-domain status must be
    /// rejected at insert time, not merely handled defensively when read back.
    #[tokio::test]
    async fn bad_status_is_rejected_by_check_constraint() {
        let store = seeded().await;
        let res = sqlx::query(
            "INSERT INTO checks (project_id, name, ping_uuid, schedule_kind, status, created_at) \
             VALUES (1, 'x', 'bad-status-uuid', 'period', 'bogus', $1)",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&store.pool)
        .await;
        assert!(
            res.is_err(),
            "expected CHECK constraint to reject bad status"
        );
    }

    #[tokio::test]
    async fn user_and_session_lifecycle() {
        let store = seeded().await; // seeds user id=1 already
        assert_eq!(store.count_users().await.unwrap(), 1);

        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let uid = store
            .create_user("bob", Some("phc"), true, now)
            .await
            .unwrap();
        assert_eq!(store.count_users().await.unwrap(), 2);

        let bob = store.find_user_by_username("bob").await.unwrap().unwrap();
        assert_eq!(bob.id, uid);
        assert!(bob.is_admin);
        assert_eq!(bob.password_hash.as_deref(), Some("phc"));
        assert!(
            store
                .find_user_by_username("nobody")
                .await
                .unwrap()
                .is_none()
        );

        store
            .create_session(
                "sess-1",
                uid,
                "csrf-1",
                now + chrono::Duration::hours(1),
                Some("curl/8.0"),
                Some("127.0.0.1"),
                now,
            )
            .await
            .unwrap();
        // valid at now
        let u = store
            .find_session_user("sess-1", now)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(u.id, uid);
        // expired two hours later
        assert!(
            store
                .find_session_user("sess-1", now + chrono::Duration::hours(2))
                .await
                .unwrap()
                .is_none()
        );

        // Listing surfaces the metadata stamped at creation, and the
        // `last_seen_at` throttle stamped it on the first `find_session_user`
        // lookup above (both happened "at now", so it reads back as `now`).
        let rows = store.list_sessions_for_user(uid, now).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "sess-1");
        assert_eq!(rows[0].created_at, Some(now));
        assert_eq!(rows[0].last_seen_at, Some(now));
        assert_eq!(rows[0].user_agent.as_deref(), Some("curl/8.0"));
        assert_eq!(rows[0].ip.as_deref(), Some("127.0.0.1"));

        // A lookup less than 60s later must not move `last_seen_at` (throttled).
        store
            .find_session_user("sess-1", now + chrono::Duration::seconds(30))
            .await
            .unwrap();
        let rows = store.list_sessions_for_user(uid, now).await.unwrap();
        assert_eq!(rows[0].last_seen_at, Some(now));

        // 60s+ later, the throttle lets the timestamp advance.
        let later = now + chrono::Duration::seconds(61);
        store.find_session_user("sess-1", later).await.unwrap();
        let rows = store.list_sessions_for_user(uid, later).await.unwrap();
        assert_eq!(rows[0].last_seen_at, Some(later));

        // A second session for the same user, newer than the first.
        let created2 = now + chrono::Duration::seconds(5);
        store
            .create_session(
                "sess-2",
                uid,
                "csrf-2",
                now + chrono::Duration::hours(2),
                None,
                None,
                created2,
            )
            .await
            .unwrap();
        let rows = store.list_sessions_for_user(uid, later).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "sess-2", "newest-created session lists first");

        // Revoking "other" sessions keeps only sess-2.
        let removed = store
            .delete_other_sessions_for_user(uid, "sess-2")
            .await
            .unwrap();
        assert_eq!(removed, 1);
        let rows = store.list_sessions_for_user(uid, later).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "sess-2");

        // Owner-scoped delete: another user's id is a silent no-op.
        assert!(!store.delete_session_owned("sess-2", 1).await.unwrap());
        assert_eq!(
            store
                .list_sessions_for_user(uid, later)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(store.delete_session_owned("sess-2", uid).await.unwrap());
        assert!(
            store
                .list_sessions_for_user(uid, later)
                .await
                .unwrap()
                .is_empty()
        );

        // Plain `delete_session` (used by logout) still works unscoped.
        store
            .create_session(
                "sess-3",
                uid,
                "csrf-3",
                now + chrono::Duration::hours(1),
                None,
                None,
                now,
            )
            .await
            .unwrap();
        store.delete_session("sess-3").await.unwrap();
        assert!(
            store
                .find_session_user("sess-3", now)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_expired_sessions_boundary() {
        let store = seeded().await;
        let uid = store
            .create_user("carol", Some("phc"), false, Utc::now())
            .await
            .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();

        // expires_at == now must be deleted (`<=`, not `<`).
        store
            .create_session(
                "sess-at-now",
                uid,
                "csrf-at-now",
                now,
                None,
                None,
                now - chrono::Duration::hours(1),
            )
            .await
            .unwrap();
        // expires_at in the future must survive.
        store
            .create_session(
                "sess-future",
                uid,
                "csrf-future",
                now + chrono::Duration::hours(1),
                None,
                None,
                now - chrono::Duration::hours(1),
            )
            .await
            .unwrap();

        let deleted = store
            .delete_expired_sessions(&now.to_rfc3339())
            .await
            .unwrap();
        assert_eq!(deleted, 1);
        let rows = store.list_sessions_for_user(uid, now).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "sess-future");
    }

    #[tokio::test]
    async fn new_user_is_not_disabled() {
        let store = seeded().await;
        let id = store
            .create_user("u2", Some("phc"), false, Utc::now())
            .await
            .unwrap();
        let u = store.find_user_by_id(id).await.unwrap().unwrap();
        assert!(!u.disabled);
    }

    #[tokio::test]
    async fn set_user_disabled_toggles() {
        let store = seeded().await;
        let id = store
            .create_user("u3", Some("phc"), false, Utc::now())
            .await
            .unwrap();
        store.set_user_disabled(id, true).await.unwrap();
        assert!(store.find_user_by_id(id).await.unwrap().unwrap().disabled);
        store.set_user_disabled(id, false).await.unwrap();
        assert!(!store.find_user_by_id(id).await.unwrap().unwrap().disabled);
    }

    #[tokio::test]
    async fn set_password_then_login_hash_changes() {
        let store = seeded().await;
        let id = store
            .create_user("u4", Some("old"), false, Utc::now())
            .await
            .unwrap();
        store.set_user_password(id, "newphc").await.unwrap();
        let u = store.find_user_by_id(id).await.unwrap().unwrap();
        assert_eq!(u.password_hash.as_deref(), Some("newphc"));
    }

    #[tokio::test]
    async fn set_admin_and_count_enabled_admins() {
        let store = seeded().await;
        let a = store
            .create_user("a", Some("p"), true, Utc::now())
            .await
            .unwrap();
        let b = store
            .create_user("b", Some("p"), false, Utc::now())
            .await
            .unwrap();
        assert_eq!(store.count_enabled_admins().await.unwrap(), 1);
        store.set_user_admin(b, true).await.unwrap();
        assert_eq!(store.count_enabled_admins().await.unwrap(), 2);
        store.set_user_disabled(a, true).await.unwrap();
        assert_eq!(store.count_enabled_admins().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn project_channel_binding_and_settings() {
        let store = seeded().await;
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();

        let pid = store
            .create_project(1, "web", "", Some(15), None, now)
            .await
            .unwrap();
        assert_eq!(store.list_projects_for_user(1).await.unwrap().len(), 2); // 'p' from seed + 'web'
        assert_eq!(
            store
                .find_project(pid)
                .await
                .unwrap()
                .unwrap()
                .scan_interval_secs,
            Some(15)
        );

        let cid = store
            .create_channel(
                pid,
                ChannelKind::Webhook,
                "hook",
                r#"{"url":"http://x"}"#,
                now,
            )
            .await
            .unwrap();
        assert_eq!(store.list_channels_for_project(pid).await.unwrap().len(), 1);

        let chk = store
            .create_check(&NewCheck {
                project_id: pid,
                name: "job",
                ping_uuid: "uuid-x",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        store.bind_channel(chk, cid).await.unwrap();
        assert_eq!(store.bound_channel_ids(chk).await.unwrap(), vec![cid]);
        assert_eq!(store.channels_for_check(chk).await.unwrap().len(), 1);
        store.unbind_channel(chk, cid).await.unwrap();
        assert!(store.bound_channel_ids(chk).await.unwrap().is_empty());

        store
            .record_notification(chk, cid, EventKind::Down, NotifyStatus::Ok, None, now)
            .await
            .unwrap();
        assert_eq!(
            store
                .list_recent_notifications(chk, 10)
                .await
                .unwrap()
                .len(),
            1
        );

        assert!(store.get_setting("scan_interval").await.unwrap().is_none());
        store.set_setting("scan_interval", "45").await.unwrap();
        assert_eq!(
            store.get_setting("scan_interval").await.unwrap().as_deref(),
            Some("45")
        );
        store.set_setting("scan_interval", "60").await.unwrap(); // upsert
        assert_eq!(
            store.get_setting("scan_interval").await.unwrap().as_deref(),
            Some("60")
        );

        let map = store.all_project_scan_intervals().await.unwrap();
        assert_eq!(map.get(&pid), Some(&Some(15)));
    }

    #[tokio::test]
    async fn new_check_has_nag_defaults() {
        let store = seeded().await;
        let id = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "c",
                ping_uuid: "uu",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let c = store.find_check(id).await.unwrap().unwrap();
        assert_eq!(c.nag_interval_secs, None);
        assert_eq!(c.last_alert_at, None);
        assert!(!c.acknowledged);
    }

    #[tokio::test]
    async fn nag_state_methods_roundtrip() {
        let store = seeded().await;
        let id = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "c",
                ping_uuid: "uu",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        store.set_status(id, CheckStatus::Down).await.unwrap();

        // down check appears in list_down_checks
        let down = store.list_down_checks().await.unwrap();
        assert_eq!(down.len(), 1);
        assert_eq!(down[0].id, id);

        let t0 = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();
        store.begin_down_alert(id, t0).await.unwrap();
        let c = store.find_check(id).await.unwrap().unwrap();
        assert_eq!(c.last_alert_at, Some(t0));
        assert!(!c.acknowledged);

        store.acknowledge(id).await.unwrap();
        assert!(store.find_check(id).await.unwrap().unwrap().acknowledged);

        let t1 = t0 + chrono::Duration::seconds(90);
        store.record_reminder(id, t1).await.unwrap();
        assert_eq!(
            store.find_check(id).await.unwrap().unwrap().last_alert_at,
            Some(t1)
        );

        store.clear_nag(id).await.unwrap();
        let c = store.find_check(id).await.unwrap().unwrap();
        assert_eq!(c.last_alert_at, None);
        assert!(!c.acknowledged);

        // project nag intervals map exposes the (possibly-null) override
        let map = store.all_project_nag_intervals().await.unwrap();
        assert!(map.contains_key(&1));
    }

    #[tokio::test]
    async fn delete_before_removes_only_old_rows() {
        use chrono::Duration;
        let store = seeded().await;
        let cid = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "c",
                ping_uuid: "uu",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let chan = store
            .create_channel(
                1,
                ChannelKind::Webhook,
                "h",
                "{\"url\":\"http://x\"}",
                Utc::now(),
            )
            .await
            .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
        let old = now - Duration::days(10);
        let recent = now - Duration::days(1);

        // two pings: one old, one recent
        store
            .insert_ping(cid, PingKind::Success, None, "", None, old)
            .await
            .unwrap();
        store
            .insert_ping(cid, PingKind::Success, None, "", None, recent)
            .await
            .unwrap();
        // two notifications: one old, one recent
        store
            .record_notification(cid, chan, EventKind::Down, NotifyStatus::Ok, None, old)
            .await
            .unwrap();
        store
            .record_notification(cid, chan, EventKind::Up, NotifyStatus::Ok, None, recent)
            .await
            .unwrap();

        // cutoff = 7 days before now → deletes the 10-day-old rows, keeps the 1-day-old
        let cutoff = (now - Duration::days(7)).to_rfc3339();
        assert_eq!(store.delete_pings_before(&cutoff).await.unwrap(), 1);
        assert_eq!(store.delete_notifications_before(&cutoff).await.unwrap(), 1);
        assert_eq!(store.list_recent_pings(cid, 10).await.unwrap().len(), 1);
        assert_eq!(
            store
                .list_recent_notifications(cid, 10)
                .await
                .unwrap()
                .len(),
            1
        );

        // a far-past cutoff deletes nothing more
        let far = (now - Duration::days(365)).to_rfc3339();
        assert_eq!(store.delete_pings_before(&far).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn audit_roundtrips() {
        let store = seeded().await;
        let uid = store
            .create_user("adm", Some("phc"), true, Utc::now())
            .await
            .unwrap();
        store
            .record_audit(
                &NewAudit {
                    actor_user_id: uid,
                    actor_username: "adm",
                    action: "admin.access",
                    target_type: Some("project"),
                    target_id: Some(7),
                    target_owner_id: Some(42),
                    method: Some("GET"),
                    path: Some("/admin/projects/7"),
                    detail: None,
                },
                Utc::now(),
            )
            .await
            .unwrap();
        let rows = store.list_audit(10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action, "admin.access");
        assert_eq!(rows[0].target_owner_id, Some(42));
        assert_eq!(rows[0].actor_username, "adm");
    }

    #[tokio::test]
    async fn status_counts_and_scale() {
        let store = seeded().await;
        // `seeded()` already pre-seeds one user 'u' (id 1) and one project 'p'
        // (id 1) with no checks, so `username` must be distinct here to avoid
        // the UNIQUE constraint on `users.username`.
        let uid = store
            .create_user("u2", Some("p"), false, Utc::now())
            .await
            .unwrap();
        let pid = store
            .create_project(uid, "p2", "", None, None, Utc::now())
            .await
            .unwrap();
        store
            .create_check(&NewCheck {
                project_id: pid,
                name: "a",
                ping_uuid: "uuid-a",
                kind: ScheduleKind::Period,
                period_secs: Some(3600),
                grace_secs: 300,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let bid = store
            .create_check(&NewCheck {
                project_id: pid,
                name: "b",
                ping_uuid: "uuid-b",
                kind: ScheduleKind::Period,
                period_secs: Some(3600),
                grace_secs: 300,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();

        // The two checks just created are the only rows in `checks` (the
        // seeded project has none), and both start out `new`.
        let counts = store.count_checks_by_status().await.unwrap();
        assert_eq!(
            counts.new + counts.up + counts.down + counts.paused,
            store.count_checks().await.unwrap()
        );
        assert_eq!(counts.new, 2);
        assert_eq!(counts.up, 0);
        assert_eq!(counts.down, 0);
        assert_eq!(counts.paused, 0);
        assert_eq!(counts.running, 0);
        assert_eq!(store.count_projects().await.unwrap(), 2); // seeded 'p' + this 'p2'

        // `b` gets a start ping and never finishes: stays stored `new`, but
        // becomes running.
        let t1 = Utc::now();
        let t2 = t1 + Duration::seconds(1);
        store
            .mark_ping(bid, CheckStatus::New, None, Some(t1), None)
            .await
            .unwrap();

        // `c` finishes successfully (stored `up`), then starts again without
        // finishing: running on top of `up`.
        let cid = store
            .create_check(&NewCheck {
                project_id: pid,
                name: "c",
                ping_uuid: "uuid-c",
                kind: ScheduleKind::Period,
                period_secs: Some(3600),
                grace_secs: 300,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        store
            .mark_ping(cid, CheckStatus::Up, Some(t1), None, None)
            .await
            .unwrap();
        store
            .mark_ping(cid, CheckStatus::Up, None, Some(t2), None)
            .await
            .unwrap();

        // `d` fails (stored `down`), then starts again: a `down` check with
        // an in-flight start must NOT be counted as running — `Down` beats
        // `Running` in the display-status precedence.
        let did = store
            .create_check(&NewCheck {
                project_id: pid,
                name: "d",
                ping_uuid: "uuid-d",
                kind: ScheduleKind::Period,
                period_secs: Some(3600),
                grace_secs: 300,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        store
            .mark_ping(did, CheckStatus::Down, Some(t1), None, None)
            .await
            .unwrap();
        store
            .mark_ping(did, CheckStatus::Down, None, Some(t2), None)
            .await
            .unwrap();

        let counts = store.count_checks_by_status().await.unwrap();
        assert_eq!(
            counts.new + counts.up + counts.down + counts.paused,
            store.count_checks().await.unwrap()
        );
        assert_eq!(counts.new, 2); // `a` and `b` (a start ping doesn't change stored status)
        assert_eq!(counts.up, 1); // `c`
        assert_eq!(counts.down, 1); // `d`
        assert_eq!(counts.running, 2); // `b` (new+running) and `c` (up+running), not `d`
        assert_eq!(store.count_checks().await.unwrap(), 4); // `a`, `b`, `c`, `d`
    }

    #[tokio::test]
    async fn notification_counts_split_ok_error() {
        let store = seeded().await;
        let cid = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "c",
                ping_uuid: "notif-uuid",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let chan = store
            .create_channel(
                1,
                ChannelKind::Webhook,
                "h",
                "{\"url\":\"http://x\"}",
                Utc::now(),
            )
            .await
            .unwrap();

        let now = Utc::now();
        store
            .record_notification(cid, chan, EventKind::Up, NotifyStatus::Ok, None, now)
            .await
            .unwrap();
        store
            .record_notification(
                cid,
                chan,
                EventKind::Down,
                NotifyStatus::Error,
                Some("boom"),
                now,
            )
            .await
            .unwrap();

        let (ok, err) = store
            .notification_counts_since(Utc::now() - chrono::Duration::days(1))
            .await
            .unwrap();
        assert_eq!(ok, 1);
        assert_eq!(err, 1);
    }

    #[tokio::test]
    async fn channel_failure_counts_does_not_merge_same_named_channels() {
        // Regression test: `channels.name` is NOT unique (only `channels.id`
        // is). Two distinct channels that happen to share a name must be
        // reported as two separate rows, not merged into one.
        let store = seeded().await;
        let cid = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "dup-check",
                ping_uuid: "dup-uuid",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let chan_a = store
            .create_channel(
                1,
                ChannelKind::Webhook,
                "dup",
                "{\"url\":\"http://a\"}",
                Utc::now(),
            )
            .await
            .unwrap();
        let chan_b = store
            .create_channel(
                1,
                ChannelKind::Webhook,
                "dup",
                "{\"url\":\"http://b\"}",
                Utc::now(),
            )
            .await
            .unwrap();
        assert_ne!(chan_a, chan_b);

        let now = Utc::now();
        store
            .record_notification(
                cid,
                chan_a,
                EventKind::Down,
                NotifyStatus::Error,
                Some("boom-a"),
                now,
            )
            .await
            .unwrap();
        store
            .record_notification(
                cid,
                chan_b,
                EventKind::Down,
                NotifyStatus::Error,
                Some("boom-b"),
                now,
            )
            .await
            .unwrap();

        let rows = store
            .channel_failure_counts_since(Utc::now() - chrono::Duration::days(1))
            .await
            .unwrap();

        assert_eq!(
            rows.len(),
            2,
            "same-named channels must not be merged into one row: {rows:?}"
        );
        for (name, ok, err) in &rows {
            assert_eq!(name, "dup");
            assert_eq!(*ok, 0);
            assert_eq!(*err, 1);
        }
    }

    #[tokio::test]
    async fn down_checks_order_never_pinged_last() {
        let store = seeded().await;
        let a = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "A",
                ping_uuid: "uuid-a",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let b = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "B",
                ping_uuid: "uuid-b",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();

        // A has a last_ping_at (was pinged before going down); B never pinged.
        let t0 = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();
        store
            .mark_ping(a, CheckStatus::Down, Some(t0), Some(t0), None)
            .await
            .unwrap();
        store.set_status(b, CheckStatus::Down).await.unwrap();

        let rows = store.list_down_checks_with_owner().await.unwrap();
        let names: Vec<_> = rows.iter().map(|(c, _, _)| c.name.clone()).collect();
        let ia = names.iter().position(|n| n == "A").unwrap();
        let ib = names.iter().position(|n| n == "B").unwrap();
        // A (pinged) must precede B (never pinged) — NULLs sort last.
        assert!(
            ia < ib,
            "expected pinged check before never-pinged: {names:?}"
        );
    }

    #[tokio::test]
    async fn batch_recent_pings_matches_per_check_and_honors_limit() {
        let store = seeded().await;
        let mk = |n: &'static str, u: &'static str| {
            let store = &store;
            async move {
                store
                    .create_check(&NewCheck {
                        project_id: 1,
                        name: n,
                        ping_uuid: u,
                        kind: ScheduleKind::Period,
                        period_secs: Some(60),
                        grace_secs: 30,
                        timezone: "UTC",
                        ..Default::default()
                    })
                    .await
                    .unwrap()
            }
        };
        let a = mk("A", "uuid-a").await;
        let b = mk("B", "uuid-b").await;
        let c = mk("C", "uuid-c").await; // will have no pings
        let base = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        // A: 5 pings, B: 2 pings, C: none.
        for i in 0..5 {
            store
                .insert_ping(
                    a,
                    PingKind::Success,
                    Some(0),
                    "x",
                    None,
                    base + chrono::Duration::seconds(i),
                )
                .await
                .unwrap();
        }
        for i in 0..2 {
            store
                .insert_ping(
                    b,
                    PingKind::Success,
                    Some(0),
                    "y",
                    None,
                    base + chrono::Duration::seconds(i),
                )
                .await
                .unwrap();
        }

        let batch = store
            .list_recent_pings_for_checks(&[a, b, c], 3)
            .await
            .unwrap();
        // Per-check limit honored: A capped at 3, B has 2, C absent.
        assert_eq!(batch.get(&a).unwrap().len(), 3);
        assert_eq!(batch.get(&b).unwrap().len(), 2);
        assert!(!batch.contains_key(&c));
        // Batch matches the per-check query (same ids, same newest-first order).
        for id in [a, b] {
            let single: Vec<i64> = store
                .list_recent_pings(id, 3)
                .await
                .unwrap()
                .iter()
                .map(|p| p.id)
                .collect();
            let batched: Vec<i64> = batch.get(&id).unwrap().iter().map(|p| p.id).collect();
            assert_eq!(batched, single, "check {id}");
        }
        // Empty input short-circuits to an empty map.
        assert!(
            store
                .list_recent_pings_for_checks(&[], 3)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn list_pings_page_keyset_pagination() {
        let store = seeded().await;
        let cid = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "c",
                ping_uuid: "page-uuid",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let base = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        for i in 0..5 {
            store
                .insert_ping(
                    cid,
                    PingKind::Success,
                    Some(0),
                    "",
                    None,
                    base + chrono::Duration::seconds(i),
                )
                .await
                .unwrap();
        }
        // Discover the actual (auto-increment) ids, oldest to newest, rather
        // than assuming literal values.
        let mut all: Vec<i64> = store
            .list_recent_pings(cid, 10)
            .await
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect();
        all.sort_unstable();
        assert_eq!(all.len(), 5);
        let [id1, id2, id3, id4, id5]: [i64; 5] = all.try_into().unwrap();

        // Latest, limit 2: newest 2 ids, has_newer=false, has_older=true.
        let page = store
            .list_pings_page(cid, PageCursor::Latest, 2, &PingFilter::default())
            .await
            .unwrap();
        assert_eq!(
            page.items.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![id5, id4]
        );
        assert!(!page.has_newer);
        assert!(page.has_older);

        // Before(oldest id of the latest page) -> next 2 older ids, both flags true.
        let page2 = store
            .list_pings_page(cid, PageCursor::Before(id4), 2, &PingFilter::default())
            .await
            .unwrap();
        assert_eq!(
            page2.items.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![id3, id2]
        );
        assert!(page2.has_newer);
        assert!(page2.has_older);

        // Paging Before again -> the last remaining row, has_older=false.
        let page3 = store
            .list_pings_page(cid, PageCursor::Before(id2), 2, &PingFilter::default())
            .await
            .unwrap();
        assert_eq!(
            page3.items.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![id1]
        );
        assert!(page3.has_newer);
        assert!(!page3.has_older);

        // After(newest id of that last page) -> steps back toward newest;
        // there are still newer rows (id4, id5) beyond this page.
        let page4 = store
            .list_pings_page(cid, PageCursor::After(id1), 2, &PingFilter::default())
            .await
            .unwrap();
        assert_eq!(
            page4.items.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![id3, id2]
        );
        assert!(page4.has_newer);
        assert!(page4.has_older);
    }

    #[tokio::test]
    async fn list_notifications_page_keyset_pagination() {
        let store = seeded().await;
        let cid = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "c",
                ping_uuid: "notif-page-uuid",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let chan = store
            .create_channel(
                1,
                ChannelKind::Webhook,
                "h",
                "{\"url\":\"http://x\"}",
                Utc::now(),
            )
            .await
            .unwrap();
        let base = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        for i in 0..5 {
            store
                .record_notification(
                    cid,
                    chan,
                    EventKind::Down,
                    NotifyStatus::Ok,
                    None,
                    base + chrono::Duration::seconds(i),
                )
                .await
                .unwrap();
        }
        let mut all: Vec<i64> = store
            .list_recent_notifications(cid, 10)
            .await
            .unwrap()
            .iter()
            .map(|n| n.id)
            .collect();
        all.sort_unstable();
        assert_eq!(all.len(), 5);
        let [id1, id2, id3, id4, id5]: [i64; 5] = all.try_into().unwrap();

        let page = store
            .list_notifications_page(cid, PageCursor::Latest, 2, &NotifFilter::default())
            .await
            .unwrap();
        assert_eq!(
            page.items.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![id5, id4]
        );
        assert!(!page.has_newer);
        assert!(page.has_older);

        let page2 = store
            .list_notifications_page(cid, PageCursor::Before(id4), 2, &NotifFilter::default())
            .await
            .unwrap();
        assert_eq!(
            page2.items.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![id3, id2]
        );
        assert!(page2.has_newer);
        assert!(page2.has_older);

        let page3 = store
            .list_notifications_page(cid, PageCursor::Before(id2), 2, &NotifFilter::default())
            .await
            .unwrap();
        assert_eq!(
            page3.items.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![id1]
        );
        assert!(page3.has_newer);
        assert!(!page3.has_older);

        let page4 = store
            .list_notifications_page(cid, PageCursor::After(id1), 2, &NotifFilter::default())
            .await
            .unwrap();
        assert_eq!(
            page4.items.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![id3, id2]
        );
        assert!(page4.has_newer);
        assert!(page4.has_older);
    }

    #[tokio::test]
    async fn list_pings_page_filters_by_kind_and_date() {
        let store = seeded().await;
        let cid = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "c",
                ping_uuid: "ping-filter-uuid",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let base = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let kinds = [
            PingKind::Success,
            PingKind::Fail,
            PingKind::Start,
            PingKind::Success,
            PingKind::Fail,
        ];
        for (i, k) in kinds.iter().enumerate() {
            store
                .insert_ping(
                    cid,
                    *k,
                    None,
                    "",
                    None,
                    base + chrono::Duration::seconds(i as i64),
                )
                .await
                .unwrap();
        }

        // Kind filter: only the two fails, newest-first, no other pages.
        let f = PingFilter {
            kinds: vec![PingKind::Fail],
            ..Default::default()
        };
        let page = store
            .list_pings_page(cid, PageCursor::Latest, 20, &f)
            .await
            .unwrap();
        assert_eq!(
            page.items.iter().map(|p| p.kind).collect::<Vec<_>>(),
            vec![PingKind::Fail, PingKind::Fail]
        );
        assert!(!page.has_newer && !page.has_older);

        // Inclusive date range [base+1s, base+3s] -> the three middle rows.
        let f = PingFilter {
            from: Some(base + chrono::Duration::seconds(1)),
            to: Some(base + chrono::Duration::seconds(3)),
            ..Default::default()
        };
        let page = store
            .list_pings_page(cid, PageCursor::Latest, 20, &f)
            .await
            .unwrap();
        let secs: Vec<i64> = page
            .items
            .iter()
            .map(|p| (p.created_at - base).num_seconds())
            .collect();
        assert_eq!(secs, vec![3, 2, 1]);

        // Kind + date combined: fails within [base+3s, base+5s] -> just 4s.
        let f = PingFilter {
            kinds: vec![PingKind::Fail],
            from: Some(base + chrono::Duration::seconds(3)),
            to: Some(base + chrono::Duration::seconds(5)),
        };
        let page = store
            .list_pings_page(cid, PageCursor::Latest, 20, &f)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].kind, PingKind::Fail);
        assert_eq!((page.items[0].created_at - base).num_seconds(), 4);
    }

    #[tokio::test]
    async fn list_notifications_page_filters_by_event_and_status() {
        let store = seeded().await;
        let cid = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "c",
                ping_uuid: "notif-filter-uuid",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let chan = store
            .create_channel(
                1,
                ChannelKind::Webhook,
                "h",
                "{\"url\":\"http://x\"}",
                Utc::now(),
            )
            .await
            .unwrap();
        let base = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let rows = [
            (EventKind::Down, NotifyStatus::Ok),
            (EventKind::Up, NotifyStatus::Error),
            (EventKind::Down, NotifyStatus::Error),
            (EventKind::Reminder, NotifyStatus::Ok),
            (EventKind::Up, NotifyStatus::Ok),
        ];
        for (i, (event, status)) in rows.iter().enumerate() {
            let err = (*status == NotifyStatus::Error).then_some("boom");
            store
                .record_notification(
                    cid,
                    chan,
                    *event,
                    *status,
                    err,
                    base + chrono::Duration::seconds(i as i64),
                )
                .await
                .unwrap();
        }

        // Event filter: the two Up events.
        let f = NotifFilter {
            events: vec![EventKind::Up],
            ..Default::default()
        };
        let page = store
            .list_notifications_page(cid, PageCursor::Latest, 20, &f)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(page.items.iter().all(|n| n.event == EventKind::Up));

        // Delivery-result filter: the two Error deliveries.
        let f = NotifFilter {
            statuses: vec![NotifyStatus::Error],
            ..Default::default()
        };
        let page = store
            .list_notifications_page(cid, PageCursor::Latest, 20, &f)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(page.items.iter().all(|n| n.status == NotifyStatus::Error));

        // Event + status combined: Up AND Error -> just the row at 1s.
        let f = NotifFilter {
            events: vec![EventKind::Up],
            statuses: vec![NotifyStatus::Error],
            ..Default::default()
        };
        let page = store
            .list_notifications_page(cid, PageCursor::Latest, 20, &f)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!((page.items[0].created_at - base).num_seconds(), 1);
    }
}
