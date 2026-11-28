package db

import (
	"context"
	"net/url"
	"os"
	"slices"
	"strings"
	"sync"
	"testing"
	"testing/fstest"
)

// openTestPostgres returns a database emptied of every table, or skips without
// TEST_DATABASE_URL. Run these tests with -p 1: they share one schema.
func openTestPostgres(t *testing.T) *DB {
	t.Helper()
	target := os.Getenv("TEST_DATABASE_URL")
	if target == "" {
		t.Skip("TEST_DATABASE_URL is not set")
	}
	database, err := Open(target)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = database.Close() })
	if _, err := database.DB.ExecContext(context.Background(), "DROP SCHEMA public CASCADE; CREATE SCHEMA public"); err != nil {
		t.Fatal(err)
	}
	return database
}

const (
	pgVersionRows = "SELECT version FROM pingward_schema_version"
	pgHasVersion  = "SELECT count(*) FROM pg_catalog.pg_tables WHERE schemaname = 'public' AND tablename = 'pingward_schema_version'"
)

// openOldPostgres makes a database look as a release up to 0.13 left it: the same
// tables, recorded in _sqlx_migrations only.
func openOldPostgres(t *testing.T) *DB {
	t.Helper()
	database := openTestPostgres(t)
	mustMigrate(t, database)
	mustExec(t, database, "DROP TABLE pingward_schema_version")
	mustExec(t, database, "DELETE FROM _sqlx_migrations")
	insertOldRecords(t, database)
	mustExec(t, database, "INSERT INTO users (username, created_at) VALUES ('admin', '2026-11-17T15:24:56+00:00')")
	return database
}

func TestPostgresMigratesAFreshDatabaseOnce(t *testing.T) {
	database := openTestPostgres(t)
	mustMigrate(t, database)
	mustMigrate(t, database)
	if got := scanStrings(t, database, pgVersionRows); !slices.Equal(got, []string{"16"}) {
		t.Fatalf("version %v", got)
	}
	if got := scanStrings(t, database, recordedVersions); !slices.Equal(got, []string{"99999999999999"}) {
		t.Fatalf("records %v", got)
	}
}

func TestPostgresUpgradesAnOldDatabase(t *testing.T) {
	database := openOldPostgres(t)
	mustMigrate(t, database)
	if got := scanStrings(t, database, pgVersionRows); !slices.Equal(got, []string{"16"}) {
		t.Fatalf("version %v", got)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM _sqlx_migrations"); got != 17 {
		t.Fatalf("%d records", got)
	}
	if got := scanInt(t, database, "SELECT COUNT(*) FROM users"); got != 1 {
		t.Fatalf("%d users", got)
	}
}

// A role that may create tables but owns none of them can still upgrade.
func TestPostgresUpgradeNeedsNoTableOwnership(t *testing.T) {
	database := openOldPostgres(t)
	// SET ROLE lasts as long as its connection, so there must be only one.
	database.SetMaxOpenConns(1)
	mustExec(t, database, "DROP ROLE IF EXISTS pingward_test_app; CREATE ROLE pingward_test_app")
	t.Cleanup(func() {
		mustExec(t, database, "RESET ROLE; DROP OWNED BY pingward_test_app; DROP ROLE pingward_test_app")
	})
	mustExec(t, database, `GRANT USAGE, CREATE ON SCHEMA public TO pingward_test_app;
		GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO pingward_test_app;
		SET ROLE pingward_test_app`)
	mustMigrate(t, database)
	if got := scanInt(t, database, "SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 99999999999999"); got != 1 {
		t.Fatalf("%d sentinels", got)
	}
}

