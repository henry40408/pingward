package db

import (
	"bufio"
	"context"
	"crypto/sha512"
	"encoding/hex"
	"errors"
	"fmt"
	"hash/crc32"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"testing/fstest"
)

type migrationRow struct {
	version     int64
	description string
	checksum    string
	success     bool
}

func readMigrationRows(t *testing.T, database *DB) []migrationRow {
	t.Helper()
	rows, err := database.QueryContext(context.Background(),
		"SELECT version, description, checksum, success FROM _sqlx_migrations ORDER BY version")
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	var out []migrationRow
	for rows.Next() {
		var row migrationRow
		var checksum []byte
		if err := rows.Scan(&row.version, &row.description, &checksum, &row.success); err != nil {
			t.Fatal(err)
		}
		row.checksum = hex.EncodeToString(checksum)
		out = append(out, row)
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	return out
}

// readGolden parses "<version> <description…> <sha384 hex>" lines.
func readGolden(t *testing.T, name string) []migrationRow {
	t.Helper()
	file, err := os.Open(filepath.Join("testdata", name))
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	var out []migrationRow
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		fields := strings.Fields(scanner.Text())
		if len(fields) < 3 {
			continue
		}
		var row migrationRow
		if _, err := fmt.Sscan(fields[0], &row.version); err != nil {
			t.Fatal(err)
		}
		row.description = strings.Join(fields[1:len(fields)-1], " ")
		row.checksum = fields[len(fields)-1]
		row.success = true
		out = append(out, row)
	}
	return out
}

func TestGoMigratedDatabaseMatchesTheRustGolden(t *testing.T) {
	database := openMemory(t)
	if err := database.Migrate(context.Background()); err != nil {
		t.Fatal(err)
	}
	got, want := readMigrationRows(t, database), readGolden(t, "sqlx_migrations.sqlite.txt")
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Fatalf("got  %v\nwant %v", got, want)
	}
}

func TestPostgresGoldenIsTheFileChecksums(t *testing.T) {
	dialectFiles, err := loadMigrations(migrationFiles, Postgres)
	if err != nil {
		t.Fatal(err)
	}
	var got []migrationRow
	for _, m := range dialectFiles {
		got = append(got, migrationRow{m.version, m.description, hex.EncodeToString(m.checksum), true})
	}
	if want := readGolden(t, "sqlx_migrations.postgres.txt"); fmt.Sprint(got) != fmt.Sprint(want) {
		t.Fatalf("got  %v\nwant %v", got, want)
	}
}

func TestMigrationFilesAreChecksummedByteForByte(t *testing.T) {
	loaded, err := loadMigrations(migrationFiles, SQLite)
	if err != nil {
		t.Fatal(err)
	}
	first := loaded[0]
	content, err := migrationFiles.ReadFile("migrations/sqlite/0001_init.sql")
	if err != nil {
		t.Fatal(err)
	}
	sum := sha512.Sum384(content)
	if first.version != 1 || first.description != "init" || hex.EncodeToString(first.checksum) != hex.EncodeToString(sum[:]) {
		t.Fatalf("%+v", first)
	}
}

func copyFile(t *testing.T, from, to string) {
	t.Helper()
	source, err := os.Open(from)
	if err != nil {
		t.Fatal(err)
	}
	defer source.Close()
	target, err := os.Create(to)
	if err != nil {
		t.Fatal(err)
	}
	defer target.Close()
	if _, err := io.Copy(target, source); err != nil {
		t.Fatal(err)
	}
}

func TestMigratingARustCreatedDatabaseIsANoOp(t *testing.T) {
	path := filepath.Join(t.TempDir(), "rust.sqlite3")
	copyFile(t, filepath.Join("testdata", "rust-created.sqlite3"), path)
	database, err := Open("sqlite://" + path)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()

	before := scanString(t, database, "SELECT group_concat(execution_time) FROM _sqlx_migrations")
	if err := database.Migrate(context.Background()); err != nil {
		t.Fatal(err)
	}
	if after := scanString(t, database, "SELECT group_concat(execution_time) FROM _sqlx_migrations"); after != before {
		t.Fatalf("a no-op migrate rewrote rows: %s -> %s", before, after)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM _sqlx_migrations"); got != 16 {
		t.Fatalf("%d rows", got)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM users"); got != 3 {
		t.Fatalf("%d users", got)
	}
}

func TestGoCreatedTableDDLMatchesSqlx(t *testing.T) {
	database := openMemory(t)
	if err := database.Migrate(context.Background()); err != nil {
		t.Fatal(err)
	}
	rust := openRustCopy(t)
	query := "SELECT sql FROM sqlite_master WHERE name = '_sqlx_migrations'"
	if got, want := scanString(t, database, query), scanString(t, rust, query); got != want {
		t.Fatalf("got:\n%s\nwant:\n%s", got, want)
	}
}

func openRustCopy(t *testing.T) *DB {
	t.Helper()
	path := filepath.Join(t.TempDir(), "rust.sqlite3")
	copyFile(t, filepath.Join("testdata", "rust-created.sqlite3"), path)
	database, err := Open("sqlite://" + path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = database.Close() })
	return database
}

func TestMigrationRecordsARealExecutionTime(t *testing.T) {
	database := openMemory(t)
	if err := database.Migrate(context.Background()); err != nil {
		t.Fatal(err)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM _sqlx_migrations WHERE execution_time <= 0 OR success = 0"); got != 0 {
		t.Fatalf("%d rows still have the placeholder execution_time", got)
	}
}

