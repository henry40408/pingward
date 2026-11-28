package db

import (
	"cmp"
	"context"
	"database/sql"
	"fmt"
	"hash/crc32"
	"io/fs"
	"path"
	"regexp"
	"slices"
	"strconv"
)

// advisoryLockSalt is sqlx's constant. Releases up to 0.13 lock
// salt * crc32(database) while they migrate, so taking the same lock makes one
// of them that starts at the same time wait as well.
const advisoryLockSalt = 0x3d32ad9e

// Either of two rows in _sqlx_migrations shows that a database reached the
// baseline: the one releases up to 0.13 wrote for that version, or the one the
// baseline itself writes. The descriptions count, because the numbers alone would
// also match another program that uses sqlx. They prove version 16 only: a later
// baseline needs a proof of its own.
const (
	oldBaselineRecord       = "audit filter indexes"
	sentinelVersion   int64 = 99999999999999
	sentinelRecord          = "pingward no longer records migrations here"
)

// migrationFileName is `<version>_<description>.sql`.
var migrationFileName = regexp.MustCompile(`^(\d+)_.+\.sql$`)

// migration brings the schema to version.
type migration struct {
	version int64
	sql     string
}

// versionRange holds the first and the latest schema version this build has.
type versionRange struct {
	baseline, latest int64
}

func (r versionRange) refuseNewer(current int64) error {
	if current > r.latest {
		return fmt.Errorf("database schema version %d is newer than %d, the latest this pingward knows; run a newer pingward",
			current, r.latest)
	}
	return nil
}

// Migrate brings the database to the latest schema this build has.
func (d *DB) Migrate(ctx context.Context) error {
	return d.migrateFrom(ctx, migrationFiles)
}

func (d *DB) migrateFrom(ctx context.Context, files fs.FS) error {
	migrations, err := loadMigrations(files, d.dialect)
	if err != nil {
		return err
	}
	known := versionRange{baseline: migrations[0].version, latest: migrations[len(migrations)-1].version}
	current, err := d.schemaVersion(ctx, d.DB)
	if err != nil {
		return err
	}
	if err := known.refuseNewer(current); err != nil {
		return err
	}
	for _, m := range migrations {
		if m.version <= current {
			continue
		}
		if err := d.applyMigration(ctx, m, known); err != nil {
			return fmt.Errorf("migration %d: %w", m.version, err)
		}
	}
	return nil
}

// applyMigration runs the script and records its version in one transaction, so
// a failure leaves neither half behind.
func (d *DB) applyMigration(ctx context.Context, m migration, known versionRange) error {
	// On SQLite the DSN's _txlock=immediate takes the write lock here.
	tx, err := d.DB.BeginTx(ctx, d.migrationTxOptions())
	if err != nil {
		return err
	}
	defer func() { _ = tx.Rollback() }()
	current, err := d.lockedSchemaVersion(ctx, tx)
	if err != nil {
		return err
	}
	if err := known.refuseNewer(current); err != nil {
		return err
	}
	if current >= m.version {
		return nil
	}
	if m.version == known.baseline {
		err = d.applyBaseline(ctx, tx, m)
	} else {
		_, err = tx.ExecContext(ctx, m.sql)
	}
	if err != nil {
		return err
	}
	if err := d.recordSchemaVersion(ctx, tx, m.version); err != nil {
		return err
	}
	return tx.Commit()
}

// migrationTxOptions: under a stricter default isolation, Postgres takes the
// snapshot before the advisory lock, and the version read under the lock would
// miss what the previous holder committed.
func (d *DB) migrationTxOptions() *sql.TxOptions {
	if d.dialect != Postgres {
		return nil
	}
	return &sql.TxOptions{Isolation: sql.LevelReadCommitted}
}

// applyBaseline creates the schema, unless an older release or an earlier run
// already did: then only the sentinel may be missing. The script must not run on
// such a database, because Postgres demands ownership of a table even for CREATE
// INDEX IF NOT EXISTS.
func (d *DB) applyBaseline(ctx context.Context, tx *sql.Tx, m migration) error {
	exists, err := d.baselineExists(ctx, tx, m.version)
	if err != nil {
		return err
	}
	if exists {
		return d.writeSentinel(ctx, tx)
	}
	_, err = tx.ExecContext(ctx, m.sql)
	return err
}