func TestPostgresRefusesTablesWithoutACompleteRecord(t *testing.T) {
	cases := map[string]struct {
		open   func(*testing.T) *DB
		damage string
	}{
		"records stop short": {openOldPostgres, "DELETE FROM _sqlx_migrations WHERE version = 16"},
		"no records at all":  {openTestPostgres, "CREATE TABLE users (id BIGINT)"},
	}
	for name, tc := range cases {
		t.Run(name, func(t *testing.T) {
			database := tc.open(t)
			mustExec(t, database, tc.damage)
			err := database.Migrate(context.Background())
			if err == nil || !strings.Contains(err.Error(), "no complete record of schema version 16") {
				t.Fatalf("got %v", err)
			}
			if got := scanInt(t, database, pgHasVersion); got != 0 {
				t.Error("the version table was created")
			}
		})
	}
}

func TestPostgresSchemaMayHoldOtherPrograms(t *testing.T) {
	database := openTestPostgres(t)
	mustExec(t, database, "CREATE TABLE other (x BIGINT)")
	mustMigrate(t, database)
}

// Other programs may keep a version table in the same schema, and another
// pingward one further along the search_path.
func TestPostgresReadsOnlyItsOwnVersionTable(t *testing.T) {
	database := openTestPostgres(t)
	// SET lasts as long as its connection, so there must be only one.
	database.SetMaxOpenConns(1)
	mustExec(t, database, `DROP SCHEMA IF EXISTS pingward_test CASCADE; CREATE SCHEMA pingward_test;
		CREATE TABLE pingward_test.schema_version (version TEXT);
		CREATE TABLE public.pingward_schema_version (version BIGINT);
		INSERT INTO public.pingward_schema_version VALUES (4000);
		SET search_path = pingward_test, public`)
	t.Cleanup(func() { mustExec(t, database, "DROP SCHEMA pingward_test CASCADE") })
	mustMigrate(t, database)
	if got := scanInt(t, database, "SELECT COUNT(*) FROM pg_catalog.pg_tables WHERE schemaname = 'pingward_test' AND tablename = 'users'"); got != 1 {
		t.Fatal("the schema was not created")
	}
}

func TestPostgresRefusesANewerSchema(t *testing.T) {
	database := openTestPostgres(t)
	mustMigrate(t, database)
	mustExec(t, database, "UPDATE pingward_schema_version SET version = 17")
	err := database.Migrate(context.Background())
	if err == nil || !strings.Contains(err.Error(), "version 17 is newer than 16") {
		t.Fatalf("got %v", err)
	}
}

// openSerializablePostgres is openTestPostgres with transactions that default to
// SERIALIZABLE, under which a snapshot taken before the lock would go stale.
func openSerializablePostgres(t *testing.T) *DB {
	t.Helper()
	openTestPostgres(t)
	target, err := url.Parse(os.Getenv("TEST_DATABASE_URL"))
	if err != nil {
		t.Fatal(err)
	}
	query := target.Query()
	query.Set("default_transaction_isolation", "serializable")
	target.RawQuery = query.Encode()
	database, err := Open(target.String())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = database.Close() })
	return database
}

func TestPostgresConcurrentMigratesSerialiseOnTheAdvisoryLock(t *testing.T) {
	database := openSerializablePostgres(t)
	// Neither step survives a second run.
	files := fstest.MapFS{
		"migrations/postgres/0001_a.sql": {Data: []byte("CREATE TABLE a (x BIGINT);")},
		"migrations/postgres/0002_b.sql": {Data: []byte("ALTER TABLE a ADD COLUMN y BIGINT;")},
	}
	const runners = 4
	errs := make([]error, runners)
	var wg sync.WaitGroup
	for i := range runners {
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
	if got := scanStrings(t, database, pgVersionRows); !slices.Equal(got, []string{"2"}) {
		t.Fatalf("version %v", got)
	}
}

func TestPostgresRebindLeavesDollarPlaceholdersAlone(t *testing.T) {
	database := openTestPostgres(t)
	var first, second int
	if err := database.QueryRowContext(context.Background(), "SELECT $2::int, $1::int", 10, 20).Scan(&first, &second); err != nil {
		t.Fatal(err)
	}
	if first != 20 || second != 10 {
		t.Fatalf("%d %d", first, second)
	}
}
