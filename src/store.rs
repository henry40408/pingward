use crate::db::Pool;
use crate::models::{
    ApiKey, AuditLog, Channel, ChannelKind, Check, CheckStatus, Notification, NotifyStatus, Ping,
    PingKind, PingSummary, Project, ScheduleKind, Session, User,
};
use crate::notify::EventKind;
use chrono::{DateTime, Utc};
use sqlx::Row;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

#[derive(Clone)]
pub struct Store {
    pub pool: Pool,
}

/// Why a [`Store::create_user`] failed. A duplicate username is an ordinary form
/// outcome, so it gets its own variant instead of `AppError::Db`'s blank 500.
#[derive(Debug, thiserror::Error)]
pub enum CreateUserError {
    #[error("that username is already taken")]
    UsernameTaken,
    #[error(transparent)]
    Db(sqlx::Error),
}

impl From<sqlx::Error> for CreateUserError {
    /// Any unique violation is the username: it is the only unique constraint
    /// on `users`. A second one would need distinguishing here.
    fn from(e: sqlx::Error) -> Self {
        match &e {
            sqlx::Error::Database(db) if db.is_unique_violation() => Self::UsernameTaken,
            _ => Self::Db(e),
        }
    }
}

/// Cross-user rollup of check statuses for the admin dashboard.
#[derive(Debug, Clone, Default)]
pub struct CheckStatusCounts {
    pub new: i64,
    pub up: i64,
    pub down: i64,
    pub paused: i64,
    /// Stored `up`/`new` checks with an in-flight `start` (display-only
    /// `Running`). A second query, since `SUM(CASE ...)`'s type differs across
    /// backends on `Any`.
    pub running: i64,
}

/// Keyset cursor over `id` rather than `created_at`: monotonic, indexed, and
/// stable under concurrent inserts.
#[derive(Debug, Clone, Copy)]
pub enum PageCursor {
    /// The newest page.
    Latest,
    /// Rows older than this id.
    Before(i64),
    /// Rows newer than this id.
    After(i64),
}

/// Always newest-first (`id DESC`), whichever direction was queried.
#[derive(Debug)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub has_newer: bool,
    pub has_older: bool,
}

/// Empty/`None` fields mean no constraint. Date bounds are re-serialized with
/// `to_rfc3339()` so comparing them to the `created_at` text stays chronological.
#[derive(Debug, Clone, Default)]
pub struct PingFilter {
    pub kinds: Vec<PingKind>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

impl PingFilter {
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

/// Empty/`None` fields mean no constraint; date bounds as in [`PingFilter`].
#[derive(Debug, Clone, Default)]
pub struct NotifFilter {
    pub events: Vec<EventKind>,
    pub statuses: Vec<NotifyStatus>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

impl NotifFilter {
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

/// `actor` and `action` are exact matches (the UI's selects list stored values).
/// `None` means no constraint; `Some("")` matches only an empty value. Date
/// bounds as in [`PingFilter`].
#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    pub actor: Option<String>,
    pub action: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

impl AuditFilter {
    pub fn is_empty(&self) -> bool {
        self.actor.is_none() && self.action.is_none() && self.from.is_none() && self.to.is_none()
    }

