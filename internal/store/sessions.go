package store

import (
	"context"
	"time"

	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/models"
)

// lastSeenInterval keeps a busy session from costing a write per request.
const lastSeenInterval = 60 * time.Second

type NewSession struct {
	ID        string
	UserID    int64
	ExpiresAt time.Time
	UserAgent *string
	IP        *string
	SSO       bool
}

func (s *Store) CreateSession(ctx context.Context, session NewSession, now time.Time) error {
	_, err := s.DB.ExecContext(ctx,
		"INSERT INTO sessions (id, user_id, expires_at, created_at, user_agent, ip, sso) "+
			"VALUES ($1,$2,$3,$4,$5,$6,$7)",
		session.ID, session.UserID, timeText(session.ExpiresAt), timeText(now),
		session.UserAgent, session.IP, boolToInt(session.SSO))
	return err
}

// sessionLookup is the joined row FindSessionUser reads.
type sessionLookup struct {
	user      models.User
	lastSeen  *time.Time
	createdAt *time.Time
	expiresAt time.Time
	ip        *string
	userAgent *string
}

func decodeSessionLookup(r *row) (sessionLookup, error) {
	user, err := decodeUser(r)
	if err != nil {
		return sessionLookup{}, err
	}
	lookup := sessionLookup{
		user:      user,
		lastSeen:  r.optionalTime("session_last_seen_at"),
		createdAt: r.optionalTime("session_created_at"),
		expiresAt: r.requiredTime("sessions", "session_expires_at"),
		ip:        r.optionalText("session_ip"),
		userAgent: r.optionalText("session_user_agent"),
	}
	return lookup, r.err
}

// FindSessionUser returns the session's user (nil when none), honouring the
// idle window (expires_at, in SQL) and the absolute cap (created_at, in Go).
// last_seen_at is refreshed at most once per 60s; expires_at slides only past
// the idle window's half-life (auth.RefreshedExpiry).
func (s *Store) FindSessionUser(ctx context.Context, sessionID string, now time.Time) (*models.User, error) {
	rows, err := s.DB.QueryContext(ctx,
		"SELECT u.*, s.last_seen_at AS session_last_seen_at, "+
			"s.created_at AS session_created_at, s.expires_at AS session_expires_at, "+
			"s.ip AS session_ip, s.user_agent AS session_user_agent "+
			"FROM sessions s JOIN users u ON u.id = s.user_id "+
			"WHERE s.id = $1 AND s.expires_at > $2",
		sessionID, timeText(now))
	if err != nil {
		return nil, err
	}
	lookup, found, err := readOptionalRow(rows, decodeSessionLookup)
	if err != nil || !found {
		return nil, err
	}
	// Not in SQL: a pre-0010 row has created_at = '', which sorts below every
	// timestamp.
	if auth.IsPastAbsoluteCap(lookup.createdAt, now) {
		return nil, nil
	}
	if err := s.touchSession(ctx, sessionID, lookup, now); err != nil {
		return nil, err
	}
	return &lookup.user, nil
}

func (s *Store) touchSession(ctx context.Context, sessionID string, lookup sessionLookup, now time.Time) error {
	stale := lookup.lastSeen == nil || now.Sub(*lookup.lastSeen) >= lastSeenInterval
	renewal := auth.RefreshedExpiry(lookup.createdAt, lookup.expiresAt, now)
	if renewal != nil {
		_, err := s.DB.ExecContext(ctx, "UPDATE sessions SET last_seen_at = $1, expires_at = $2 WHERE id = $3",
			timeText(now), timeText(renewal.ExpiresAt), sessionID)
		if err != nil {
			return err
		}
		s.logRenewal(sessionID, lookup, *renewal)
		return nil
	}
	if !stale {
		return nil
	}
	_, err := s.DB.ExecContext(ctx, "UPDATE sessions SET last_seen_at = $1 WHERE id = $2", timeText(now), sessionID)
	return err
}

