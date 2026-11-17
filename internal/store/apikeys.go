package store

import (
	"context"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

// apiKeyTouchInterval keeps a hot key from costing a write per request.
const apiKeyTouchInterval = 60 * time.Second

// NewAPIKey holds what is stored for a key: only the hash and the non-secret
// prefix, never the token.
type NewAPIKey struct {
	UserID    int64
	Name      string
	TokenHash string
	Prefix    string
	ExpiresAt *time.Time
}

func (s *Store) InsertAPIKey(ctx context.Context, key NewAPIKey, now time.Time) (int64, error) {
	var id int64
	err := s.DB.QueryRowContext(ctx,
		"INSERT INTO api_keys (user_id, name, token_hash, prefix, created_at, expires_at) "+
			"VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
		key.UserID, key.Name, key.TokenHash, key.Prefix, timeText(now), optionalTimeText(key.ExpiresAt)).Scan(&id)
	return id, err
}

// ListAPIKeysForUser lists newest first.
func (s *Store) ListAPIKeysForUser(ctx context.Context, userID int64) ([]models.ApiKey, error) {
	rows, err := s.DB.QueryContext(ctx, "SELECT * FROM api_keys WHERE user_id = $1 ORDER BY id DESC", userID)
	if err != nil {
		return nil, err
	}
	return readRows(rows, decodeAPIKey)
}

// DeleteAPIKey is owner-scoped. It reports false for a missing key and a
// foreign one alike, so existence is not disclosed.
func (s *Store) DeleteAPIKey(ctx context.Context, id, userID int64) (bool, error) {
	result, err := s.DB.ExecContext(ctx, "DELETE FROM api_keys WHERE id = $1 AND user_id = $2", id, userID)
	if err != nil {
		return false, err
	}
	affected, err := result.RowsAffected()
	return affected > 0, err
}

// ValidateAPIKey returns the owning user id of an unexpired key, or found=false.
// last_used_at is refreshed at most once per 60s.
func (s *Store) ValidateAPIKey(ctx context.Context, tokenHash string, now time.Time) (userID int64, found bool, err error) {
	rows, err := s.DB.QueryContext(ctx,
		"SELECT id, user_id, expires_at, last_used_at FROM api_keys WHERE token_hash = $1", tokenHash)
	if err != nil {
		return 0, false, err
	}
	key, found, err := readOptionalRow(rows, decodeKeyUsage)
	if err != nil || !found {
		return 0, false, err
	}
	if key.expiresAt != nil && !key.expiresAt.After(now) {
		return 0, false, nil
	}
	if key.lastUsedAt == nil || now.Sub(*key.lastUsedAt) >= apiKeyTouchInterval {
		_, err := s.DB.ExecContext(ctx, "UPDATE api_keys SET last_used_at = $1 WHERE id = $2", timeText(now), key.id)
		if err != nil {
			return 0, false, err
		}
	}
	return key.userID, true, nil
}

type keyUsage struct {
	id, userID            int64
	expiresAt, lastUsedAt *time.Time
}

func decodeKeyUsage(r *row) (keyUsage, error) {
	usage := keyUsage{
		id:         r.int64("id"),
		userID:     r.int64("user_id"),
		expiresAt:  r.optionalTime("expires_at"),
		lastUsedAt: r.optionalTime("last_used_at"),
	}
	return usage, r.err
}
