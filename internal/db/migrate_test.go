package db

import (
	"context"
	"database/sql"
	"hash/crc32"
	"regexp"
	"slices"
	"strings"
	"sync"
	"testing"
	"testing/fstest"
)

func mustMigrate(t *testing.T, database *DB) {
	t.Helper()
	if err := database.Migrate(context.Background()); err != nil {
		t.Fatal(err)
	}
}

func scanStrings(t *testing.T, database *DB, query string) []string {
	t.Helper()
	rows, err := database.QueryContext(context.Background(), query)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = rows.Close() }()
	var values []string
	for rows.Next() {
		var value string
		if err := rows.Scan(&value); err != nil {
			t.Fatal(err)
		}
		values = append(values, value)
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	return values
}

const recordedVersions = "SELECT version FROM _sqlx_migrations ORDER BY version"

// schemaSignature describes what the tables behave like, ignoring how the
// statements that created them were written.
const schemaSignature = `
SELECT 'column ' || m.name || ' ' || c.cid || ' ' || c.name || ' ' || c.type || ' ' || c."notnull" || ' ' ||
    coalesce(c.dflt_value, '-') || ' ' || c.pk
  FROM sqlite_master m, pragma_table_xinfo(m.name) c WHERE m.type = 'table'
UNION ALL
SELECT 'foreign key ' || m.name || ' ' || f."from" || ' ' || f."table" || ' ' || f."to" || ' ' || f.on_delete
  FROM sqlite_master m, pragma_foreign_key_list(m.name) f WHERE m.type = 'table'
UNION ALL
SELECT 'index ' || m.name || ' ' || i.name || ' ' || i."unique" || ' ' || i.origin || ' ' || i.partial || ' ' ||
    k.seqno || ' ' || k.name || ' ' || k."desc" || ' ' || k.coll
  FROM sqlite_master m, pragma_index_list(m.name) i, pragma_index_xinfo(i.name) k
  WHERE m.type = 'table' AND k."key"
ORDER BY 1`

// No pragma reports these two.
var tableClause = regexp.MustCompile(`CHECK\(\w+IN\([^)]*\)\)|AUTOINCREMENT`)

