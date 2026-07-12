use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

pub type Pool = sqlx::SqlitePool;

/// SQLite's `:memory:` database is scoped to a single physical connection: every
/// new connection opens its own empty in-memory database. A pool of more than one
/// connection against an in-memory URL would risk intermittent "no such table"
/// errors depending on which connection happens to serve a given query.
fn is_in_memory_url(url: &str) -> bool {
    url.contains(":memory:") || url.contains("mode=memory")
}

pub async fn connect(url: &str) -> Result<Pool, sqlx::Error> {
    // `foreign_keys` is a per-connection SQLite pragma, so it must be set on the
    // connect options rather than executed once against the pool: the latter only
    // affects whichever single connection happens to run that statement, leaving
    // every other pooled connection (and thus `ON DELETE CASCADE`) unenforced.
    let options = SqliteConnectOptions::from_str(url)?
        .foreign_keys(true)
        .create_if_missing(true);

    // Cap in-memory databases to a single connection so all operations share the
    // one in-memory database instead of racing across isolated per-connection copies.
    let max_connections = if is_in_memory_url(url) { 1 } else { 5 };

    SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await
}

pub async fn migrate(pool: &Pool) -> Result<(), sqlx::Error> {
    let m = Migrator::new(Path::new("migrations/sqlite")).await?;
    m.run(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrate_creates_checks_table() {
        let pool = connect("sqlite::memory:").await.unwrap();
        migrate(&pool).await.unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='checks'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn cascade_delete_removes_dependent_project_and_check() {
        let pool = connect("sqlite::memory:").await.unwrap();
        migrate(&pool).await.unwrap();

        sqlx::query(
            "INSERT INTO users (id, username, created_at) VALUES (1, 'alice', '2024-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO projects (id, user_id, name, created_at) VALUES (1, 1, 'proj', '2024-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO checks (id, project_id, name, ping_uuid, schedule_kind, created_at) \
             VALUES (1, 1, 'chk', 'uuid-1', 'period', '2024-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query("DELETE FROM users WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();

        let project_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(project_count, 0, "project should cascade-delete with user");

        let check_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM checks")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(check_count, 0, "check should cascade-delete with project");
    }
}
