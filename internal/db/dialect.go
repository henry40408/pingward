// Package db opens SQLite and Postgres and applies the sqlx-compatible
// migrations that the Rust build also used, so a database written by one
// build can be opened by the other.
package db

import (
	"context"
	"database/sql"
)

// Dialect names the backend, for the few statements that differ.
type Dialect int

const (
	SQLite Dialect = iota
	Postgres
)

// DB is a connection pool that writes its SQL once, in Postgres placeholder
// style ($1, $2), and rewrites it for the backend. Use it instead of the
// embedded *sql.DB for every statement that takes arguments.
type DB struct {
	*sql.DB
	dialect Dialect
}

func (d *DB) Dialect() Dialect { return d.dialect }

func (d *DB) ExecContext(ctx context.Context, query string, args ...any) (sql.Result, error) {
	return d.DB.ExecContext(ctx, d.dialect.Rebind(query), args...)
}

func (d *DB) QueryContext(ctx context.Context, query string, args ...any) (*sql.Rows, error) {
	return d.DB.QueryContext(ctx, d.dialect.Rebind(query), args...)
}

func (d *DB) QueryRowContext(ctx context.Context, query string, args ...any) *sql.Row {
	return d.DB.QueryRowContext(ctx, d.dialect.Rebind(query), args...)
}

func (d *DB) BeginTx(ctx context.Context, opts *sql.TxOptions) (*Tx, error) {
	tx, err := d.DB.BeginTx(ctx, opts)
	if err != nil {
		return nil, err
	}
	return &Tx{Tx: tx, dialect: d.dialect}, nil
}

// Tx is a transaction with the same placeholder rewriting as DB.
type Tx struct {
	*sql.Tx
	dialect Dialect
}

func (t *Tx) ExecContext(ctx context.Context, query string, args ...any) (sql.Result, error) {
	return t.Tx.ExecContext(ctx, t.dialect.Rebind(query), args...)
}

func (t *Tx) QueryContext(ctx context.Context, query string, args ...any) (*sql.Rows, error) {
	return t.Tx.QueryContext(ctx, t.dialect.Rebind(query), args...)
}

func (t *Tx) QueryRowContext(ctx context.Context, query string, args ...any) *sql.Row {
	return t.Tx.QueryRowContext(ctx, t.dialect.Rebind(query), args...)
}