func readSchema(t *testing.T, database *DB) []string {
	t.Helper()
	signature := scanStrings(t, database, schemaSignature)
	tables := scanStrings(t, database, "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
	for _, table := range tables {
		statement := scanString(t, database, "SELECT sql FROM sqlite_master WHERE name = '"+table+"'")
		compact := strings.Join(strings.Fields(regexp.MustCompile(`--[^\n]*`).ReplaceAllString(statement, "")), "")
		signature = append(signature, table+" "+strings.Join(tableClause.FindAllString(compact, -1), " "))
	}
	return signature
}

func TestMigratingAFreshDatabaseRecordsTheVersionAndTheSentinel(t *testing.T) {
	database := openMemory(t)
	mustMigrate(t, database)
	if got := scanInt(t, database, "PRAGMA user_version"); got != 16 {
		t.Fatalf("user_version = %d", got)
	}
	if got := scanStrings(t, database, recordedVersions); !slices.Equal(got, []string{"99999999999999"}) {
		t.Fatalf("records %v", got)
	}
}

func TestMigrateTwiceIsIdempotent(t *testing.T) {
	database := openMemory(t)
	mustMigrate(t, database)
	mustMigrate(t, database)
	if got := scanInt(t, database, "SELECT COUNT(*) FROM _sqlx_migrations"); got != 1 {
		t.Fatalf("%d rows", got)
	}
}

func TestUpgradingAnOldDatabaseKeepsItsRows(t *testing.T) {
	database := openOld(t)
	mustMigrate(t, database)
	if got := scanInt(t, database, "PRAGMA user_version"); got != 16 {
		t.Fatalf("user_version = %d", got)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM _sqlx_migrations"); got != 17 {
		t.Fatalf("%d records, want the 16 old ones and the sentinel", got)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM users"); got != 2 {
		t.Fatalf("%d users", got)
	}
}

// An old database skips the baseline script, so the runner writes the row itself.
func TestOldAndFreshDatabasesGetTheSameSentinel(t *testing.T) {
	const sentinel = `SELECT description || '|' || success || '|' || typeof(checksum) || '|' ||
		length(checksum) || '|' || execution_time FROM _sqlx_migrations WHERE version = 99999999999999`
	old, fresh := openOld(t), openMemory(t)
	mustMigrate(t, old)
	mustMigrate(t, fresh)
	if got, want := scanString(t, old, sentinel), scanString(t, fresh, sentinel); got != want {
		t.Fatalf("old %q, fresh %q", got, want)
	}
}

func TestLitestreamTablesDoNotMakeANewFileLookUsed(t *testing.T) {
	database := openMemory(t)
	mustExec(t, database, "CREATE TABLE _litestream_seq (id INTEGER PRIMARY KEY, seq INTEGER)")
	mustExec(t, database, "CREATE TABLE _litestream_lock (id INTEGER)")
	mustMigrate(t, database)
}

// The baseline replaces the old steps, so it must build the same schema.
func TestOldAndFreshDatabasesHaveTheSameSchema(t *testing.T) {
	old, fresh := openOld(t), openMemory(t)
	mustMigrate(t, old)
	mustMigrate(t, fresh)
	got, want := readSchema(t, fresh), readSchema(t, old)
	if !slices.Equal(got, want) {
		t.Fatalf("fresh:\n%s\nold:\n%s", strings.Join(got, "\n"), strings.Join(want, "\n"))
	}
}

// Restoring a SQLite `.dump` loses user_version.
func TestADatabaseThatLostItsVersionMigratesAgain(t *testing.T) {
	for name, database := range map[string]*DB{"fresh": openMemory(t), "upgraded": openOld(t)} {
		mustMigrate(t, database)
		records := scanStrings(t, database, recordedVersions)
		mustExec(t, database, "PRAGMA user_version = 0")
		mustMigrate(t, database)
		if got := scanInt(t, database, "PRAGMA user_version"); got != 16 {
			t.Errorf("%s: user_version = %d", name, got)
		}
		if got := scanStrings(t, database, recordedVersions); !slices.Equal(got, records) {
			t.Errorf("%s: records %v, want %v", name, got, records)
		}
	}
}

func TestDatabasesWithoutACompleteRecordAreRefusedUntouched(t *testing.T) {
	cases := map[string]struct {
		open    func(*testing.T) *DB
		damage  string
		message string
	}{
		"another program's file":   {openMemory, "CREATE TABLE other (x INTEGER)", "no complete record of schema version 16"},
		"records stop short":       {openOld, "DELETE FROM _sqlx_migrations WHERE version = 16", "no complete record"},
		"another program's record": {openOld, "UPDATE _sqlx_migrations SET description = 'x' WHERE version = 16", "no complete record"},
		"a failed step":            {openOld, "UPDATE _sqlx_migrations SET success = FALSE WHERE version = 3", "no complete record"},
		"no records at all":        {openOld, "DROP TABLE _sqlx_migrations", "no complete record"},
		"records of another shape": {openMemory, "CREATE TABLE _sqlx_migrations (x INTEGER)", "read old migration records"},
	}
	for name, tc := range cases {
		t.Run(name, func(t *testing.T) {
			database := tc.open(t)
			mustExec(t, database, tc.damage)
			before := readSchema(t, database)
			err := database.Migrate(context.Background())
			if err == nil || !strings.Contains(err.Error(), tc.message) {
				t.Fatalf("got %v", err)
			}
			if got := readSchema(t, database); !slices.Equal(got, before) {
				t.Errorf("schema changed:\n%s", strings.Join(got, "\n"))
			}
			if got := scanInt(t, database, "PRAGMA user_version"); got != 0 {
				t.Errorf("user_version = %d", got)
			}
		})
	}
}

func testFiles(files map[string]string) fstest.MapFS {
	fsys := fstest.MapFS{}
	for name, body := range files {
		fsys["migrations/sqlite/"+name] = &fstest.MapFile{Data: []byte(body)}
	}
	return fsys
}

func TestLaterMigrationsApplyInOrderAndOnce(t *testing.T) {
	database := openMemory(t)
	ctx := context.Background()
	first := map[string]string{"0001_a.sql": "CREATE TABLE a (x INTEGER);"}
	if err := database.migrateFrom(ctx, testFiles(first)); err != nil {
		t.Fatal(err)
	}
	all := testFiles(map[string]string{
		"0001_a.sql": first["0001_a.sql"],
		"2_b.sql":    "INSERT INTO a VALUES (2);",
		"10_c.sql":   "INSERT INTO a SELECT 10 FROM a WHERE x = 2;",
	})
	for range 2 {
		if err := database.migrateFrom(ctx, all); err != nil {
			t.Fatal(err)
		}
	}
	if got := scanStrings(t, database, "SELECT x FROM a ORDER BY rowid"); !slices.Equal(got, []string{"2", "10"}) {
		t.Fatalf("rows %v", got)
	}
	if got := scanInt(t, database, "PRAGMA user_version"); got != 10 {
		t.Fatalf("user_version = %d", got)
	}
}

// Only a database below the baseline has to prove where it comes from.
func TestOldRecordsDoNotMatterFromTheBaselineOn(t *testing.T) {
	database := openMemory(t)
	mustExec(t, database, "CREATE TABLE a (x INTEGER)")
	mustExec(t, database, "PRAGMA user_version = 1")
	files := testFiles(map[string]string{
		"0001_a.sql": "CREATE TABLE a (x INTEGER);",
		"0002_b.sql": "CREATE TABLE b (x INTEGER);",
	})
	if err := database.migrateFrom(context.Background(), files); err != nil {
		t.Fatal(err)
	}
	if got := scanInt(t, database, "PRAGMA user_version"); got != 2 {
		t.Fatalf("user_version = %d", got)
	}
}

func TestANewerDatabaseIsRefused(t *testing.T) {
	database := openMemory(t)
	mustExec(t, database, "PRAGMA user_version = 17")
	err := database.Migrate(context.Background())
	if err == nil || !strings.Contains(err.Error(), "version 17 is newer than 16") {
		t.Fatalf("got %v", err)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM sqlite_master"); got != 0 {
		t.Fatalf("%d objects were created", got)
	}
}

func TestAFailedMigrationLeavesNeitherSchemaNorVersion(t *testing.T) {
	database := openMemory(t)
	files := testFiles(map[string]string{
		"0001_a.sql":   "CREATE TABLE a (x INTEGER);",
		"0002_bad.sql": "CREATE TABLE b (x INTEGER); INSERT INTO nope VALUES (1);",
	})
	err := database.migrateFrom(context.Background(), files)
	if err == nil || !strings.HasPrefix(err.Error(), "migration 2: ") {
		t.Fatalf("got %v", err)
	}
	if got := scanInt(t, database, "PRAGMA user_version"); got != 1 {
		t.Fatalf("user_version = %d", got)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM sqlite_master WHERE name = 'b'"); got != 0 {
		t.Fatal("the failed migration's DDL was not rolled back")
	}
}

// Another process may migrate between the first read and the write lock.
func TestApplyMigrationReadsTheVersionAgain(t *testing.T) {
	database := openMemory(t)
	ctx := context.Background()
	step := migration{version: 2, sql: "CREATE TABLE a (x INTEGER);"}
	known := versionRange{baseline: 1, latest: 2}

	mustExec(t, database, "PRAGMA user_version = 2")
	if err := database.applyMigration(ctx, step, known); err != nil {
		t.Fatal(err)
	}
	mustExec(t, database, "PRAGMA user_version = 3")
	if err := database.applyMigration(ctx, step, known); err == nil || !strings.Contains(err.Error(), "newer than 2") {
		t.Fatalf("got %v", err)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM sqlite_master"); got != 0 {
		t.Fatal("the step ran again")
	}
}

func TestAnUpToDateDatabaseStartsWhileAnotherProcessWrites(t *testing.T) {
	database, url := openFile(t)
	mustMigrate(t, database)
	writer, err := Open(url)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = writer.Close() }()
	tx, err := writer.BeginTx(context.Background(), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = tx.Rollback() }()
	mustMigrate(t, database)
}

