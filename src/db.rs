use sqlx::any::{install_default_drivers, AnyConnectOptions, AnyPoolOptions};
use sqlx::migrate::Migrator;
use std::path::Path;
use std::str::FromStr;

pub type Pool = sqlx::AnyPool;

/// SQLite's `:memory:` database is scoped to a single physical connection.
fn is_in_memory_url(url: &str) -> bool {
    url.contains(":memory:") || url.contains("mode=memory")
}

fn is_sqlite_url(url: &str) -> bool {
    url.starts_with("sqlite:")
}

pub async fn connect(url: &str) -> Result<Pool, sqlx::Error> {
    // The `Any` driver dispatches to whichever concrete driver a URL names;
    // its default drivers must be registered once before connecting.
    install_default_drivers();

    let sqlite = is_sqlite_url(url);
    // Cap in-memory SQLite to one connection so all operations share the one
    // in-memory database. Postgres and file SQLite use a small pool.
    let max_connections = if sqlite && is_in_memory_url(url) {
        1
    } else {
        5
    };

    // Before the `Any` driver migration, SQLite connections were opened with
    // `SqliteConnectOptions::create_if_missing(true)`, so any SQLite file URL
    // auto-created the database file. The `Any` driver has no equivalent
    // builder option; the SQLite backend instead honours `?mode=rwc` in the
    // URL itself. Append it here for file URLs that don't already specify a
    // `mode=` so behaviour matches the pre-migration default (in-memory URLs
    // and URLs that already set `mode=` are left untouched).
    let created_url;
    let url = if sqlite && !is_in_memory_url(url) && !url.contains("mode=") {
        created_url = if url.contains('?') {
            format!("{url}&mode=rwc")
        } else {
            format!("{url}?mode=rwc")
        };
        created_url.as_str()
    } else {
        url
    };

    let opts = AnyConnectOptions::from_str(url)?;

    AnyPoolOptions::new()
        .max_connections(max_connections)
        .after_connect(move |conn, _meta| {
            Box::pin(async move {
                // `foreign_keys` is a per-connection SQLite pragma. Under the
                // `Any` driver we cannot set `SqliteConnectOptions.foreign_keys`,
                // so enable it on every new SQLite connection here (otherwise
                // `ON DELETE CASCADE` is silently unenforced). Postgres enforces
                // foreign keys natively and needs no pragma.
                if sqlite {
                    sqlx::query("PRAGMA foreign_keys = ON")
                        .execute(conn)
                        .await?;
                }
                Ok(())
            })
        })
        .connect_with(opts)
        .await
}

pub async fn migrate(pool: &Pool, url: &str) -> Result<(), sqlx::Error> {
    let dir = if is_sqlite_url(url) {
        "migrations/sqlite"
    } else {
        "migrations/postgres"
    };
    let m = Migrator::new(Path::new(dir)).await?;
    m.run(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrate_creates_checks_table() {
        let pool = connect("sqlite::memory:").await.unwrap();
        migrate(&pool, "sqlite::memory:").await.unwrap();
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
        migrate(&pool, "sqlite::memory:").await.unwrap();

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

    /// Regression test for a SQLite file URL with no `?mode=` query param:
    /// pre-`Any`-driver, `create_if_missing(true)` made this auto-create the
    /// database file. `connect()` must still do so by appending `mode=rwc`.
    #[tokio::test]
    async fn connect_creates_sqlite_file_without_mode_param() {
        let path = std::env::temp_dir().join("pingward_dbtest_autocreate.sqlite3");
        let _ = std::fs::remove_file(&path);
        assert!(!path.exists());

        let url = format!("sqlite://{}", path.display());
        let pool = connect(&url).await.unwrap();
        migrate(&pool, &url).await.unwrap();

        assert!(
            path.exists(),
            "connect() should auto-create the sqlite file"
        );

        drop(pool);
        let _ = std::fs::remove_file(&path);
    }
}
