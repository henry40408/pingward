package db

import (
	"database/sql"
	"fmt"
	"strings"

	_ "github.com/jackc/pgx/v5/stdlib" // registers the "pgx" driver
	_ "modernc.org/sqlite"             // registers the "sqlite" driver
)

const (
	filePoolSize     = 5
	busyTimeoutMilli = 5000
)

// IsSQLiteURL reports whether url selects SQLite ("sqlite:..."); anything else
// is handed to Postgres.
func IsSQLiteURL(url string) bool {
	return strings.HasPrefix(url, "sqlite:")
}

// isInMemoryURL: an in-memory database lives on a single physical connection.
func isInMemoryURL(url string) bool {
	return strings.Contains(url, ":memory:") || strings.Contains(url, "mode=memory")
}

// Open connects to url and applies the per-connection settings. It does not
// run migrations.
func Open(url string) (*DB, error) {
	if !IsSQLiteURL(url) {
		return openPostgres(url)
	}
	return openSQLite(url)
}

func openPostgres(url string) (*DB, error) {
	conn, err := sql.Open("pgx", url)
	if err != nil {
		return nil, err
	}
	conn.SetMaxOpenConns(filePoolSize)
	return &DB{DB: conn, dialect: Postgres}, nil
}

func openSQLite(url string) (*DB, error) {
	conn, err := sql.Open("sqlite", sqliteDSN(url))
	if err != nil {
		return nil, err
	}
	if isInMemoryURL(url) {
		// A second connection would see a different, empty database, and a
		// recycled one would lose it.
		conn.SetMaxOpenConns(1)
		conn.SetConnMaxLifetime(0)
	} else {
		conn.SetMaxOpenConns(filePoolSize)
	}
	// sql.Open is lazy: surface a bad path or pragma now.
	if err := conn.Ping(); err != nil {
		_ = conn.Close()
		return nil, err
	}
	return &DB{DB: conn, dialect: SQLite}, nil
}

// sqliteDSN builds the modernc DSN. foreign_keys is needed or ON DELETE
// CASCADE is silently unenforced; busy_timeout retries a blocked write instead
// of failing with SQLITE_BUSY. A file database also gets WAL so readers run
// alongside a writer (NORMAL is safe under WAL); an in-memory database's
// journal mode is always "memory".
func sqliteDSN(url string) string {
	pragmas := []string{
		"foreign_keys(1)",
		fmt.Sprintf("busy_timeout(%d)", busyTimeoutMilli),
	}
	if isInMemoryURL(url) {
		return "file::memory:?" + pragmaQuery(pragmas)
	}
	pragmas = append(pragmas, "journal_mode(WAL)", "synchronous(NORMAL)")

	path, query := splitQuery(strings.TrimPrefix(strings.TrimPrefix(url, "sqlite:"), "//"))
	// sqlx's Any driver has no create_if_missing; mode=rwc creates the file.
	if !strings.Contains(query, "mode=") {
		query = joinQuery(query, "mode=rwc")
	}
	return "file:" + escapeURIPath(path) + "?" + joinQuery(query, pragmaQuery(pragmas))
}

func pragmaQuery(pragmas []string) string {
	parts := make([]string, len(pragmas))
	for i, pragma := range pragmas {
		parts[i] = "_pragma=" + pragma
	}
	return strings.Join(parts, "&")
}

func splitQuery(target string) (path, query string) {
	path, query, _ = strings.Cut(target, "?")
	return path, query
}

func joinQuery(existing, extra string) string {
	if existing == "" {
		return extra
	}
	return existing + "&" + extra
}

// escapeURIPath keeps characters that SQLite's URI parser treats as syntax
// from being read as such.
func escapeURIPath(path string) string {
	return strings.NewReplacer("%", "%25", "?", "%3f", "#", "%23").Replace(path)
}