// writeSentinel does what the end of the baseline script does.
func (d *DB) writeSentinel(ctx context.Context, tx *sql.Tx) error {
	_, err := tx.ExecContext(ctx, d.dialect.Rebind(`INSERT INTO _sqlx_migrations
		(version, description, success, checksum, execution_time) VALUES ($1, $2, TRUE, $3, 0)
		ON CONFLICT (version) DO NOTHING`), sentinelVersion, sentinelRecord, []byte{})
	return err
}

// lockedSchemaVersion reads the version again once no other process can change
// it: another one, maybe a newer pingward, may have got here first.
func (d *DB) lockedSchemaVersion(ctx context.Context, tx *sql.Tx) (int64, error) {
	if d.dialect == Postgres {
		if err := lockPostgres(ctx, tx); err != nil {
			return 0, err
		}
	}
	return d.schemaVersion(ctx, tx)
}

// lockPostgres serialises concurrent starts until tx ends.
func lockPostgres(ctx context.Context, tx *sql.Tx) error {
	var database string
	if err := tx.QueryRowContext(ctx, "SELECT current_database()").Scan(&database); err != nil {
		return err
	}
	_, err := tx.ExecContext(ctx, "SELECT pg_advisory_xact_lock($1)", advisoryLockID(database))
	return err
}

// advisoryLockID wraps on overflow, as sqlx's wrapping multiplication does.
func advisoryLockID(database string) int64 {
	return int64(advisoryLockSalt) * int64(crc32.ChecksumIEEE([]byte(database)))
}

// rowQuerier is what *sql.DB and *sql.Tx share to read one row.
type rowQuerier interface {
	QueryRowContext(ctx context.Context, query string, args ...any) *sql.Row
}

// schemaVersion is 0 for a database no migration has been recorded in. SQLite
// keeps the number in PRAGMA user_version, Postgres in a table whose name carries
// pingward's, because the database may be shared.
func (d *DB) schemaVersion(ctx context.Context, q rowQuerier) (int64, error) {
	query := "PRAGMA user_version"
	if d.dialect == Postgres {
		// pg_tables, not to_regclass: a lookup cached before another process
		// created the table could still answer that it is missing.
		var recorded bool
		err := q.QueryRowContext(ctx, `SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_tables
			WHERE tablename = 'pingward_schema_version' AND schemaname = current_schema())`).Scan(&recorded)
		if err != nil {
			return 0, fmt.Errorf("find schema version: %w", err)
		}
		if !recorded {
			return 0, nil
		}
		query = "SELECT coalesce(max(version), 0) FROM pingward_schema_version"
	}
	var version int64
	if err := q.QueryRowContext(ctx, query).Scan(&version); err != nil {
		return 0, fmt.Errorf("read schema version: %w", err)
	}
	return version, nil
}

func (d *DB) recordSchemaVersion(ctx context.Context, tx *sql.Tx, version int64) error {
	if d.dialect == SQLite {
		// A PRAGMA takes no bound parameters.
		_, err := tx.ExecContext(ctx, fmt.Sprintf("PRAGMA user_version = %d", version))
		return err
	}
	// The primary key and the check allow one row only.
	_, err := tx.ExecContext(ctx, `CREATE TABLE IF NOT EXISTS pingward_schema_version (
		singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
		version BIGINT NOT NULL
	)`)
	if err != nil {
		return err
	}
	_, err = tx.ExecContext(ctx, `INSERT INTO pingward_schema_version (version) VALUES ($1)
		ON CONFLICT (singleton) DO UPDATE SET version = EXCLUDED.version`, version)
	return err
}

