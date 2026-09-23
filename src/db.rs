use sqlx::any::{AnyConnectOptions, AnyPoolOptions, install_default_drivers};
use sqlx::migrate::Migrator;
use std::str::FromStr;

// Embedded: the release image ships only the binary, with no `migrations/` on disk.
static SQLITE_MIGRATOR: Migrator = sqlx::migrate!("migrations/sqlite");
static POSTGRES_MIGRATOR: Migrator = sqlx::migrate!("migrations/postgres");

pub type Pool = sqlx::AnyPool;

/// `SQLite`'s `:memory:` database is scoped to a single physical connection.
fn is_in_memory_url(url: &str) -> bool {
    url.contains(":memory:") || url.contains("mode=memory")
}

fn is_sqlite_url(url: &str) -> bool {
    url.starts_with("sqlite:")
}

pub async fn connect(url: &str) -> Result<Pool, sqlx::Error> {
    install_default_drivers();

    let sqlite = is_sqlite_url(url);
    let max_connections = if sqlite && is_in_memory_url(url) {
        1
    } else {
        5
    };

    // The `Any` driver has no `create_if_missing`; `?mode=rwc` creates the file instead.
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

    // An in-memory database's journal mode is always `memory`.
    let sqlite_file = sqlite && !is_in_memory_url(url);

    let opts = AnyConnectOptions::from_str(url)?;

    AnyPoolOptions::new()
        .max_connections(max_connections)
        .after_connect(move |conn, _meta| {
            Box::pin(async move {
                // Per connection: the `Any` driver has no `SqliteConnectOptions`.
                if sqlite {
                    // Without this, `ON DELETE CASCADE` is silently unenforced.
                    sqlx::query("PRAGMA foreign_keys = ON")
                        .execute(&mut *conn)
                        .await?;
                    // Retry a blocked write for up to 5s instead of `SQLITE_BUSY`.
                    sqlx::query("PRAGMA busy_timeout = 5000")
                        .execute(&mut *conn)
                        .await?;
                }
                if sqlite_file {
                    // Readers run alongside a writer; NORMAL is safe under WAL.
                    sqlx::query("PRAGMA journal_mode = WAL")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA synchronous = NORMAL")
                        .execute(&mut *conn)
                        .await?;
                }
                Ok(())
            })
        })
        .connect_with(opts)
        .await
}

pub async fn migrate(pool: &Pool, url: &str) -> Result<(), sqlx::Error> {
    let m = if is_sqlite_url(url) {
        &SQLITE_MIGRATOR
    } else {
        &POSTGRES_MIGRATOR
    };
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

    #[tokio::test]
    async fn sqlite_file_connection_sets_busy_timeout_and_wal() {
        let path = std::env::temp_dir().join("pingward_dbtest_pragmas.sqlite3");
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }

        let url = format!("sqlite://{}", path.display());
        let pool = connect(&url).await.unwrap();

        let busy: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(busy, 5000, "busy_timeout must be 5000ms on file SQLite");

        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal", "file SQLite must use WAL");

        drop(pool);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    /// Asserting the sidecars exist before the close keeps this a test of the close.
    #[tokio::test]
    async fn closing_the_pool_checkpoints_and_removes_wal_sidecars() {
        let path = std::env::temp_dir().join("pingward_dbtest_close.sqlite3");
        let sidecars = ["-wal", "-shm"].map(|s| format!("{}{s}", path.display()));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }

        let url = format!("sqlite://{}", path.display());
        let pool = connect(&url).await.unwrap();
        migrate(&pool, &url).await.unwrap();
        sqlx::query("INSERT INTO settings (key, value) VALUES ('probe', 'v')")
            .execute(&pool)
            .await
            .unwrap();

        for f in &sidecars {
            assert!(
                std::path::Path::new(f).exists(),
                "a written WAL database must have {f} while the pool is open"
            );
        }

        pool.close().await;

        for f in &sidecars {
            assert!(
                !std::path::Path::new(f).exists(),
                "{f} must be gone after a clean pool close (WAL checkpointed)"
            );
        }

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn sqlite_memory_sets_busy_timeout_but_not_wal() {
        let pool = connect("sqlite::memory:").await.unwrap();

        let busy: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            busy, 5000,
            "busy_timeout must be 5000ms on in-memory SQLite"
        );

        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_ne!(
            mode.to_lowercase(),
            "wal",
            "WAL must not be applied to in-memory SQLite"
        );
    }

    /// `set_current_dir` is safe only because nextest runs each test in its own process.
    #[tokio::test]
    async fn migrate_works_without_migrations_dir_on_disk() {
        let cwd = std::env::temp_dir();
        assert!(
            !cwd.join("migrations").exists(),
            "test precondition: {} must not contain a migrations/ directory",
            cwd.display()
        );
        std::env::set_current_dir(&cwd).unwrap();

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
