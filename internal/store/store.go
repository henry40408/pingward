// Package store holds every SQL statement the app runs. The SQL is written
// once in Postgres placeholder style ($1) and rewritten for SQLite by the db
// package; timestamps are always bound as db.FormatTime text.
package store

import (
	"database/sql"
	"log/slog"
	"time"

	"github.com/henry40408/pingward/internal/db"
)

// Store is the data access layer over one database.
type Store struct {
	DB     *db.DB
	Logger *slog.Logger
}

func New(database *db.DB) *Store {
	return &Store{DB: database, Logger: slog.Default()}
}

// timeText is the stored form of an instant.
func timeText(t time.Time) string { return db.FormatTime(t) }

// optionalTimeText is the stored form of a nullable instant.
func optionalTimeText(t *time.Time) *string {
	if t == nil {
		return nil
	}
	text := timeText(*t)
	return &text
}

func boolToInt(value bool) int64 {
	if value {
		return 1
	}
	return 0
}

// parseStoredTime is lenient like the Rust build's parse_ts: text that is not
// RFC 3339 (a pre-0010 session's empty created_at) reads as "no value".
func parseStoredTime(text sql.NullString) *time.Time {
	if !text.Valid {
		return nil
	}
	parsed, err := time.Parse(time.RFC3339Nano, text.String)
	if err != nil {
		return nil
	}
	parsed = parsed.UTC()
	return &parsed
}
