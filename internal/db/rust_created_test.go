package db

import (
	"context"
	"encoding/hex"
	"path/filepath"
	"testing"
)

// sqlxMigrationsTable is the statement sqlx stored in sqlite_master for
// _sqlx_migrations, as captured from a database created by the Rust v0.12.0
// image (ghcr.io/henry40408/pingward, revision ca5682f).
const sqlxMigrationsTable = `CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
)`

// sqlxExecutionTimes is execution_time (nanoseconds) of versions 1 to 16 in the
// same capture. sqlx wrote every installed_on as 2026-11-17 15:24:38.
var sqlxExecutionTimes = []int64{
	463876, 151125, 131625, 692459, 114167, 90334, 104458, 81917,
	121792, 445250, 310501, 526667, 152458, 100792, 32708, 95500,
}

// openRustCreated recreates a database the Rust image left behind: the schema
// its migrations built (the Go build embeds the same files, which the checksums
// in sqliteMigrationGolden pin), sqlx's bookkeeping table with its rows, and the
// three capture users.
func openRustCreated(t *testing.T) *DB {
	t.Helper()
	database, err := Open("sqlite://" + filepath.Join(t.TempDir(), "rust.sqlite3"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = database.Close() })
	ctx := context.Background()
	exec := func(query string, args ...any) {
		t.Helper()
		if _, err := database.ExecContext(ctx, query, args...); err != nil {
			t.Fatal(err)
		}
	}

	exec(sqlxMigrationsTable)
	files, err := loadMigrations(migrationFiles, SQLite)
	if err != nil {
		t.Fatal(err)
	}
	for _, file := range files {
		exec(file.sql)
	}
	rows := parseMigrationGolden(t, sqliteMigrationGolden)
	if len(rows) != len(sqlxExecutionTimes) {
		t.Fatalf("%d golden rows, %d execution times", len(rows), len(sqlxExecutionTimes))
	}
	for i, row := range rows {
		checksum, err := hex.DecodeString(row.checksum)
		if err != nil {
			t.Fatal(err)
		}
		exec(`INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time)
VALUES (?, ?, '2026-11-17 15:24:38', 1, ?, ?)`, row.version, row.description, checksum, sqlxExecutionTimes[i])
	}
	exec(`INSERT INTO users (username, is_admin, created_at) VALUES
('admin', 1, '2026-11-17T15:24:56.085137657+00:00'),
('bob', 1, '2026-11-17T15:25:00.989983645+00:00'),
('carol', 0, '2026-11-17T15:30:10.190704932+00:00')`)
	return database
}