// Each query counts the objects that rule out running the baseline blindly, and
// how many of them are _sqlx_migrations. A SQLite file belongs to pingward, so
// anything in it counts, except what SQLite and Litestream (which replicates the
// file) add themselves. A Postgres schema may be shared, so only the names
// pingward uses count.
const (
	sqliteInventory = `SELECT count(*), count(*) FILTER (WHERE name = '_sqlx_migrations')
		FROM sqlite_master
		WHERE name NOT LIKE 'sqlite\_%' ESCAPE '\' AND name NOT LIKE '\_litestream\_%' ESCAPE '\'`
	postgresInventory = `SELECT count(*), count(*) FILTER (WHERE c.relname = '_sqlx_migrations')
		FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
		WHERE n.nspname = current_schema() AND c.relname IN (
			'_sqlx_migrations', 'api_keys', 'audit_log', 'channels', 'check_channels', 'checks',
			'notifications', 'pings', 'projects', 'sessions', 'settings', 'users')`
)

// baselineExists is false for a database with nothing of its own in it, and true
// for one that reached the baseline before: through the migrations of an older
// release, or through the baseline itself if the version was lost since (a
// restored SQLite dump). Anything else is refused: the baseline only creates what
// is missing, so on an older schema it would leave old tables as they are.
func (d *DB) baselineExists(ctx context.Context, q rowQuerier, baseline int64) (bool, error) {
	inventory := sqliteInventory
	if d.dialect == Postgres {
		inventory = postgresInventory
	}
	var objects, records int
	if err := q.QueryRowContext(ctx, inventory).Scan(&objects, &records); err != nil {
		return false, fmt.Errorf("inspect database: %w", err)
	}
	if objects == 0 {
		return false, nil
	}
	if records > 0 {
		reached, err := d.recordsReach(ctx, q, baseline)
		if err != nil || reached {
			return reached, err
		}
	}
	return false, fmt.Errorf("the database is not empty and has no complete record of schema version %d; "+
		"if it is from an older pingward, start pingward 0.13 on it once, then upgrade", baseline)
}

func (d *DB) recordsReach(ctx context.Context, q rowQuerier, baseline int64) (bool, error) {
	var reached bool
	err := q.QueryRowContext(ctx, d.dialect.Rebind(`SELECT
		EXISTS (SELECT 1 FROM _sqlx_migrations
			WHERE (version = $1 AND description = $2) OR (version = $3 AND description = $4))
		AND NOT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE success = FALSE)`),
		baseline, oldBaselineRecord, sentinelVersion, sentinelRecord).Scan(&reached)
	if err != nil {
		return false, fmt.Errorf("read old migration records: %w", err)
	}
	return reached, nil
}

// loadMigrations reads the dialect's scripts, ordered by version. The first one
// is the baseline: it creates the whole schema of its version.
func loadMigrations(files fs.FS, dialect Dialect) ([]migration, error) {
	dir := "migrations/sqlite"
	if dialect == Postgres {
		dir = "migrations/postgres"
	}
	names, err := fs.Glob(files, dir+"/*.sql")
	if err != nil {
		return nil, err
	}
	if len(names) == 0 {
		return nil, fmt.Errorf("no migration files in %s", dir)
	}
	migrations := make([]migration, 0, len(names))
	for _, name := range names {
		m, err := readMigration(files, name)
		if err != nil {
			return nil, err
		}
		migrations = append(migrations, m)
	}
	slices.SortFunc(migrations, func(a, b migration) int { return cmp.Compare(a.version, b.version) })
	for i := 1; i < len(migrations); i++ {
		if migrations[i].version == migrations[i-1].version {
			return nil, fmt.Errorf("migrations share version %d", migrations[i].version)
		}
	}
	return migrations, nil
}

func readMigration(files fs.FS, name string) (migration, error) {
	parts := migrationFileName.FindStringSubmatch(path.Base(name))
	if parts == nil {
		return migration{}, fmt.Errorf("migration file %q is not named <version>_<description>.sql", name)
	}
	// user_version is a signed 32-bit integer, and 0 means no migration yet.
	version, err := strconv.ParseInt(parts[1], 10, 32)
	if err != nil || version == 0 {
		return migration{}, fmt.Errorf("migration file %q: the version must be from 1 to 2147483647", name)
	}
	content, err := fs.ReadFile(files, name)
	if err != nil {
		return migration{}, err
	}
	return migration{version: version, sql: string(content)}, nil
}
