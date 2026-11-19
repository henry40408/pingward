package db

import (
	"bytes"
	"context"
	"crypto/sha512"
	"database/sql"
	"errors"
	"fmt"
	"hash/crc32"
	"io/fs"
	"path"
	"sort"
	"strconv"
	"strings"
	"time"
)

// The migration bookkeeping mirrors sqlx-core (migrate/migrator.rs and the
// sqlite and postgres migrate.rs), so a Rust build can open a database this
// code migrated and the other way round.

const createMigrationsTableSQLite = `CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
);`

const createMigrationsTablePostgres = `CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
    success BOOLEAN NOT NULL,
    checksum BYTEA NOT NULL,
    execution_time BIGINT NOT NULL
);`

// advisoryLockSalt is sqlx's constant; the lock id is salt * crc32(database).
const advisoryLockSalt = 0x3d32ad9e

// DirtyError means a migration started but never finished.
type DirtyError struct{ Version int64 }

func (e DirtyError) Error() string {
	return fmt.Sprintf("migration %d is partially applied; fix and remove row from `_sqlx_migrations` table", e.Version)
}

// VersionMissingError means the database has a migration this build lacks.
type VersionMissingError struct{ Version int64 }

func (e VersionMissingError) Error() string {
	return fmt.Sprintf("migration %d was previously applied but is missing in the resolved migrations", e.Version)
}

// VersionMismatchError means an applied migration's file has changed.
type VersionMismatchError struct{ Version int64 }

func (e VersionMismatchError) Error() string {
	return fmt.Sprintf("migration %d was previously applied but has been modified", e.Version)
}

type migration struct {
	version     int64
	description string
	sql         string
	checksum    []byte
}

// Migrate applies every pending migration for the database's dialect.
func (d *DB) Migrate(ctx context.Context) error {
	return d.migrateFrom(ctx, migrationFiles)
}

func (d *DB) migrateFrom(ctx context.Context, files fs.FS) error {
	migrations, err := loadMigrations(files, d.dialect)
	if err != nil {
		return err
	}
	// One connection for the whole run: the Postgres advisory lock belongs to
	// the session.
	conn, err := d.Conn(ctx)
	if err != nil {
		return err
	}
	defer func() { _ = conn.Close() }()

	// Lock before creating the table: two replicas starting on an empty
	// database would otherwise both run CREATE TABLE and collide.
	if d.dialect == Postgres {
		unlock, err := lockPostgres(ctx, conn)
		if err != nil {
			return err
		}
		defer unlock()
	}
	if _, err := conn.ExecContext(ctx, createMigrationsTable(d.dialect)); err != nil {
		return err
	}
	return applyPending(ctx, applier{conn: conn, dialect: d.dialect}, migrations)
}

func createMigrationsTable(dialect Dialect) string {
	if dialect == Postgres {
		return createMigrationsTablePostgres
	}
	return createMigrationsTableSQLite
}

// lockPostgres serialises concurrent starts. The returned function releases
// the lock with a fresh context: the caller's may already be cancelled.
func lockPostgres(ctx context.Context, conn *sql.Conn) (unlock func(), err error) {
	var database string
	if err := conn.QueryRowContext(ctx, "SELECT current_database()").Scan(&database); err != nil {
		return nil, err
	}
	lockID := advisoryLockID(database)
	if _, err := conn.ExecContext(ctx, "SELECT pg_advisory_lock($1)", lockID); err != nil {
		return nil, err
	}
	return func() {
		releaseCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_, _ = conn.ExecContext(releaseCtx, "SELECT pg_advisory_unlock($1)", lockID)
	}, nil
}

// advisoryLockID wraps on overflow, as sqlx's wrapping multiplication does.
func advisoryLockID(database string) int64 {
	return int64(advisoryLockSalt) * int64(crc32.ChecksumIEEE([]byte(database)))
}

func loadMigrations(files fs.FS, dialect Dialect) ([]migration, error) {
	dir := "migrations/sqlite"
	if dialect == Postgres {
		dir = "migrations/postgres"
	}
	entries, err := fs.ReadDir(files, dir)
	if err != nil {
		return nil, err
	}
	var migrations []migration
	for _, entry := range entries {
		if entry.IsDir() || !strings.HasSuffix(entry.Name(), ".sql") {
			continue
		}
		content, err := fs.ReadFile(files, path.Join(dir, entry.Name()))
		if err != nil {
			return nil, err
		}
		parsed, err := parseMigration(entry.Name(), content)
		if err != nil {
			return nil, err
		}
		migrations = append(migrations, parsed)
	}
	sort.Slice(migrations, func(i, j int) bool { return migrations[i].version < migrations[j].version })
	return migrations, nil
}