// logRenewal tells a clamp (a deployment signal) from routine activity.
func (s *Store) logRenewal(sessionID string, lookup sessionLookup, renewal auth.SessionRenewal) {
	s.Logger.Info("session.renewed",
		"target", "session",
		"handle", auth.SessionLogHandle(sessionID),
		"user_id", lookup.user.ID,
		"ip", derefText(lookup.ip),
		"user_agent", derefText(lookup.userAgent),
		"expires_at", timeText(renewal.ExpiresAt),
		"renewal", string(renewal.Kind))
}

func derefText(text *string) any {
	if text == nil {
		return nil
	}
	return *text
}

func (s *Store) DeleteSession(ctx context.Context, id string) error {
	_, err := s.DB.ExecContext(ctx, "DELETE FROM sessions WHERE id = $1", id)
	return err
}

// ListSessionsForUser lists unexpired sessions, newest-created first.
func (s *Store) ListSessionsForUser(ctx context.Context, userID int64, now time.Time) ([]models.Session, error) {
	rows, err := s.DB.QueryContext(ctx,
		"SELECT * FROM sessions WHERE user_id = $1 AND expires_at > $2 ORDER BY created_at DESC, id",
		userID, timeText(now))
	if err != nil {
		return nil, err
	}
	sessions, err := readRows(rows, decodeSession)
	if err != nil {
		return nil, err
	}
	// /account reaps capped rows first; this guards the other callers.
	live := sessions[:0]
	for _, session := range sessions {
		if !auth.IsPastAbsoluteCap(session.CreatedAt, now) {
			live = append(live, session)
		}
	}
	return live, nil
}

// DeleteCappedSessionsForUser deletes userID's sessions past the absolute cap,
// so /account does not wait for the next prune to stop hiding them.
// created_at <> ” spares pre-0010 rows, as in DeleteExpiredSessions.
func (s *Store) DeleteCappedSessionsForUser(ctx context.Context, userID int64, now time.Time) (int64, error) {
	return s.deleteCount(ctx,
		"DELETE FROM sessions WHERE user_id = $1 AND created_at <> '' AND created_at <= $2",
		userID, timeText(now.Add(-auth.SessionAbsoluteMax)))
}

// DeleteSessionOwned is owner-scoped, as DeleteAPIKey.
func (s *Store) DeleteSessionOwned(ctx context.Context, id string, userID int64) (bool, error) {
	deleted, err := s.deleteCount(ctx, "DELETE FROM sessions WHERE id = $1 AND user_id = $2", id, userID)
	return deleted > 0, err
}

// DeleteOtherSessionsForUser deletes every session for userID except keepID.
func (s *Store) DeleteOtherSessionsForUser(ctx context.Context, userID int64, keepID string) (int64, error) {
	return s.deleteCount(ctx, "DELETE FROM sessions WHERE user_id = $1 AND id <> $2", userID, keepID)
}

// DeleteSessionsForUser is for a password reset or a disabled account. When
// the operator is the target, use DeleteOtherSessionsForUser to keep their own.
func (s *Store) DeleteSessionsForUser(ctx context.Context, userID int64) (int64, error) {
	return s.deleteCount(ctx, "DELETE FROM sessions WHERE user_id = $1", userID)
}

// DeleteExpiredSessions deletes sessions past their idle window or absolute
// cap. They are already unusable, so this is not retention-driven.
func (s *Store) DeleteExpiredSessions(ctx context.Context, now time.Time) (int64, error) {
	// created_at <> '': a pre-0010 row is unaged, not infinitely old.
	return s.deleteCount(ctx,
		"DELETE FROM sessions WHERE expires_at <= $1 OR (created_at <> '' AND created_at <= $2)",
		timeText(now), timeText(now.Add(-auth.SessionAbsoluteMax)))
}

func (s *Store) deleteCount(ctx context.Context, query string, args ...any) (int64, error) {
	result, err := s.DB.ExecContext(ctx, query, args...)
	if err != nil {
		return 0, err
	}
	return result.RowsAffected()
}
