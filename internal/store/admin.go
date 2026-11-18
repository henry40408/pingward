package store

import (
	"context"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

// CheckStatusCounts is the cross-user rollup of check statuses for the admin dashboard.
type CheckStatusCounts struct {
	New    int64
	Up     int64
	Down   int64
	Paused int64
	// Running counts stored up/new checks with an in-flight start (the
	// display-only Running status), a second query since SUM(CASE ...)'s type
	// differs across backends.
	Running int64
}

// DownCheck is a down check with its project name and owner.
type DownCheck struct {
	Check         models.Check
	ProjectName   string
	OwnerUsername string
}

// ChannelFailureCount is one channel's notification outcomes.
type ChannelFailureCount struct {
	ChannelName string
	OK          int64
	Errors      int64
}

func (s *Store) CountChecksByStatus(ctx context.Context) (CheckStatusCounts, error) {
	var counts CheckStatusCounts
	rows, err := s.DB.QueryContext(ctx, "SELECT status, COUNT(*) AS n FROM checks GROUP BY status")
	if err != nil {
		return counts, err
	}
	_, err = readRows(rows, func(r *row) (struct{}, error) {
		total := r.int64("n")
		switch models.CheckStatus(r.text("status")) {
		case models.StatusNew:
			counts.New = total
		case models.StatusUp:
			counts.Up = total
		case models.StatusDown:
			counts.Down = total
		case models.StatusPaused:
			counts.Paused = total
		}
		return struct{}{}, r.err
	})
	if err != nil {
		return counts, err
	}
	// Matches view.DisplayStatus: Running only applies on top of up/new.
	counts.Running, err = s.count(ctx,
		"SELECT COUNT(*) FROM checks "+
			"WHERE status IN ('up','new') AND last_start_at IS NOT NULL "+
			"AND (last_ping_at IS NULL OR last_start_at > last_ping_at)")
	return counts, err
}

// ListDownChecksWithOwner returns down checks with project name and owner,
// oldest last_ping_at first and never-pinged last (IS NULL sorts the same on
// both backends).
func (s *Store) ListDownChecksWithOwner(ctx context.Context) ([]DownCheck, error) {
	rows, err := s.DB.QueryContext(ctx,
		"SELECT c.*, p.name AS project_name, u.username AS owner_username "+
			"FROM checks c JOIN projects p ON p.id = c.project_id "+
			"JOIN users u ON u.id = p.user_id "+
			"WHERE c.status = 'down' ORDER BY c.last_ping_at IS NULL, c.last_ping_at, c.id")
	if err != nil {
		return nil, err
	}
	return readRows(rows, func(r *row) (DownCheck, error) {
		check, err := decodeCheck(r)
		return DownCheck{Check: check, ProjectName: r.text("project_name"), OwnerUsername: r.text("owner_username")}, err
	})
}

// NotificationCountsSince returns the ok and error notification counts across all users.
func (s *Store) NotificationCountsSince(ctx context.Context, cutoff time.Time) (ok, failed int64, err error) {
	if ok, err = s.count(ctx,
		"SELECT COUNT(*) FROM notifications WHERE status = 'ok' AND created_at >= $1", timeText(cutoff)); err != nil {
		return 0, 0, err
	}
	failed, err = s.count(ctx,
		"SELECT COUNT(*) FROM notifications WHERE status = 'error' AND created_at >= $1", timeText(cutoff))
	return ok, failed, err
}

// ChannelFailureCountsSince returns per-channel outcomes, most failures first.
// Cast to BIGINT: a bare SUM() may not decode as int64 on Postgres.
func (s *Store) ChannelFailureCountsSince(ctx context.Context, cutoff time.Time) ([]ChannelFailureCount, error) {
	rows, err := s.DB.QueryContext(ctx,
		"SELECT ch.name AS channel_name, "+
			"CAST(SUM(CASE WHEN n.status = 'ok' THEN 1 ELSE 0 END) AS BIGINT) AS ok, "+
			"CAST(SUM(CASE WHEN n.status = 'error' THEN 1 ELSE 0 END) AS BIGINT) AS err "+
			"FROM notifications n JOIN channels ch ON ch.id = n.channel_id "+
			"WHERE n.created_at >= $1 GROUP BY ch.id, ch.name ORDER BY err DESC, ch.id",
		timeText(cutoff))
	if err != nil {
		return nil, err
	}
	return readRows(rows, func(r *row) (ChannelFailureCount, error) {
		count := ChannelFailureCount{ChannelName: r.text("channel_name"), OK: r.int64("ok"), Errors: r.int64("err")}
		return count, r.err
	})
}

func (s *Store) RecentFailedNotifications(ctx context.Context, limit int64) ([]models.Notification, error) {
	rows, err := s.DB.QueryContext(ctx,
		"SELECT * FROM notifications WHERE status = 'error' ORDER BY id DESC LIMIT $1", limit)
	if err != nil {
		return nil, err
	}
	return readRows(rows, decodeNotification)
}

func (s *Store) CountProjects(ctx context.Context) (int64, error) {
	return s.count(ctx, "SELECT COUNT(*) FROM projects")
}

func (s *Store) CountChecks(ctx context.Context) (int64, error) {
	return s.count(ctx, "SELECT COUNT(*) FROM checks")
}

func (s *Store) CountPingsSince(ctx context.Context, cutoff time.Time) (int64, error) {
	return s.count(ctx, "SELECT COUNT(*) FROM pings WHERE created_at >= $1", timeText(cutoff))
}
