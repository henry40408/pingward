package db

import (
	"context"
	"fmt"
	"os"
	"sync"
	"testing"
)

// openTestPostgres returns a database emptied of every table, or skips without
// TEST_DATABASE_URL. Run these tests with -p 1: they share one schema.
func openTestPostgres(t *testing.T) *DB {
	t.Helper()
	url := os.Getenv("TEST_DATABASE_URL")
	if url == "" {
		t.Skip("TEST_DATABASE_URL is not set")
	}
	database, err := Open(url)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = database.Close() })
	if _, err := database.DB.ExecContext(context.Background(), "DROP SCHEMA public CASCADE; CREATE SCHEMA public"); err != nil {
		t.Fatal(err)
	}
	return database
}

func TestPostgresMigrationsMatchTheDerivedGolden(t *testing.T) {
	database := openTestPostgres(t)
	if err := database.Migrate(context.Background()); err != nil {
		t.Fatal(err)
	}
	got, want := readMigrationRows(t, database), readGolden(t, "sqlx_migrations.postgres.txt")
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Fatalf("got  %v\nwant %v", got, want)
	}
	if n := scanInt(t, database, "SELECT COUNT(*) FROM _sqlx_migrations WHERE execution_time <= 0"); n != 0 {
		t.Fatalf("%d rows keep the placeholder execution_time", n)
	}
	var columnType string
	query := "SELECT data_type FROM information_schema.columns WHERE table_name = '_sqlx_migrations' AND column_name = 'checksum'"
	if err := database.QueryRowContext(context.Background(), query).Scan(&columnType); err != nil || columnType != "bytea" {
		t.Fatalf("checksum column type %q, %v", columnType, err)
	}
	if err := database.Migrate(context.Background()); err != nil {
		t.Fatalf("second run: %v", err)
	}
}

func TestPostgresConcurrentMigratesSerialiseOnTheAdvisoryLock(t *testing.T) {
	database := openTestPostgres(t)
	const runners = 4
	errs := make([]error, runners)
	var wg sync.WaitGroup
	for i := range runners {
		wg.Add(1)
		go func() {
			defer wg.Done()
			errs[i] = database.Migrate(context.Background())
		}()
	}
	wg.Wait()
	for i, err := range errs {
		if err != nil {
			t.Errorf("runner %d: %v", i, err)
		}
	}
	if n := scanInt(t, database, "SELECT COUNT(*) FROM _sqlx_migrations"); n != 16 {
		t.Fatalf("%d rows", n)
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
