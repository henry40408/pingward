// Package testutil builds throwaway databases for tests.
package testutil

import (
	"context"
	"os"
	"path/filepath"
	"testing"

	"github.com/henry40408/pingward/internal/db"
	"github.com/henry40408/pingward/internal/store"
)

// NewStore returns a migrated Store on a fresh SQLite file in t.TempDir().
func NewStore(t *testing.T) *store.Store {
	t.Helper()
	return openStore(t, "sqlite://"+filepath.Join(t.TempDir(), "test.db"))
}

// NewPostgresStore returns a migrated Store on an emptied Postgres database,
// or skips without TEST_DATABASE_URL. Run with -p 1: tests share one schema.
func NewPostgresStore(t *testing.T) *store.Store {
	t.Helper()
	url := os.Getenv("TEST_DATABASE_URL")
	if url == "" {
		t.Skip("TEST_DATABASE_URL is not set")
	}
	database, err := db.Open(url)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.DB.ExecContext(context.Background(), "DROP SCHEMA public CASCADE; CREATE SCHEMA public"); err != nil {
		t.Fatal(err)
	}
	_ = database.Close()
	return openStore(t, url)
}

func openStore(t *testing.T, url string) *store.Store {
	t.Helper()
	database, err := db.Open(url)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = database.Close() })
	if err := database.Migrate(context.Background()); err != nil {
		t.Fatal(err)
	}
	return store.New(database)
}