func TestMigrateTwiceIsIdempotent(t *testing.T) {
	database := openMemory(t)
	for range 2 {
		if err := database.Migrate(context.Background()); err != nil {
			t.Fatal(err)
		}
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM _sqlx_migrations"); got != 16 {
		t.Fatalf("%d rows", got)
	}
}

func testFiles(files map[string]string) fstest.MapFS {
	fsys := fstest.MapFS{}
	for name, body := range files {
		fsys["migrations/sqlite/"+name] = &fstest.MapFile{Data: []byte(body)}
	}
	return fsys
}

func TestDirtyMigrationIsRefused(t *testing.T) {
	database := openMemory(t)
	ctx := context.Background()
	files := testFiles(map[string]string{"0001_a.sql": "CREATE TABLE a (x INTEGER);"})
	if err := database.migrateFrom(ctx, files); err != nil {
		t.Fatal(err)
	}
	if _, err := database.ExecContext(ctx, "UPDATE _sqlx_migrations SET success = false"); err != nil {
		t.Fatal(err)
	}
	var dirty DirtyError
	if err := database.migrateFrom(ctx, files); !errors.As(err, &dirty) || dirty.Version != 1 {
		t.Fatalf("got %v", err)
	}
}

func TestMissingAndModifiedMigrationsAreRefused(t *testing.T) {
	database := openMemory(t)
	ctx := context.Background()
	original := testFiles(map[string]string{
		"0001_a.sql": "CREATE TABLE a (x INTEGER);",
		"0002_b.sql": "CREATE TABLE b (x INTEGER);",
	})
	if err := database.migrateFrom(ctx, original); err != nil {
		t.Fatal(err)
	}
	var missing VersionMissingError
	onlyFirst := testFiles(map[string]string{"0001_a.sql": "CREATE TABLE a (x INTEGER);"})
	if err := database.migrateFrom(ctx, onlyFirst); !errors.As(err, &missing) || missing.Version != 2 {
		t.Fatalf("missing: got %v", err)
	}
	var mismatch VersionMismatchError
	edited := testFiles(map[string]string{
		"0001_a.sql": "CREATE TABLE a (x INTEGER); ",
		"0002_b.sql": "CREATE TABLE b (x INTEGER);",
	})
	if err := database.migrateFrom(ctx, edited); !errors.As(err, &mismatch) || mismatch.Version != 1 {
		t.Fatalf("mismatch: got %v", err)
	}
}

func TestFailedMigrationRollsBackAndLeavesNoRow(t *testing.T) {
	database := openMemory(t)
	ctx := context.Background()
	files := testFiles(map[string]string{
		"0001_a.sql":   "CREATE TABLE a (x INTEGER);",
		"0002_bad.sql": "CREATE TABLE b (x INTEGER); INSERT INTO nope VALUES (1);",
	})
	if err := database.migrateFrom(ctx, files); err == nil {
		t.Fatal("a failing migration must fail the run")
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM _sqlx_migrations"); got != 1 {
		t.Fatalf("%d rows, want only the first migration", got)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM sqlite_master WHERE name = 'b'"); got != 0 {
		t.Fatal("the failed migration's DDL was not rolled back")
	}
}

func TestMigrationNameParsing(t *testing.T) {
	m, err := parseMigration("0003_max_runtime.sql", []byte("x"))
	if err != nil || m.version != 3 || m.description != "max runtime" {
		t.Fatalf("%+v %v", m, err)
	}
	for _, bad := range []string{"init.sql", "x_init.sql"} {
		if _, err := parseMigration(bad, nil); err == nil {
			t.Errorf("%q must be refused", bad)
		}
	}
	if _, err := parseMigration("0001_a.sql", []byte("-- no-transaction\nSELECT 1")); err == nil {
		t.Error("no-transaction migrations are unsupported and must be refused")
	}
}

func TestAdvisoryLockIDWrapsLikeSqlx(t *testing.T) {
	// crc32("pingward") * 0x3d32ad9e does not fit in int64's positive range for
	// every database name; it must wrap exactly as Rust's wrapping_mul does.
	for _, name := range []string{"", "pingward", "postgres", "a-very-long-database-name"} {
		crc := uint64(crc32.ChecksumIEEE([]byte(name)))
		want := int64(uint64(0x3d32ad9e) * crc) // uint64 product cannot overflow; reinterpreting gives the wrapped int64
		if got := advisoryLockID(name); got != want {
			t.Errorf("%q: got %d, want %d", name, got, want)
		}
	}
}

// The wording is sqlx's, so a Rust-era runbook still matches the message.
func TestMigrationErrorsReadLikeSqlx(t *testing.T) {
	cases := []struct {
		err  error
		want string
	}{
		{DirtyError{Version: 3}, "migration 3 is partially applied; fix and remove row from `_sqlx_migrations` table"},
		{VersionMissingError{Version: 4}, "migration 4 was previously applied but is missing in the resolved migrations"},
		{VersionMismatchError{Version: 5}, "migration 5 was previously applied but has been modified"},
	}
	for _, tc := range cases {
		if got := tc.err.Error(); got != tc.want {
			t.Errorf("got %q, want %q", got, tc.want)
		}
	}
}