func TestConcurrentSQLiteStartsMigrateOnce(t *testing.T) {
	// Neither step survives a second run.
	files := testFiles(map[string]string{
		"0001_a.sql": "CREATE TABLE a (x INTEGER);",
		"0002_b.sql": "ALTER TABLE a ADD COLUMN y INTEGER;",
	})
	_, url := openFile(t)
	const runners = 4
	errs := make([]error, runners)
	var wg sync.WaitGroup
	for i := range runners {
		database, err := Open(url)
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { _ = database.Close() })
		wg.Add(1)
		go func() {
			defer wg.Done()
			errs[i] = database.migrateFrom(context.Background(), files)
		}()
	}
	wg.Wait()
	for i, err := range errs {
		if err != nil {
			t.Errorf("runner %d: %v", i, err)
		}
	}
}

func TestMigrateReportsAnUnreadableVersion(t *testing.T) {
	database := openMemory(t)
	if err := database.Close(); err != nil {
		t.Fatal(err)
	}
	err := database.Migrate(context.Background())
	if err == nil || !strings.Contains(err.Error(), "read schema version") {
		t.Fatalf("got %v", err)
	}
}

func TestMigrationFilesAreOrderedByNumber(t *testing.T) {
	files := testFiles(map[string]string{"10_c.sql": "c", "2_b.sql": "b", "2147483647_d.sql": "d"})
	loaded, err := loadMigrations(files, SQLite)
	if err != nil {
		t.Fatal(err)
	}
	want := []migration{{2, "b"}, {10, "c"}, {2147483647, "d"}}
	if !slices.Equal(loaded, want) {
		t.Fatalf("got %v", loaded)
	}
}

