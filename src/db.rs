use sqlx::migrate::Migrator;
use std::path::Path;

pub type Pool = sqlx::SqlitePool;

pub async fn connect(url: &str) -> Result<Pool, sqlx::Error> {
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await
}

pub async fn migrate(pool: &Pool) -> Result<(), sqlx::Error> {
    // Foreign keys are off by default in SQLite; enable per-connection is ideal,
    // but for the pool we enable here for the migration path.
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(pool)
        .await?;
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
}
