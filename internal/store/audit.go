package store

import (
	"context"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

type NewAudit struct {
	ActorUserID   int64
	ActorUsername string
	Action        string
	TargetType    *string
	TargetID      *int64
	TargetOwnerID *int64
	Method        *string
	Path          *string
	Detail        *string
}

func (s *Store) RecordAudit(ctx context.Context, entry NewAudit, now time.Time) (int64, error) {
	var id int64
	err := s.DB.QueryRowContext(ctx,
		"INSERT INTO audit_log "+
			"(actor_user_id, actor_username, action, target_type, target_id, "+
			"target_owner_id, method, path, detail, created_at) "+
			"VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING id",
		entry.ActorUserID, entry.ActorUsername, entry.Action, entry.TargetType, entry.TargetID,
		entry.TargetOwnerID, entry.Method, entry.Path, entry.Detail, timeText(now)).Scan(&id)
	return id, err
}

// ListAudit returns the newest limit rows, unfiltered; for tests. /admin uses
// the paged, filtered listing.
func (s *Store) ListAudit(ctx context.Context, limit int64) ([]models.AuditLog, error) {
	rows, err := s.DB.QueryContext(ctx, "SELECT * FROM audit_log ORDER BY id DESC LIMIT $1", limit)
	if err != nil {
		return nil, err
	}
	return readRows(rows, decodeAudit)
}

// AuditFilterOptions returns the distinct actors and actions for the audit
// filter's selects, read from the data so a new RecordAudit call site appears
// by itself.
func (s *Store) AuditFilterOptions(ctx context.Context) (actors, actions []string, err error) {
	if actors, err = s.distinctColumn(ctx, "SELECT DISTINCT actor_username FROM audit_log ORDER BY 1"); err != nil {
		return nil, nil, err
	}
	if actions, err = s.distinctColumn(ctx, "SELECT DISTINCT action FROM audit_log ORDER BY 1"); err != nil {
		return nil, nil, err
	}
	return actors, actions, nil
}

func (s *Store) distinctColumn(ctx context.Context, query string) ([]string, error) {
	rows, err := s.DB.QueryContext(ctx, query)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var values []string
	for rows.Next() {
		var value string
		if err := rows.Scan(&value); err != nil {
			return nil, err
		}
		values = append(values, value)
	}
	return values, rows.Err()
}
