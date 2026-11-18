package store

import (
	"context"
	"database/sql"
	"errors"
)

// GetSetting returns nil when the key is not set.
func (s *Store) GetSetting(ctx context.Context, key string) (*string, error) {
	var value string
	err := s.DB.QueryRowContext(ctx, "SELECT value FROM settings WHERE key = $1", key).Scan(&value)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &value, nil
}

func (s *Store) SetSetting(ctx context.Context, key, value string) error {
	_, err := s.DB.ExecContext(ctx,
		"INSERT INTO settings (key, value) VALUES ($1,$2) "+
			"ON CONFLICT(key) DO UPDATE SET value = excluded.value", key, value)
	return err
}
