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

fn row_to_check(row: &sqlx::sqlite::SqliteRow) -> Check {
    Check {
        id: row.get("id"),
        project_id: row.get("project_id"),
        name: row.get("name"),
        ping_uuid: row.get("ping_uuid"),
        schedule_kind: ScheduleKind::from_str(row.get::<String, _>("schedule_kind").as_str())
            .unwrap(),
        period_secs: row.get("period_secs"),
        grace_secs: row.get("grace_secs"),
        cron_expr: row.get("cron_expr"),
        timezone: row.get("timezone"),
        status: CheckStatus::from_str(row.get::<String, _>("status").as_str()).unwrap(),
        last_ping_at: parse_ts(row.get("last_ping_at")),
        last_start_at: parse_ts(row.get("last_start_at")),
        next_due_at: parse_ts(row.get("next_due_at")),
        scan_interval_secs: row.get("scan_interval_secs"),
        created_at: parse_ts(row.get("created_at")).unwrap_or_else(Utc::now),
    }
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
        Ok(row.as_ref().map(row_to_check))
    }

    pub async fn list_active_checks(&self) -> Result<Vec<Check>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM checks WHERE status IN ('new','up')")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_check).collect())
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
    use chrono::Utc;

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
        store
            .insert_ping(
                id,
                PingKind::Success,
                None,
                "hello",
                Some("1.2.3.4"),
                Utc::now(),
            )
            .await
            .unwrap();
        assert_eq!(store.list_active_checks().await.unwrap().len(), 1);
        store.set_status(id, CheckStatus::Paused).await.unwrap();
        assert_eq!(store.list_active_checks().await.unwrap().len(), 0);
    }
}
