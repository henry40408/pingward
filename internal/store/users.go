package store

import (
	"context"
	"database/sql"
	"errors"
	"time"

	"github.com/henry40408/pingward/internal/db"
	"github.com/henry40408/pingward/internal/models"
)

// ErrUsernameTaken is a duplicate username: an ordinary form outcome, not a
// server error.
var ErrUsernameTaken = errors.New("that username is already taken")

// firstUserLockKey is "pingward" as a big-endian int64.
const firstUserLockKey int64 = 0x7069_6e67_7761_7264

// classifyCreateUser maps a unique violation to ErrUsernameTaken. Any unique
// violation is the username: it is the only unique constraint on users. A
// second one would need distinguishing here.
func classifyCreateUser(err error) error {
	if db.IsUniqueViolation(err) {
		return ErrUsernameTaken
	}
	return err
}

func (s *Store) CountUsers(ctx context.Context) (int64, error) {
	return s.count(ctx, "SELECT COUNT(*) FROM users")
}

func (s *Store) count(ctx context.Context, query string, args ...any) (int64, error) {
	var total int64
	err := s.DB.QueryRowContext(ctx, query, args...).Scan(&total)
	return total, err
}

func (s *Store) CreateUser(ctx context.Context, username string, passwordHash *string, isAdmin bool, now time.Time) (int64, error) {
	var id int64
	err := s.DB.QueryRowContext(ctx,
		"INSERT INTO users (username, password_hash, is_admin, created_at) VALUES ($1,$2,$3,$4) RETURNING id",
		username, passwordHash, boolToInt(isAdmin), timeText(now)).Scan(&id)
	if err != nil {
		return 0, classifyCreateUser(err)
	}
	return id, nil
}

// insertFirstUser runs an "insert if users is empty" statement. Postgres runs
// READ COMMITTED, so two concurrent INSERT ... WHERE NOT EXISTS statements
// both see an empty table; a transaction-scoped advisory lock makes the second
// wait and then see the first's row. SQLite needs none: one statement is
// atomic and writers are serialised. found is false when the statement
// inserted nothing.
func (s *Store) insertFirstUser(ctx context.Context, query string, args ...any) (id int64, found bool, err error) {
	if s.DB.Dialect() != db.Postgres {
		return scanOptionalID(s.DB.QueryRowContext(ctx, query, args...))
	}
	tx, err := s.DB.BeginTx(ctx, nil)
	if err != nil {
		return 0, false, err
	}
	defer tx.Rollback() //nolint:errcheck // a no-op after Commit
	if _, err := tx.ExecContext(ctx, "SELECT pg_advisory_xact_lock($1)", firstUserLockKey); err != nil {
		return 0, false, err
	}
	id, found, err = scanOptionalID(tx.QueryRowContext(ctx, query, args...))
	if err != nil {
		return 0, false, err
	}
	return id, found, tx.Commit()
}

func scanOptionalID(row *sql.Row) (id int64, found bool, err error) {
	err = row.Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return 0, false, nil
	}
	return id, err == nil, err
}

// CreateFirstAdmin inserts the first admin only while users is empty, in one
// statement so two racing /setup submissions cannot both pass a separate count
// check. found is false when another user already exists (someone else won, or
// setup was already done).
func (s *Store) CreateFirstAdmin(ctx context.Context, username, passwordHash string, now time.Time) (id int64, found bool, err error) {
	return s.insertFirstUser(ctx,
		"INSERT INTO users (username, password_hash, is_admin, created_at) "+
			"SELECT $1, $2, 1, $3 WHERE NOT EXISTS (SELECT 1 FROM users) RETURNING id",
		username, passwordHash, timeText(now))
}

// CreatePasswordlessUserFirstIsAdmin provisions a passwordless account, admin
// only if users is empty. The decision is inside the INSERT, so simultaneous
// first sign-ins cannot each conclude they were first.
func (s *Store) CreatePasswordlessUserFirstIsAdmin(ctx context.Context, username string, now time.Time) (int64, error) {
	id, found, err := s.insertFirstUser(ctx,
		"INSERT INTO users (username, password_hash, is_admin, created_at) "+
			"SELECT $1, NULL, CASE WHEN EXISTS (SELECT 1 FROM users) THEN 0 ELSE 1 END, $2 "+
			"RETURNING id",
		username, timeText(now))
	if err != nil {
		return 0, classifyCreateUser(err)
	}
	if !found {
		return 0, sql.ErrNoRows
	}
	return id, nil
}

func (s *Store) FindUserByUsername(ctx context.Context, username string) (*models.User, error) {
	return s.findUser(ctx, "SELECT * FROM users WHERE username = $1", username)
}

func (s *Store) FindUserByID(ctx context.Context, id int64) (*models.User, error) {
	return s.findUser(ctx, "SELECT * FROM users WHERE id = $1", id)
}

// findUser returns nil when no row matches.
func (s *Store) findUser(ctx context.Context, query string, arg any) (*models.User, error) {
	rows, err := s.DB.QueryContext(ctx, query, arg)
	if err != nil {
		return nil, err
	}
	user, found, err := readOptionalRow(rows, decodeUser)
	if err != nil || !found {
		return nil, err
	}
	return &user, nil
}

func (s *Store) ListUsers(ctx context.Context) ([]models.User, error) {
	rows, err := s.DB.QueryContext(ctx, "SELECT * FROM users ORDER BY id")
	if err != nil {
		return nil, err
	}
	return readRows(rows, decodeUser)
}

func (s *Store) DeleteUser(ctx context.Context, id int64) error {
	_, err := s.DB.ExecContext(ctx, "DELETE FROM users WHERE id = $1", id)
	return err
}

func (s *Store) SetUserDisabled(ctx context.Context, id int64, disabled bool) error {
	_, err := s.DB.ExecContext(ctx, "UPDATE users SET disabled = $1 WHERE id = $2", boolToInt(disabled), id)
	return err
}

func (s *Store) SetUserPassword(ctx context.Context, id int64, passwordHash string) error {
	_, err := s.DB.ExecContext(ctx, "UPDATE users SET password_hash = $1 WHERE id = $2", passwordHash, id)
	return err
}

func (s *Store) SetUserAdmin(ctx context.Context, id int64, isAdmin bool) error {
	_, err := s.DB.ExecContext(ctx, "UPDATE users SET is_admin = $1 WHERE id = $2", boolToInt(isAdmin), id)
	return err
}

func (s *Store) CountEnabledAdmins(ctx context.Context) (int64, error) {
	return s.count(ctx, "SELECT COUNT(*) FROM users WHERE is_admin <> 0 AND disabled = 0")
}
