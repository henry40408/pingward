package db

import (
	"errors"

	"github.com/jackc/pgx/v5/pgconn"
	"modernc.org/sqlite"
)

const (
	postgresUniqueViolation = "23505"
	// SQLITE_CONSTRAINT_UNIQUE and SQLITE_CONSTRAINT_PRIMARYKEY.
	sqliteConstraintUnique     = 2067
	sqliteConstraintPrimaryKey = 1555
)

// IsUniqueViolation reports whether err is a UNIQUE or PRIMARY KEY violation
// on either backend.
func IsUniqueViolation(err error) bool {
	var pgErr *pgconn.PgError
	if errors.As(err, &pgErr) {
		return pgErr.Code == postgresUniqueViolation
	}
	var sqliteErr *sqlite.Error
	if errors.As(err, &sqliteErr) {
		code := sqliteErr.Code()
		return code == sqliteConstraintUnique || code == sqliteConstraintPrimaryKey
	}
	return false
}