// parseMigration reads "<version>_<description>.sql": the description is the
// text after the first "_" with "_" turned into a space, and the checksum is
// the SHA-384 of the file bytes.
func parseMigration(name string, content []byte) (migration, error) {
	stem := strings.TrimSuffix(name, ".sql")
	versionText, description, found := strings.Cut(stem, "_")
	if !found {
		return migration{}, fmt.Errorf("migration file %q is not named <version>_<description>.sql", name)
	}
	version, err := strconv.ParseInt(versionText, 10, 64)
	if err != nil {
		return migration{}, fmt.Errorf("migration file %q has a non-numeric version: %w", name, err)
	}
	if bytes.Contains(content, []byte("-- no-transaction")) {
		return migration{}, fmt.Errorf("migration file %q uses -- no-transaction, which is not supported", name)
	}
	sum := sha512.Sum384(content)
	return migration{
		version:     version,
		description: strings.ReplaceAll(description, "_", " "),
		sql:         string(content),
		checksum:    sum[:],
	}, nil
}

// applier runs the bookkeeping statements on one pinned connection.
type applier struct {
	conn    *sql.Conn
	dialect Dialect
}

func applyPending(ctx context.Context, run applier, migrations []migration) error {
	if dirty, found, err := run.dirtyVersion(ctx); err != nil {
		return err
	} else if found {
		return DirtyError{Version: dirty}
	}
	applied, err := run.appliedChecksums(ctx)
	if err != nil {
		return err
	}
	if err := checkNoneMissing(applied, migrations); err != nil {
		return err
	}
	for _, m := range migrations {
		checksum, done := applied[m.version]
		if done {
			if !bytes.Equal(checksum, m.checksum) {
				return VersionMismatchError{Version: m.version}
			}
			continue
		}
		if err := run.apply(ctx, m); err != nil {
			return fmt.Errorf("migration %d: %w", m.version, err)
		}
	}
	return nil
}

func checkNoneMissing(applied map[int64][]byte, migrations []migration) error {
	known := make(map[int64]bool, len(migrations))
	for _, m := range migrations {
		known[m.version] = true
	}
	versions := make([]int64, 0, len(applied))
	for version := range applied {
		versions = append(versions, version)
	}
	sort.Slice(versions, func(i, j int) bool { return versions[i] < versions[j] })
	for _, version := range versions {
		if !known[version] {
			return VersionMissingError{Version: version}
		}
	}
	return nil
}

func (a applier) dirtyVersion(ctx context.Context) (version int64, found bool, err error) {
	err = a.conn.QueryRowContext(ctx,
		"SELECT version FROM _sqlx_migrations WHERE success = false ORDER BY version LIMIT 1").Scan(&version)
	if errors.Is(err, sql.ErrNoRows) {
		return 0, false, nil
	}
	return version, err == nil, err
}

func (a applier) appliedChecksums(ctx context.Context) (map[int64][]byte, error) {
	rows, err := a.conn.QueryContext(ctx, "SELECT version, checksum FROM _sqlx_migrations ORDER BY version")
	if err != nil {
		return nil, err
	}
	defer func() { _ = rows.Close() }()
	applied := make(map[int64][]byte)
	for rows.Next() {
		var version int64
		var checksum []byte
		if err := rows.Scan(&version, &checksum); err != nil {
			return nil, err
		}
		applied[version] = checksum
	}
	return applied, rows.Err()
}

// apply runs one migration in its own transaction: the SQL, then the
// bookkeeping row with execution_time -1. After the commit the real duration
// in nanoseconds replaces the -1, exactly as sqlx does.
func (a applier) apply(ctx context.Context, m migration) error {
	start := time.Now()
	tx, err := a.conn.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	if err := a.runInTx(ctx, tx, m); err != nil {
		_ = tx.Rollback()
		return err
	}
	if err := tx.Commit(); err != nil {
		return err
	}
	_, err = a.conn.ExecContext(ctx,
		a.dialect.Rebind("UPDATE _sqlx_migrations SET execution_time = $1 WHERE version = $2"),
		time.Since(start).Nanoseconds(), m.version)
	return err
}

func (a applier) runInTx(ctx context.Context, tx *sql.Tx, m migration) error {
	if _, err := tx.ExecContext(ctx, m.sql); err != nil {
		return err
	}
	_, err := tx.ExecContext(ctx, a.dialect.Rebind(
		"INSERT INTO _sqlx_migrations ( version, description, success, checksum, execution_time ) VALUES ( $1, $2, TRUE, $3, -1 )"),
		m.version, m.description, m.checksum)
	return err
}
