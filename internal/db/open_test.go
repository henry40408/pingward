package db

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func openMemory(t *testing.T) *DB {
	t.Helper()
	database, err := Open("sqlite::memory:")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = database.Close() })
	return database
}

func openFile(t *testing.T) (database *DB, url string) {
	t.Helper()
	url = "sqlite://" + filepath.Join(t.TempDir(), "pingward.sqlite3")
	database, err := Open(url)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = database.Close() })
	return database, url
}

func scanInt(t *testing.T, database *DB, query string) int64 {
	t.Helper()
	var value int64
	if err := database.QueryRowContext(context.Background(), query).Scan(&value); err != nil {
		t.Fatal(err)
	}
	return value
}

func scanString(t *testing.T, database *DB, query string) string {
	t.Helper()
	var value string
	if err := database.QueryRowContext(context.Background(), query).Scan(&value); err != nil {
		t.Fatal(err)
	}
	return value
}

func TestMigrateCreatesChecksTable(t *testing.T) {
	database := openMemory(t)
	if err := database.Migrate(context.Background()); err != nil {
		t.Fatal(err)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='checks'"); got != 1 {
		t.Fatalf("checks table count = %d", got)
	}
}

func TestCascadeDeleteRemovesDependentProjectAndCheck(t *testing.T) {
	database := openMemory(t)
	ctx := context.Background()
	if err := database.Migrate(ctx); err != nil {
		t.Fatal(err)
	}
	for _, statement := range []string{
		"INSERT INTO users (id, username, created_at) VALUES (1, 'alice', '2024-01-01T00:00:00Z')",
		"INSERT INTO projects (id, user_id, name, created_at) VALUES (1, 1, 'proj', '2024-01-01T00:00:00Z')",
		"INSERT INTO checks (id, project_id, name, ping_uuid, schedule_kind, created_at) VALUES (1, 1, 'chk', 'uuid-1', 'period', '2024-01-01T00:00:00Z')",
		"DELETE FROM users WHERE id = 1",
	} {
		if _, err := database.ExecContext(ctx, statement); err != nil {
			t.Fatal(err)
		}
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM projects"); got != 0 {
		t.Errorf("project should cascade-delete with user, %d left", got)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM checks"); got != 0 {
		t.Errorf("check should cascade-delete with project, %d left", got)
	}
}

func TestSQLiteFileConnectionSetsBusyTimeoutAndWAL(t *testing.T) {
	database, _ := openFile(t)
	if got := scanInt(t, database, "PRAGMA busy_timeout"); got != 5000 {
		t.Errorf("busy_timeout = %d", got)
	}
	if got := strings.ToLower(scanString(t, database, "PRAGMA journal_mode")); got != "wal" {
		t.Errorf("journal_mode = %s", got)
	}
	if got := scanInt(t, database, "PRAGMA synchronous"); got != 1 {
		t.Errorf("synchronous = %d, want 1 (NORMAL)", got)
	}
	if got := scanInt(t, database, "PRAGMA foreign_keys"); got != 1 {
		t.Errorf("foreign_keys = %d", got)
	}
}

func TestClosingThePoolCheckpointsAndRemovesWALSidecars(t *testing.T) {
	database, url := openFile(t)
	ctx := context.Background()
	if err := database.Migrate(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := database.ExecContext(ctx, "INSERT INTO settings (key, value) VALUES ('probe', 'v')"); err != nil {
		t.Fatal(err)
	}
	path := strings.TrimPrefix(url, "sqlite://")
	sidecars := []string{path + "-wal", path + "-shm"}
	for _, sidecar := range sidecars {
		if _, err := os.Stat(sidecar); err != nil {
			t.Fatalf("a written WAL database must have %s while open: %v", sidecar, err)
		}
	}
	if err := database.Close(); err != nil {
		t.Fatal(err)
	}
	for _, sidecar := range sidecars {
		if _, err := os.Stat(sidecar); err == nil {
			t.Errorf("%s must be gone after a clean close", sidecar)
		}
	}
}

func TestSQLiteMemorySetsBusyTimeoutButNotWAL(t *testing.T) {
	database := openMemory(t)
	if got := scanInt(t, database, "PRAGMA busy_timeout"); got != 5000 {
		t.Errorf("busy_timeout = %d", got)
	}
	if got := strings.ToLower(scanString(t, database, "PRAGMA journal_mode")); got == "wal" {
		t.Error("WAL must not be applied to in-memory SQLite")
	}
}

func TestMemoryDatabaseSurvivesAcrossStatements(t *testing.T) {
	database := openMemory(t)
	ctx := context.Background()
	if _, err := database.ExecContext(ctx, "CREATE TABLE t (a INTEGER)"); err != nil {
		t.Fatal(err)
	}
	if database.Stats().MaxOpenConnections != 1 {
		t.Fatal("an in-memory database needs exactly one connection")
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM sqlite_master WHERE name='t'"); got != 1 {
		t.Fatal("the table vanished between statements")
	}
}

func TestMigrateWorksWithoutAMigrationsDirOnDisk(t *testing.T) {
	t.Chdir(t.TempDir())
	database := openMemory(t)
	if err := database.Migrate(context.Background()); err != nil {
		t.Fatal(err)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='checks'"); got != 1 {
		t.Fatal("migrations must be embedded")
	}
}

func TestOpenCreatesTheSQLiteFileWithoutAModeParam(t *testing.T) {
	database, url := openFile(t)
	path := strings.TrimPrefix(url, "sqlite://")
	if err := database.Migrate(context.Background()); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); err != nil {
		t.Fatalf("Open must auto-create the sqlite file: %v", err)
	}
}

func TestOpenHonoursAnExplicitReadOnlyMode(t *testing.T) {
	path := filepath.Join(t.TempDir(), "missing.sqlite3")
	if _, err := Open("sqlite://" + path + "?mode=ro"); err == nil {
		t.Fatal("mode=ro on a missing file must fail, not create it")
	}
}

func TestSQLiteDSN(t *testing.T) {
	cases := map[string]string{
		"sqlite::memory:":             "file::memory:?_pragma=foreign_keys(1)&_pragma=busy_timeout(5000)",
		"sqlite://x.db":               "file:x.db?mode=rwc&_pragma=foreign_keys(1)&_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)",
		"sqlite:///data/p.db?mode=rw": "file:/data/p.db?mode=rw&_pragma=foreign_keys(1)&_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)",
		"sqlite://a%b#c.db":           "file:a%25b%23c.db?mode=rwc&_pragma=foreign_keys(1)&_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)",
		"sqlite:x.db?cache=shared":    "file:x.db?cache=shared&mode=rwc&_pragma=foreign_keys(1)&_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)",
		"sqlite://x.db?mode=memory":   "file::memory:?_pragma=foreign_keys(1)&_pragma=busy_timeout(5000)",
	}
	for url, want := range cases {
		if got := sqliteDSN(url); got != want {
			t.Errorf("%s:\n got %s\nwant %s", url, got, want)
		}
	}
}

func TestIsSQLiteURL(t *testing.T) {
	if !IsSQLiteURL("sqlite::memory:") || !IsSQLiteURL("sqlite://x") || IsSQLiteURL("postgres://h/db") {
		t.Fatal("URL classification is wrong")
	}
}
