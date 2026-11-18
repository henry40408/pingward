package store

import (
	"context"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

type NewPing struct {
	CheckID  int64
	Kind     models.PingKind
	ExitCode *int64
	Body     string
	SourceIP *string
}

func (s *Store) InsertPing(ctx context.Context, ping NewPing, now time.Time) error {
	_, err := s.DB.ExecContext(ctx,
		"INSERT INTO pings (check_id, kind, exit_code, body, source_ip, created_at) VALUES ($1,$2,$3,$4,$5,$6)",
		ping.CheckID, string(ping.Kind), ping.ExitCode, ping.Body, ping.SourceIP, timeText(now))
	return err
}

func (s *Store) ListRecentPings(ctx context.Context, checkID, limit int64) ([]models.Ping, error) {
	rows, err := s.DB.QueryContext(ctx,
		"SELECT * FROM pings WHERE check_id = $1 ORDER BY id DESC LIMIT $2", checkID, limit)
	if err != nil {
		return nil, err
	}
	return readRows(rows, decodePing)
}

// ListRecentPingSummaries returns the newest limit pings without their body.
func (s *Store) ListRecentPingSummaries(ctx context.Context, checkID, limit int64) ([]models.PingSummary, error) {
	rows, err := s.DB.QueryContext(ctx,
		"SELECT id, check_id, kind, created_at FROM pings "+
			"WHERE check_id = $1 ORDER BY id DESC LIMIT $2", checkID, limit)
	if err != nil {
		return nil, err
	}
	return readRows(rows, decodePingSummary)
}

// ListRecentPingSummariesForChecks is the batched ListRecentPingSummaries: the
// dashboard's hot query, hence no body. Checks with no pings are absent.
// ROW_NUMBER() needs SQLite >= 3.25.
func (s *Store) ListRecentPingSummariesForChecks(ctx context.Context, checkIDs []int64, perCheckLimit int64) (map[int64][]models.PingSummary, error) {
	byCheck := map[int64][]models.PingSummary{}
	if len(checkIDs) == 0 {
		return byCheck, nil
	}
	args := append(int64Args(checkIDs), perCheckLimit)
	rows, err := s.DB.QueryContext(ctx,
		"SELECT id, check_id, kind, created_at FROM ( "+
			"SELECT p.id, p.check_id, p.kind, p.created_at, "+
			"ROW_NUMBER() OVER (PARTITION BY p.check_id ORDER BY p.id DESC) AS rn "+
			"FROM pings p WHERE p.check_id IN ("+placeholders(len(checkIDs), 1)+") "+
			") sub WHERE rn <= "+placeholders(1, len(checkIDs)+1)+" ORDER BY check_id, id DESC",
		args...)
	if err != nil {
		return nil, err
	}
	summaries, err := readRows(rows, decodePingSummary)
	if err != nil {
		return nil, err
	}
	for _, summary := range summaries {
		byCheck[summary.CheckID] = append(byCheck[summary.CheckID], summary)
	}
	return byCheck, nil
}

// DeletePingsBefore: created_at is RFC 3339 UTC text, so the lexicographic <
// is chronological.
func (s *Store) DeletePingsBefore(ctx context.Context, cutoff string) (int64, error) {
	return s.deleteCount(ctx, "DELETE FROM pings WHERE created_at < $1", cutoff)
}
