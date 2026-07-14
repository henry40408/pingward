use crate::db::Pool;
use crate::models::{
    AuditLog, Channel, ChannelKind, Check, CheckStatus, Notification, NotifyStatus, Ping, PingKind,
    Project, ScheduleKind, User,
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

fn row_to_project(row: &sqlx::any::AnyRow) -> Result<Project, sqlx::Error> {
    Ok(Project {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
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
                    continue;
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
                    continue;
                }
            }
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
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

    #[allow(clippy::too_many_arguments)]
    pub async fn create_check(
        &self,
        project_id: i64,
        name: &str,
        ping_uuid: &str,
        kind: ScheduleKind,
        period_secs: Option<i64>,
        grace_secs: i64,
        cron_expr: Option<&str>,
        timezone: &str,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query(
            "INSERT INTO checks (project_id, name, ping_uuid, schedule_kind, period_secs, \
             grace_secs, cron_expr, timezone, status, created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8, 'new', $9) \
             RETURNING id",
        )
        .bind(project_id).bind(name).bind(ping_uuid).bind(kind.as_str())
        .bind(period_secs).bind(grace_secs).bind(cron_expr).bind(timezone)
        .bind(Utc::now().to_rfc3339())
        .fetch_one(&self.pool).await?;
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

    pub async fn create_session(
        &self,
        id: &str,
        user_id: i64,
        expires_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO sessions (id, user_id, expires_at) VALUES ($1,$2,$3)")
            .bind(id)
            .bind(user_id)
            .bind(expires_at.to_rfc3339())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn find_session_user(
        &self,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<User>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT u.* FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.id = $1 AND s.expires_at > $2",
        )
        .bind(session_id)
        .bind(now.to_rfc3339())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(row_to_user).transpose()
    }

    pub async fn delete_session(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM sessions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // --- projects ---
    pub async fn create_project(
        &self,
        user_id: i64,
        name: &str,
        scan_interval_secs: Option<i64>,
        nag_interval_secs: Option<i64>,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query(
            "INSERT INTO projects (user_id, name, scan_interval_secs, nag_interval_secs, created_at) \
             VALUES ($1,$2,$3,$4,$5) RETURNING id",
        )
        .bind(user_id)
        .bind(name)
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
        scan_interval_secs: Option<i64>,
        nag_interval_secs: Option<i64>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE projects SET name = $1, scan_interval_secs = $2, nag_interval_secs = $3 WHERE id = $4",
        )
        .bind(name)
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

    #[allow(clippy::too_many_arguments)]
    pub async fn update_check_schedule(
        &self,
        id: i64,
        name: &str,
        kind: ScheduleKind,
        period_secs: Option<i64>,
        grace_secs: i64,
        cron_expr: Option<&str>,
        timezone: &str,
        scan_interval_secs: Option<i64>,
        max_runtime_secs: Option<i64>,
        nag_interval_secs: Option<i64>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE checks SET name=$1, schedule_kind=$2, period_secs=$3, grace_secs=$4, \
             cron_expr=$5, timezone=$6, scan_interval_secs=$7, max_runtime_secs=$8, \
             nag_interval_secs=$9 WHERE id=$10",
        )
        .bind(name)
        .bind(kind.as_str())
        .bind(period_secs)
        .bind(grace_secs)
        .bind(cron_expr)
        .bind(timezone)
        .bind(scan_interval_secs)
        .bind(max_runtime_secs)
        .bind(nag_interval_secs)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db,
        models::{CheckStatus, PingKind, ScheduleKind},
    };
    use chrono::{TimeZone, Utc};

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
            .create_check(
                1,
                "job",
                "uuid-1",
                ScheduleKind::Period,
                Some(60),
                30,
                None,
                "UTC",
            )
            .await
            .unwrap();
        let found = store.find_check_by_uuid("uuid-1").await.unwrap().unwrap();
        assert_eq!(found.id, id);
        assert_eq!(found.status, CheckStatus::New);
        assert!(store.find_check_by_uuid("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn insert_ping_and_list_active() {
        let store = seeded().await;
        let id = store
            .create_check(
                1,
                "job",
                "u",
                ScheduleKind::Period,
                Some(60),
                30,
                None,
                "UTC",
            )
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
            .create_check(
                1,
                "job",
                "up-uuid",
                ScheduleKind::Period,
                Some(60),
                30,
                None,
                "UTC",
            )
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
            .create_check(
                1,
                "job",
                "mark-uuid",
                ScheduleKind::Period,
                Some(60),
                30,
                None,
                "UTC",
            )
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
        assert!(store
            .find_user_by_username("nobody")
            .await
            .unwrap()
            .is_none());

        store
            .create_session("sess-1", uid, now + chrono::Duration::hours(1))
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
        assert!(store
            .find_session_user("sess-1", now + chrono::Duration::hours(2))
            .await
            .unwrap()
            .is_none());
        // deleted
        store.delete_session("sess-1").await.unwrap();
        assert!(store
            .find_session_user("sess-1", now)
            .await
            .unwrap()
            .is_none());
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
    async fn project_channel_binding_and_settings() {
        let store = seeded().await;
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();

        let pid = store
            .create_project(1, "web", Some(15), None, now)
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
            .create_check(
                pid,
                "job",
                "uuid-x",
                ScheduleKind::Period,
                Some(60),
                30,
                None,
                "UTC",
            )
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
            .create_check(
                1,
                "c",
                "uu",
                ScheduleKind::Period,
                Some(60),
                30,
                None,
                "UTC",
            )
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
            .create_check(
                1,
                "c",
                "uu",
                ScheduleKind::Period,
                Some(60),
                30,
                None,
                "UTC",
            )
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
            .create_check(
                1,
                "c",
                "uu",
                ScheduleKind::Period,
                Some(60),
                30,
                None,
                "UTC",
            )
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
}