func TestBadMigrationFilesAreRefused(t *testing.T) {
	cases := map[string]struct {
		files   map[string]string
		message string
	}{
		"none":             {nil, "no migration files"},
		"no version":       {map[string]string{"init.sql": ""}, "is not named"},
		"no description":   {map[string]string{"0001.sql": ""}, "is not named"},
		"version zero":     {map[string]string{"0000_a.sql": ""}, "must be from 1"},
		"version too big":  {map[string]string{"2147483648_a.sql": ""}, "must be from 1"},
		"duplicate number": {map[string]string{"0001_a.sql": "", "1_b.sql": ""}, "share version 1"},
	}
	for name, tc := range cases {
		database := openMemory(t)
		err := database.migrateFrom(context.Background(), testFiles(tc.files))
		if err == nil || !strings.Contains(err.Error(), tc.message) {
			t.Errorf("%s: got %v", name, err)
		}
	}
}

func TestBothDialectsHaveTheSameVersions(t *testing.T) {
	versions := func(dialect Dialect) []int64 {
		loaded, err := loadMigrations(migrationFiles, dialect)
		if err != nil {
			t.Fatal(err)
		}
		var out []int64
		for _, m := range loaded {
			out = append(out, m.version)
		}
		return out
	}
	sqlite, postgres := versions(SQLite), versions(Postgres)
	if !slices.Equal(sqlite, postgres) || sqlite[0] != 16 {
		t.Fatalf("sqlite %v, postgres %v", sqlite, postgres)
	}
}

// The Postgres tests need a server; this holds without one.
func TestOnlyPostgresAsksForAnIsolationLevel(t *testing.T) {
	if got := (&DB{dialect: SQLite}).migrationTxOptions(); got != nil {
		t.Errorf("SQLite: %+v", got)
	}
	got := (&DB{dialect: Postgres}).migrationTxOptions()
	if got == nil || got.Isolation != sql.LevelReadCommitted {
		t.Errorf("Postgres: %+v", got)
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
