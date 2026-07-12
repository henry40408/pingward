use crate::db::Pool;
use crate::models::{Check, CheckStatus, PingKind, ScheduleKind};
use chrono::{DateTime, Utc};
use sqlx::Row;
use std::str::FromStr;

#[derive(Clone)]
pub struct Store {
    pub pool: Pool,
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
fn row_to_check(row: &sqlx::sqlite::SqliteRow) -> Result<Check, sqlx::Error> {
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
        created_at,
    })
}

impl Store {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    pub async fn find_check_by_uuid(&self, uuid: &str) -> Result<Option<Check>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM checks WHERE ping_uuid = ?")
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
            "INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at) VALUES (?,?,?,?,?,?)",
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
            "UPDATE checks SET status=?, last_ping_at=COALESCE(?, last_ping_at), \
             last_start_at=COALESCE(?, last_start_at), next_due_at=? WHERE id=?",
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
        sqlx::query("UPDATE checks SET status=? WHERE id=?")
            .bind(status.as_str())
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
        let res = sqlx::query(
            "INSERT INTO checks (project_id, name, ping_uuid, schedule_kind, period_secs, \
             grace_secs, cron_expr, timezone, status, created_at) VALUES (?,?,?,?,?,?,?,?, 'new', ?)",
        )
        .bind(project_id).bind(name).bind(ping_uuid).bind(kind.as_str())
        .bind(period_secs).bind(grace_secs).bind(cron_expr).bind(timezone)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool).await?;
        Ok(res.last_insert_rowid())
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
        db::migrate(&pool).await.unwrap();
        sqlx::query("INSERT INTO users (username, is_admin, created_at) VALUES ('u', 0, ?)")
            .bind(Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO projects (user_id, name, created_at) VALUES (1, 'p', ?)")
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

        let row = sqlx::query("SELECT * FROM pings WHERE check_id = ?")
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
             VALUES (1, 'x', 'bad-status-uuid', 'period', 'bogus', ?)",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&store.pool)
        .await;
        assert!(
            res.is_err(),
            "expected CHECK constraint to reject bad status"
        );
    }
}