    fn predicates(&self) -> Vec<Predicate> {
        let mut p = Vec::new();
        if let Some(a) = &self.actor {
            p.push(Predicate::TextCmp("actor_username", "=", a.clone()));
        }
        if let Some(a) = &self.action {
            p.push(Predicate::TextCmp("action", "=", a.clone()));
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

/// Manual because `ScheduleKind` has no `Default`.
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

/// No `Default`, unlike [`NewCheck`]: on an UPDATE a defaulted field would
/// overwrite the stored value (e.g. blank the name).
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

/// Fallible: a panic on a corrupt row (via `list_active_checks`) would
/// permanently kill the scan task.
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
        sso: row.get::<i64, _>("sso") != 0,
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

/// Narrow counterpart of [`row_to_ping`]; keep the two in step.
fn row_to_ping_summary(row: &sqlx::any::AnyRow) -> Result<PingSummary, sqlx::Error> {
    let kind_raw: String = row.get("kind");
    let kind = PingKind::from_str(&kind_raw)
        .map_err(|e| decode_err(format!("invalid ping kind {kind_raw:?}: {e}")))?;
    Ok(PingSummary {
        id: row.get("id"),
        check_id: row.get("check_id"),
        kind,
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

/// The only data that reaches a keyset query, always as a bound parameter.
enum QueryBind {
    Int(i64),
    Text(String),
}

/// A filter predicate: column and operator are caller literals, values are bound.
enum Predicate {
    /// `col op $n`.
    TextCmp(&'static str, &'static str, String),
    /// `col IN (…)`. Filter builders never construct it empty, so no `IN ()`.
    TextIn(&'static str, Vec<String>),
}

/// Keyset pagination by `id` for `pings`/`notifications`/`audit_log`. Fetches
/// `limit + 1` rows to detect another page without a `COUNT(*)`. `scope` is
/// e.g. `("check_id", id)`; `audit_log` passes `None`, so `WHERE` may be empty.
#[allow(
    clippy::cast_sign_loss,
    reason = "`limit` is a small positive page size supplied by callers, never negative"
)]
async fn keyset_page<T>(
    pool: &crate::db::Pool,
    table: &'static str,
    scope: Option<(&'static str, i64)>,
    cursor: PageCursor,
    limit: i64,
    filters: &[Predicate],
    row_to: fn(&sqlx::any::AnyRow) -> Result<T, sqlx::Error>,
) -> Result<Page<T>, sqlx::Error> {
    let fetch_limit = limit + 1;
    let mut conds: Vec<String> = Vec::new();
    let mut binds: Vec<QueryBind> = Vec::new();

    if let Some((col, id)) = scope {
        binds.push(QueryBind::Int(id));
        conds.push(format!("{col} = ${}", binds.len()));
    }

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
    let where_clause = if conds.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conds.join(" AND "))
    };
    let sql = format!(
        "SELECT * FROM {table}{where_clause} ORDER BY id {order} LIMIT ${}",
        binds.len()
    );
    // Safe: only literals are interpolated; all values are bound.
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
    for b in &binds {
        q = match b {
            QueryBind::Int(i) => q.bind(*i),
            QueryBind::Text(s) => q.bind(s.clone()),
        };
    }
    let mut rows = q.fetch_all(pool).await?;

    // A Before/After cursor came from an adjacent page, so the opposite flag is true.
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
                // ASC, so the overflow row is last (farthest from the cursor).
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

    /// Corrupt rows are logged and skipped so one cannot abort the whole scan.
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

    /// Nag candidates; corrupt rows are skipped as in `list_active_checks`.
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

    /// Start a down incident: stamp the alert baseline and clear any prior
    /// acknowledgement so a fresh incident is never silent.
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

    /// Clear nag state on recovery.
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
    ) -> Result<i64, CreateUserError> {
        let row = sqlx::query(
            "INSERT INTO users (username, password_hash, is_admin, created_at) VALUES ($1,$2,$3,$4) RETURNING id",
        )
        .bind(username)
        .bind(password_hash)
        .bind(is_admin as i64)
        .bind(now.to_rfc3339())
        .fetch_one(&self.pool)
        .await
        .map_err(CreateUserError::from)?;
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

    /// Only the hash and the non-secret `prefix` are stored.
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

    /// Newest first.
    pub async fn list_api_keys_for_user(&self, user_id: i64) -> Result<Vec<ApiKey>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM api_keys WHERE user_id = $1 ORDER BY id DESC")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_api_key).collect()
    }

    /// Owner-scoped. `false` for a missing key and a foreign one alike, so
    /// existence is not disclosed.
    pub async fn delete_api_key(&self, id: i64, user_id: i64) -> Result<bool, sqlx::Error> {
        let res = sqlx::query("DELETE FROM api_keys WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// The owning user id of an unexpired key. `last_used_at` is refreshed at
    /// most once per 60s, so a hot key does not cost a write per request.
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

    #[allow(
        clippy::too_many_arguments,
        reason = "each param is a distinct piece of per-session metadata; a struct would just move the noise"
    )]
    pub async fn create_session(
        &self,
        id: &str,
        user_id: i64,
        expires_at: DateTime<Utc>,
        user_agent: Option<&str>,
        ip: Option<&str>,
        sso: bool,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO sessions (id, user_id, expires_at, created_at, user_agent, ip, sso) \
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(id)
        .bind(user_id)
        .bind(expires_at.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(user_agent)
        .bind(ip)
        .bind(sso as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The session's user, honoring the idle window (`expires_at`, in SQL) and
    /// the absolute cap (`created_at`, in Rust). `last_seen_at` is refreshed at
    /// most once per 60s; `expires_at` slides only past the idle window's
    /// half-life (`auth::refreshed_expiry`).
    pub async fn find_session_user(
        &self,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<User>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT u.*, s.last_seen_at AS session_last_seen_at, \
                    s.created_at AS session_created_at, s.expires_at AS session_expires_at, \
                    s.ip AS session_ip, s.user_agent AS session_user_agent \
             FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.id = $1 AND s.expires_at > $2",
        )
        .bind(session_id)
        .bind(now.to_rfc3339())
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        // Not in SQL: a pre-0010 row has `created_at = ''`, which sorts below
        // every timestamp.
        let created = parse_ts(row.get("session_created_at"));
        if crate::auth::is_past_absolute_cap(created, now) {
            return Ok(None);
        }
        let stale = parse_ts(row.get("session_last_seen_at"))
            .is_none_or(|t| now - t >= chrono::Duration::seconds(60));
        let expires = parse_ts(row.get("session_expires_at"))
            .ok_or_else(|| decode_err("sessions.expires_at is not RFC3339"))?;
        match (stale, crate::auth::refreshed_expiry(created, expires, now)) {
            (_, Some(renewal)) => {
                sqlx::query("UPDATE sessions SET last_seen_at = $1, expires_at = $2 WHERE id = $3")
                    .bind(now.to_rfc3339())
                    .bind(renewal.expires_at.to_rfc3339())
                    .bind(session_id)
                    .execute(&self.pool)
                    .await?;
                let user_id: i64 = row.get("id");
                let ip: Option<String> = row.get("session_ip");
                let user_agent: Option<String> = row.get("session_user_agent");
                // `renewal` tells a clamp (deployment signal) from routine activity.
                tracing::info!(
                    target: "pingward::session",
                    handle = %crate::auth::session_log_handle(session_id),
                    user_id,
                    ip = ip.as_deref(),
                    user_agent = user_agent.as_deref(),
                    expires_at = %renewal.expires_at.to_rfc3339(),
                    renewal = renewal.kind.as_str(),
                    "session.renewed"
                );
            }
            (true, None) => {
                sqlx::query("UPDATE sessions SET last_seen_at = $1 WHERE id = $2")
                    .bind(now.to_rfc3339())
                    .bind(session_id)
                    .execute(&self.pool)
                    .await?;
            }
            (false, None) => {}
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

    /// Unexpired sessions, newest-created first.
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
        let sessions: Vec<Session> = rows.iter().map(row_to_session).collect::<Result<_, _>>()?;
        // `/account` reaps capped rows first; this guards the other callers.
        Ok(sessions
            .into_iter()
            .filter(|s| !crate::auth::is_past_absolute_cap(s.created_at, now))
            .collect())
    }

    /// Delete `user_id`'s sessions past the absolute cap, so `/account` does not
    /// wait for the next prune to stop hiding them. `created_at <> ''` spares
    /// pre-0010 rows, as in [`Store::delete_expired_sessions`].
    pub async fn delete_capped_sessions_for_user(
        &self,
        user_id: i64,
        now: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error> {
        let cap_before =
            (now - chrono::Duration::days(crate::auth::SESSION_ABSOLUTE_MAX_DAYS)).to_rfc3339();
        let res = sqlx::query(
            "DELETE FROM sessions WHERE user_id = $1 AND created_at <> '' AND created_at <= $2",
        )
        .bind(user_id)
        .bind(cap_before)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    /// Owner-scoped, as in [`Store::delete_api_key`].
    pub async fn delete_session_owned(&self, id: &str, user_id: i64) -> Result<bool, sqlx::Error> {
        let res = sqlx::query("DELETE FROM sessions WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Delete every session for `user_id` except `keep_id`.
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

    /// For a password reset or a disabled account. When the operator is the
    /// target, use [`Store::delete_other_sessions_for_user`] to keep their own.
    pub async fn delete_sessions_for_user(&self, user_id: i64) -> Result<u64, sqlx::Error> {
        let res = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id)
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

    /// Every project with its owner's username, by project id.
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

    /// Instance-wide notification timezone; `None` when unset or blank. A read
    /// failure also yields `None`: it must not stop a down alert going out.
    pub async fn display_timezone(&self) -> Option<String> {
        self.get_setting("display_timezone")
            .await
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// One map per scan/nag pass, so naming projects costs a fixed query count.
    pub async fn all_project_names(&self) -> Result<HashMap<i64, String>, sqlx::Error> {
        let rows = sqlx::query("SELECT id, name FROM projects")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<i64, _>("id"), r.get::<String, _>("name")))
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

    /// `kind` is immutable: `config_json` only means something to the kind that wrote it.
    pub async fn update_channel(
        &self,
        id: i64,
        name: &str,
        config_json: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE channels SET name = $1, config_json = $2 WHERE id = $3")
            .bind(name)
            .bind(config_json)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
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
        // Not SQLite-only `INSERT OR IGNORE`, a parse error on Postgres.
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

    /// Binds a new check to every channel on its project. The `WHERE` is also
    /// syntax: in `INSERT … SELECT … ON CONFLICT`, `SQLite` needs it to tell the
    /// upsert's `ON` from a join's.
    pub async fn bind_all_project_channels(
        &self,
        check_id: i64,
        project_id: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO check_channels (check_id, channel_id) \
             SELECT $1, id FROM channels WHERE project_id = $2 \
             ON CONFLICT DO NOTHING",
        )
        .bind(check_id)
        .bind(project_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Which of `check_ids` have at least one bound channel, in one query.
    pub async fn checks_with_channels(
        &self,
        check_ids: &[i64],
    ) -> Result<HashSet<i64>, sqlx::Error> {
        if check_ids.is_empty() {
            return Ok(HashSet::new());
        }
        let placeholders = (1..=check_ids.len())
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT DISTINCT check_id FROM check_channels WHERE check_id IN ({placeholders})"
        );
        // Safe: only `$N` placeholders are interpolated.
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
        for id in check_ids {
            q = q.bind(*id);
        }
        let rows = q.fetch_all(&self.pool).await?;
        Ok(rows.iter().map(|r| r.get::<i64, _>("check_id")).collect())
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

    /// Batched [`Store::list_checks_for_project`], keyed by `project_id` (id
    /// order kept; projects with no checks absent).
    pub async fn list_checks_for_projects(
        &self,
        project_ids: &[i64],
    ) -> Result<HashMap<i64, Vec<Check>>, sqlx::Error> {
        if project_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = (1..=project_ids.len())
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT * FROM checks WHERE project_id IN ({placeholders}) ORDER BY project_id, id"
        );
        // Safe: only `$N` placeholders are interpolated.
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
        for id in project_ids {
            q = q.bind(*id);
        }
        let rows = q.fetch_all(&self.pool).await?;
        let mut map: HashMap<i64, Vec<Check>> = HashMap::new();
        for row in &rows {
            let check = row_to_check(row)?;
            map.entry(check.project_id).or_default().push(check);
        }
        Ok(map)
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

    /// The newest `limit` pings as [`PingSummary`]s (no `body`).
    pub async fn list_recent_ping_summaries(
        &self,
        check_id: i64,
        limit: i64,
    ) -> Result<Vec<PingSummary>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, check_id, kind, created_at FROM pings \
             WHERE check_id = $1 ORDER BY id DESC LIMIT $2",
        )
        .bind(check_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_ping_summary).collect()
    }

    /// Batched [`Store::list_recent_ping_summaries`]: the dashboard's hot query,
    /// hence no `body`. Checks with no pings are absent. `ROW_NUMBER()` needs
    /// `SQLite` >= 3.25.
    pub async fn list_recent_ping_summaries_for_checks(
        &self,
        check_ids: &[i64],
        per_check_limit: i64,
    ) -> Result<HashMap<i64, Vec<PingSummary>>, sqlx::Error> {
        if check_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = (1..=check_ids.len())
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, check_id, kind, created_at FROM ( \
               SELECT p.id, p.check_id, p.kind, p.created_at, \
                      ROW_NUMBER() OVER (PARTITION BY p.check_id ORDER BY p.id DESC) AS rn \
               FROM pings p WHERE p.check_id IN ({placeholders}) \
             ) sub WHERE rn <= ${} ORDER BY check_id, id DESC",
            check_ids.len() + 1
        );
        // Safe: only `$N` placeholders are interpolated.
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
        for id in check_ids {
            q = q.bind(*id);
        }
        q = q.bind(per_check_limit);
        let rows = q.fetch_all(&self.pool).await?;
        let mut map: HashMap<i64, Vec<PingSummary>> = HashMap::new();
        for row in &rows {
            let p = row_to_ping_summary(row)?;
            map.entry(p.check_id).or_default().push(p);
        }
        Ok(map)
    }

    /// A page of a check's pings for the check-detail table. Separate from the
    /// heartbeat strip's [`Store::list_recent_ping_summaries`], which paging and
    /// filtering must never affect.
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
            Some(("check_id", check_id)),
            cursor,
            limit,
            &filter.predicates(),
            row_to_ping,
        )
        .await
    }

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
            Some(("check_id", check_id)),
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

    /// `created_at` is RFC3339 UTC text, so the lexicographic `<` is chronological.
    pub async fn delete_pings_before(&self, cutoff: &str) -> Result<u64, sqlx::Error> {
        let r = sqlx::query("DELETE FROM pings WHERE created_at < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    /// Retention defaults to off, and changing it is itself audited
    /// (`settings.update`), so shortening the window leaves a trace.
    pub async fn delete_audit_before(&self, cutoff: &str) -> Result<u64, sqlx::Error> {
        let r = sqlx::query("DELETE FROM audit_log WHERE created_at < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    pub async fn delete_notifications_before(&self, cutoff: &str) -> Result<u64, sqlx::Error> {
        let r = sqlx::query("DELETE FROM notifications WHERE created_at < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    /// Delete sessions past their idle window or absolute cap. They are already
    /// unusable, so this is not retention-driven.
    pub async fn delete_expired_sessions(&self, now: DateTime<Utc>) -> Result<u64, sqlx::Error> {
        let cap_before =
            (now - chrono::Duration::days(crate::auth::SESSION_ABSOLUTE_MAX_DAYS)).to_rfc3339();
        let r = sqlx::query(
            // `created_at <> ''`: a pre-0010 row is unaged, not infinitely old.
            "DELETE FROM sessions WHERE expires_at <= $1 OR (created_at <> '' AND created_at <= $2)",
        )
        .bind(now.to_rfc3339())
        .bind(cap_before)
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

    /// Newest `limit` rows, unfiltered; for tests. `/admin` uses
    /// [`Store::list_audit_page`].
    pub async fn list_audit(&self, limit: i64) -> Result<Vec<AuditLog>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM audit_log ORDER BY id DESC LIMIT $1")
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_audit).collect()
    }

    pub async fn list_audit_page(
        &self,
        cursor: PageCursor,
        limit: i64,
        filter: &AuditFilter,
    ) -> Result<Page<AuditLog>, sqlx::Error> {
        keyset_page(
            &self.pool,
            "audit_log",
            None,
            cursor,
            limit,
            &filter.predicates(),
            row_to_audit,
        )
        .await
    }

    /// Distinct actors and actions for the audit filter's selects, read from the
    /// data so a new `record_audit` call site appears by itself.
    pub async fn audit_filter_options(&self) -> Result<(Vec<String>, Vec<String>), sqlx::Error> {
        let actors = sqlx::query_scalar("SELECT DISTINCT actor_username FROM audit_log ORDER BY 1")
            .fetch_all(&self.pool)
            .await?;
        let actions = sqlx::query_scalar("SELECT DISTINCT action FROM audit_log ORDER BY 1")
            .fetch_all(&self.pool)
            .await?;
        Ok((actors, actions))
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
        // Matches `view::display_status`: Running only applies on top of up/new.
        c.running = sqlx::query_scalar(
            "SELECT COUNT(*) FROM checks \
             WHERE status IN ('up','new') AND last_start_at IS NOT NULL \
             AND (last_ping_at IS NULL OR last_start_at > last_ping_at)",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(c)
    }

    /// Down checks with project name and owner, oldest `last_ping_at` first and
    /// never-pinged last (`IS NULL` sorts the same on both backends).
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

    /// Per-channel `(channel_name, ok, error)` since `cutoff`, most failures
    /// first. Cast to `BIGINT`: a bare `SUM()` may not decode as `i64` on Postgres.
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

    /// Handler tests exercise the pre-check, not the constraint, so only this
    /// catches `is_unique_violation` going blind through the `Any` driver.
    #[tokio::test]
    async fn a_duplicate_username_is_classified_not_swallowed() {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool, "sqlite::memory:").await.unwrap();
        let store = Store::new(pool);
        store
            .create_user("admin", Some("phc"), true, Utc::now())
            .await
            .unwrap();

        let err = store
            .create_user("admin", Some("other"), false, Utc::now())
            .await
            .expect_err("the UNIQUE constraint must refuse the second insert");
        assert!(
            matches!(err, CreateUserError::UsernameTaken),
            "expected UsernameTaken, got {err:?}"
        );
        // Also rule out a silently ignored insert.
        assert_eq!(store.count_users().await.unwrap(), 1);
    }

    /// Case-insensitivity would take a migration, not a validator tweak.
    #[tokio::test]
    async fn usernames_differing_only_in_case_are_distinct() {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool, "sqlite::memory:").await.unwrap();
        let store = Store::new(pool);
        store
            .create_user("admin", Some("phc"), true, Utc::now())
            .await
            .unwrap();
        store
            .create_user("Admin", Some("phc"), false, Utc::now())
            .await
            .expect("the constraint is case-sensitive on both backends");
        assert_eq!(store.count_users().await.unwrap(), 2);
    }
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

        // COALESCE keeps the timestamps, but next_due_at is overwritten to NULL.
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
                now + chrono::Duration::hours(1),
                Some("curl/8.0"),
                Some("127.0.0.1"),
                false,
                now,
            )
            .await
            .unwrap();
        let u = store
            .find_session_user("sess-1", now)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(u.id, uid);
        // That lookup also slid `expires_at` (1h left is under half of 72h).
        let rows = store.list_sessions_for_user(uid, now).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "sess-1");
        assert_eq!(rows[0].created_at, Some(now));
        assert_eq!(rows[0].last_seen_at, Some(now));
        assert_eq!(rows[0].user_agent.as_deref(), Some("curl/8.0"));
        assert_eq!(rows[0].ip.as_deref(), Some("127.0.0.1"));

        // Throttled: under 60s later, `last_seen_at` stays.
        store
            .find_session_user("sess-1", now + chrono::Duration::seconds(30))
            .await
            .unwrap();
        let rows = store.list_sessions_for_user(uid, now).await.unwrap();
        assert_eq!(rows[0].last_seen_at, Some(now));

        let later = now + chrono::Duration::seconds(61);
        store.find_session_user("sess-1", later).await.unwrap();
        let rows = store.list_sessions_for_user(uid, later).await.unwrap();
        assert_eq!(rows[0].last_seen_at, Some(later));

        let created2 = now + chrono::Duration::seconds(5);
        store
            .create_session(
                "sess-2",
                uid,
                now + chrono::Duration::hours(2),
                None,
                None,
                false,
                created2,
            )
            .await
            .unwrap();
        let rows = store.list_sessions_for_user(uid, later).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "sess-2", "newest-created session lists first");

        let removed = store
            .delete_other_sessions_for_user(uid, "sess-2")
            .await
            .unwrap();
        assert_eq!(removed, 1);
        let rows = store.list_sessions_for_user(uid, later).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "sess-2");

        // Another user's id is a silent no-op.
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
                now + chrono::Duration::hours(1),
                None,
                None,
                false,
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

        // `delete_sessions_for_user` keeps no row, unlike
        // `delete_other_sessions_for_user`, and spares other users.
        let other_uid = store
            .create_user("carol", Some("phc"), false, now)
            .await
            .unwrap();
        store
            .create_session(
                "sess-4",
                uid,
                now + chrono::Duration::hours(1),
                None,
                None,
                false,
                now,
            )
            .await
            .unwrap();
        store
            .create_session(
                "sess-5",
                uid,
                now + chrono::Duration::hours(1),
                None,
                None,
                false,
                now,
            )
            .await
            .unwrap();
        store
            .create_session(
                "sess-carol",
                other_uid,
                now + chrono::Duration::hours(1),
                None,
                None,
                false,
                now,
            )
            .await
            .unwrap();
        let removed = store.delete_sessions_for_user(uid).await.unwrap();
        assert_eq!(removed, 2);
        assert!(
            store
                .list_sessions_for_user(uid, now)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .list_sessions_for_user(other_uid, now)
                .await
                .unwrap()
                .len(),
            1
        );

        // Absolute cap: a session created 40 days ago is rejected and unlisted
        // even though its idle window has not lapsed.
        store
            .create_session(
                "sess-abs-cap",
                uid,
                now + chrono::Duration::hours(1),
                None,
                None,
                false,
                now - chrono::Duration::days(40),
            )
            .await
            .unwrap();
        assert!(
            store
                .find_session_user("sess-abs-cap", now)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .list_sessions_for_user(uid, now)
                .await
                .unwrap()
                .is_empty()
        );

        store
            .create_session(
                "sess-idle-expiry",
                uid,
                now + chrono::Duration::hours(1),
                None,
                None,
                false,
                now,
            )
            .await
            .unwrap();
        assert!(
            store
                .find_session_user("sess-idle-expiry", now + chrono::Duration::hours(2))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn find_session_user_does_not_slide_a_fresh_session() {
        let store = seeded().await;
        let uid = store
            .create_user("erin", Some("phc"), false, Utc::now())
            .await
            .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let original_expiry = now + chrono::Duration::hours(70); // > half of 72h
        store
            .create_session("sess-fresh", uid, original_expiry, None, None, false, now)
            .await
            .unwrap();

        store.find_session_user("sess-fresh", now).await.unwrap();

        let rows = sqlx::query("SELECT expires_at FROM sessions WHERE id = 'sess-fresh'")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        let expires_at: String = rows.get("expires_at");
        assert_eq!(expires_at, original_expiry.to_rfc3339());
    }

    #[tokio::test]
    async fn find_session_user_slides_expiry() {
        let store = seeded().await;
        let uid = store
            .create_user("frank", Some("phc"), false, Utc::now())
            .await
            .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        store
            .create_session(
                "sess-stale",
                uid,
                now + chrono::Duration::hours(1),
                None,
                None,
                false,
                now,
            )
            .await
            .unwrap();

        store.find_session_user("sess-stale", now).await.unwrap();

        let rows = sqlx::query("SELECT expires_at FROM sessions WHERE id = 'sess-stale'")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        let expires_at: String = rows.get("expires_at");
        let expected = now + chrono::Duration::hours(crate::auth::SESSION_IDLE_TTL_HOURS);
        assert_eq!(expires_at, expected.to_rfc3339());
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
                now,
                None,
                None,
                false,
                now - chrono::Duration::hours(1),
            )
            .await
            .unwrap();
        // expires_at in the future must survive.
        store
            .create_session(
                "sess-future",
                uid,
                now + chrono::Duration::hours(1),
                None,
                None,
                false,
                now - chrono::Duration::hours(1),
            )
            .await
            .unwrap();

        let deleted = store.delete_expired_sessions(now).await.unwrap();
        assert_eq!(deleted, 1);
        let rows = store.list_sessions_for_user(uid, now).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "sess-future");

        // Inside its idle window but past the absolute cap.
        store
            .create_session(
                "sess-abscap",
                uid,
                now + chrono::Duration::hours(1),
                None,
                None,
                false,
                now - chrono::Duration::days(40),
            )
            .await
            .unwrap();
        // A pre-0010 row (created_at = ''): only its idle window governs.
        sqlx::query(
            "INSERT INTO sessions (id, user_id, expires_at, created_at, sso) \
             VALUES ($1,$2,$3,'',0)",
        )
        .bind("sess-blank-created")
        .bind(uid)
        .bind((now + chrono::Duration::hours(1)).to_rfc3339())
        .execute(&store.pool)
        .await
        .unwrap();

        let deleted = store.delete_expired_sessions(now).await.unwrap();
        assert_eq!(deleted, 1, "only sess-abscap (past the cap) is reclaimed");
        let ids: Vec<String> = store
            .list_sessions_for_user(uid, now)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert!(!ids.contains(&"sess-abscap".to_string()));
        assert!(ids.contains(&"sess-future".to_string()));
        assert!(ids.contains(&"sess-blank-created".to_string()));
    }

    #[tokio::test]
    async fn session_sso_flag_round_trips() {
        let store = seeded().await;
        let uid = store
            .create_user("dave", Some("phc"), false, Utc::now())
            .await
            .unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();

        store
            .create_session(
                "sess-sso",
                uid,
                now + chrono::Duration::hours(1),
                None,
                None,
                true,
                now,
            )
            .await
            .unwrap();
        store
            .create_session(
                "sess-password",
                uid,
                now + chrono::Duration::hours(1),
                None,
                None,
                false,
                now,
            )
            .await
            .unwrap();

        let rows = store.list_sessions_for_user(uid, now).await.unwrap();
        let sso_row = rows.iter().find(|s| s.id == "sess-sso").unwrap();
        let password_row = rows.iter().find(|s| s.id == "sess-password").unwrap();
        assert!(sso_row.sso);
        assert!(!password_row.sso);
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
    async fn bind_all_project_channels_binds_every_project_channel() {
        let store = seeded().await;
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();

        let pid = store
            .create_project(1, "web", "", None, None, now)
            .await
            .unwrap();
        let other_pid = store
            .create_project(1, "other", "", None, None, now)
            .await
            .unwrap();

        let c1 = store
            .create_channel(pid, ChannelKind::Webhook, "hook1", "{}", now)
            .await
            .unwrap();
        let c2 = store
            .create_channel(pid, ChannelKind::Webhook, "hook2", "{}", now)
            .await
            .unwrap();
        let other_c = store
            .create_channel(other_pid, ChannelKind::Webhook, "hook-other", "{}", now)
            .await
            .unwrap();

        let chk = store
            .create_check(&NewCheck {
                project_id: pid,
                name: "job",
                ping_uuid: "uuid-bind-all",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();

        store.bind_all_project_channels(chk, pid).await.unwrap();
        let mut bound = store.bound_channel_ids(chk).await.unwrap();
        bound.sort_unstable();
        let mut expected = vec![c1, c2];
        expected.sort_unstable();
        assert_eq!(bound, expected, "should bind every channel of the project");
        assert!(
            !bound.contains(&other_c),
            "must not bind a channel belonging to a different project"
        );

        // Idempotent: calling again must not error or duplicate bindings.
        store.bind_all_project_channels(chk, pid).await.unwrap();
        let mut bound_again = store.bound_channel_ids(chk).await.unwrap();
        bound_again.sort_unstable();
        assert_eq!(bound_again, expected, "second call must be a no-op");
    }

    #[tokio::test]
    async fn checks_with_channels_reports_only_bound_checks() {
        let store = seeded().await;
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();

        let pid = store
            .create_project(1, "web", "", None, None, now)
            .await
            .unwrap();
        let cid = store
            .create_channel(pid, ChannelKind::Webhook, "hook", "{}", now)
            .await
            .unwrap();

        let with_channel = store
            .create_check(&NewCheck {
                project_id: pid,
                name: "with-channel",
                ping_uuid: "uuid-with-channel",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let without_channel = store
            .create_check(&NewCheck {
                project_id: pid,
                name: "without-channel",
                ping_uuid: "uuid-without-channel",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        store.bind_channel(with_channel, cid).await.unwrap();

        assert_eq!(
            store.checks_with_channels(&[]).await.unwrap(),
            HashSet::new(),
            "empty input must return an empty set without querying"
        );

        let result = store
            .checks_with_channels(&[with_channel, without_channel])
            .await
            .unwrap();
        assert!(
            result.contains(&with_channel),
            "the bound check must be reported"
        );
        assert!(
            !result.contains(&without_channel),
            "the unbound check must not be reported"
        );
        assert_eq!(
            result.len(),
            1,
            "must not invent ids that were not asked for"
        );
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

        store
            .insert_ping(cid, PingKind::Success, None, "", None, old)
            .await
            .unwrap();
        store
            .insert_ping(cid, PingKind::Success, None, "", None, recent)
            .await
            .unwrap();
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

    /// `n` audit rows one second apart; row `i` is `(actor{i%2}, action{i%2})`.
    async fn seeded_audit(n: i64) -> Store {
        let store = seeded().await;
        let base = Utc.with_ymd_and_hms(2026, 7, 14, 8, 0, 0).unwrap();
        for i in 0..n {
            let which = i % 2;
            store
                .record_audit(
                    &NewAudit {
                        actor_user_id: which + 1,
                        actor_username: if which == 0 { "alice" } else { "bob" },
                        action: if which == 0 {
                            "admin.access"
                        } else {
                            "user.create"
                        },
                        target_type: Some("project"),
                        target_id: Some(i),
                        ..Default::default()
                    },
                    base + Duration::seconds(i),
                )
                .await
                .unwrap();
        }
        store
    }

    /// The unscoped (`scope = None`) path through `keyset_page`.
    #[tokio::test]
    async fn list_audit_page_keyset_pagination() {
        let store = seeded_audit(25).await;
        let f = AuditFilter::default();

        let first = store
            .list_audit_page(PageCursor::Latest, 10, &f)
            .await
            .unwrap();
        assert_eq!(first.items.len(), 10);
        assert!(!first.has_newer, "the latest page has nothing newer");
        assert!(first.has_older);
        assert!(first.items[0].id > first.items[9].id);

        let older = store
            .list_audit_page(PageCursor::Before(first.items[9].id), 10, &f)
            .await
            .unwrap();
        assert_eq!(older.items.len(), 10);
        assert!(older.has_newer && older.has_older);
        assert!(older.items[0].id < first.items[9].id);

        let last = store
            .list_audit_page(PageCursor::Before(older.items[9].id), 10, &f)
            .await
            .unwrap();
        assert_eq!(last.items.len(), 5, "25 rows = 10 + 10 + 5");
        assert!(!last.has_older, "nothing older than the first row");

        // Paging back lands on the page we came from.
        let back = store
            .list_audit_page(PageCursor::After(older.items[0].id), 10, &f)
            .await
            .unwrap();
        let back_ids: Vec<i64> = back.items.iter().map(|a| a.id).collect();
        let first_ids: Vec<i64> = first.items.iter().map(|a| a.id).collect();
        assert_eq!(back_ids, first_ids);
    }

    #[tokio::test]
    async fn list_audit_page_filters_by_actor_action_and_date() {
        let store = seeded_audit(10).await;
        let base = Utc.with_ymd_and_hms(2026, 7, 14, 8, 0, 0).unwrap();

        let by_actor = store
            .list_audit_page(
                PageCursor::Latest,
                50,
                &AuditFilter {
                    actor: Some("bob".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(by_actor.items.len(), 5);
        assert!(by_actor.items.iter().all(|a| a.actor_username == "bob"));

        let by_action = store
            .list_audit_page(
                PageCursor::Latest,
                50,
                &AuditFilter {
                    action: Some("admin.access".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(by_action.items.len(), 5);
        assert!(by_action.items.iter().all(|a| a.action == "admin.access"));

        // AND-ed: alice never performs user.create.
        let contradictory = store
            .list_audit_page(
                PageCursor::Latest,
                50,
                &AuditFilter {
                    actor: Some("alice".into()),
                    action: Some("user.create".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(contradictory.items.is_empty());

        // Inclusive bounds: rows 2..=4.
        let windowed = store
            .list_audit_page(
                PageCursor::Latest,
                50,
                &AuditFilter {
                    from: Some(base + Duration::seconds(2)),
                    to: Some(base + Duration::seconds(4)),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(windowed.items.len(), 3);

        let nobody = store
            .list_audit_page(
                PageCursor::Latest,
                50,
                &AuditFilter {
                    actor: Some("nobody".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(nobody.items.is_empty() && !nobody.has_older);
    }

    #[tokio::test]
    async fn audit_filter_options_are_distinct_and_sorted() {
        let store = seeded_audit(6).await;
        let (actors, actions) = store.audit_filter_options().await.unwrap();
        assert_eq!(actors, vec!["alice".to_string(), "bob".to_string()]);
        assert_eq!(
            actions,
            vec!["admin.access".to_string(), "user.create".to_string()]
        );
    }

    #[tokio::test]
    async fn status_counts_and_scale() {
        let store = seeded().await;
        // `seeded()` already made user 'u'.
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

        // `b` starts and never finishes: stored `new`, running.
        let t1 = Utc::now();
        let t2 = t1 + Duration::seconds(1);
        store
            .mark_ping(bid, CheckStatus::New, None, Some(t1), None)
            .await
            .unwrap();

        // `c` succeeds, then starts again: running on top of `up`.
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

        // `d` fails, then starts again: `Down` beats `Running`.
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
        // `channels.name` is not unique: two same-named channels stay two rows.
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
        // Never-pinged B sorts last.
        assert!(
            ia < ib,
            "expected pinged check before never-pinged: {names:?}"
        );
    }

    #[tokio::test]
    async fn batch_checks_for_projects_matches_per_project() {
        let store = seeded().await;
        for name in ["p2", "p3"] {
            store
                .create_project(1, name, "", None, None, Utc::now())
                .await
                .unwrap();
        }
        // Interleaved, to catch rows leaking across groups or insertion-order reliance.
        for (project_id, name, uuid) in [
            (1, "a1", "u-a1"),
            (2, "b1", "u-b1"),
            (1, "a2", "u-a2"),
            (2, "b2", "u-b2"),
            (1, "a3", "u-a3"),
        ] {
            store
                .create_check(&NewCheck {
                    project_id,
                    name,
                    ping_uuid: uuid,
                    kind: ScheduleKind::Period,
                    period_secs: Some(60),
                    grace_secs: 30,
                    timezone: "UTC",
                    ..Default::default()
                })
                .await
                .unwrap();
        }

        let batch = store.list_checks_for_projects(&[1, 2, 3]).await.unwrap();
        assert_eq!(batch.get(&1).unwrap().len(), 3);
        assert_eq!(batch.get(&2).unwrap().len(), 2);
        assert!(!batch.contains_key(&3));
        // Same ids, same order as the per-project query.
        for pid in [1, 2, 3] {
            let single: Vec<i64> = store
                .list_checks_for_project(pid)
                .await
                .unwrap()
                .iter()
                .map(|c| c.id)
                .collect();
            let batched: Vec<i64> = batch
                .get(&pid)
                .map(|v| v.iter().map(|c| c.id).collect())
                .unwrap_or_default();
            assert_eq!(batched, single, "project {pid}");
        }
        // Empty input short-circuits (no `IN ()`).
        assert!(
            store
                .list_checks_for_projects(&[])
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// A drift from the wide query would silently redraw every strip.
    #[tokio::test]
    async fn ping_summaries_match_the_wide_query_row_for_row() {
        let store = seeded().await;
        let base = Utc.with_ymd_and_hms(2026, 7, 14, 8, 0, 0).unwrap();
        let mut ids = Vec::new();
        for n in 0..3 {
            let cid = store
                .create_check(&NewCheck {
                    project_id: 1,
                    name: &format!("c{n}"),
                    ping_uuid: &format!("u{n}"),
                    kind: ScheduleKind::Period,
                    period_secs: Some(60),
                    grace_secs: 10,
                    timezone: "UTC",
                    ..Default::default()
                })
                .await
                .unwrap();
            ids.push(cid);
            for i in 0..5 {
                store
                    .insert_ping(
                        cid,
                        if i % 2 == 0 {
                            PingKind::Success
                        } else {
                            PingKind::Start
                        },
                        Some(0),
                        &"x".repeat(4096),
                        Some("10.0.0.1"),
                        base + chrono::Duration::seconds(i),
                    )
                    .await
                    .unwrap();
            }
        }

        // A check with no pings must be absent from the batch, not empty.
        let silent = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "silent",
                ping_uuid: "u-silent",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 10,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let mut queried = ids.clone();
        queried.push(silent);

        let narrow = store
            .list_recent_ping_summaries_for_checks(&queried, 3)
            .await
            .unwrap();
        assert_eq!(narrow.len(), ids.len());
        assert!(!narrow.contains_key(&silent));
        for cid in &ids {
            let w: Vec<PingSummary> = store
                .list_recent_pings(*cid, 3)
                .await
                .unwrap()
                .iter()
                .map(Into::into)
                .collect();
            assert_eq!(w.len(), 3, "per-check limit honored");
            let n = narrow.get(cid).unwrap();
            assert_eq!(&w, n, "batched, check {cid}");
            let single = store.list_recent_ping_summaries(*cid, 3).await.unwrap();
            assert_eq!(w, single, "per-check, check {cid}");
        }

        assert!(
            store
                .list_recent_ping_summaries_for_checks(&[], 3)
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
        // Real auto-increment ids, oldest to newest.
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

        // After(id1) -> [id3, id2]; id4 and id5 are still newer.
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
