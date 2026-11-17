package store

import (
	"context"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

type NewChannel struct {
	ProjectID  int64
	Kind       models.ChannelKind
	Name       string
	ConfigJSON string
}

func (s *Store) CreateChannel(ctx context.Context, channel NewChannel, now time.Time) (int64, error) {
	var id int64
	err := s.DB.QueryRowContext(ctx,
		"INSERT INTO channels (project_id, kind, name, config_json, created_at) VALUES ($1,$2,$3,$4,$5) "+
			"RETURNING id",
		channel.ProjectID, string(channel.Kind), channel.Name, channel.ConfigJSON, timeText(now)).Scan(&id)
	return id, err
}

// FindChannel returns nil when no channel has the id.
func (s *Store) FindChannel(ctx context.Context, id int64) (*models.Channel, error) {
	rows, err := s.DB.QueryContext(ctx, "SELECT * FROM channels WHERE id = $1", id)
	if err != nil {
		return nil, err
	}
	channel, found, err := readOptionalRow(rows, decodeChannel)
	if err != nil || !found {
		return nil, err
	}
	return &channel, nil
}

func (s *Store) ListChannelsForProject(ctx context.Context, projectID int64) ([]models.Channel, error) {
	rows, err := s.DB.QueryContext(ctx, "SELECT * FROM channels WHERE project_id = $1 ORDER BY id", projectID)
	if err != nil {
		return nil, err
	}
	return readRows(rows, decodeChannel)
}

// UpdateChannel leaves kind alone: config_json only means something to the
// kind that wrote it.
func (s *Store) UpdateChannel(ctx context.Context, id int64, name, configJSON string) error {
	_, err := s.DB.ExecContext(ctx, "UPDATE channels SET name = $1, config_json = $2 WHERE id = $3",
		name, configJSON, id)
	return err
}

func (s *Store) DeleteChannel(ctx context.Context, id int64) error {
	_, err := s.DB.ExecContext(ctx, "DELETE FROM channels WHERE id = $1", id)
	return err
}

func (s *Store) BindChannel(ctx context.Context, checkID, channelID int64) error {
	// Not SQLite-only INSERT OR IGNORE, a parse error on Postgres.
	_, err := s.DB.ExecContext(ctx,
		"INSERT INTO check_channels (check_id, channel_id) VALUES ($1,$2) ON CONFLICT DO NOTHING",
		checkID, channelID)
	return err
}

// BindAllProjectChannels binds a new check to every channel on its project.
// The WHERE is also syntax: in INSERT ... SELECT ... ON CONFLICT, SQLite needs
// it to tell the upsert's ON from a join's.
func (s *Store) BindAllProjectChannels(ctx context.Context, checkID, projectID int64) error {
	_, err := s.DB.ExecContext(ctx,
		"INSERT INTO check_channels (check_id, channel_id) "+
			"SELECT $1, id FROM channels WHERE project_id = $2 "+
			"ON CONFLICT DO NOTHING", checkID, projectID)
	return err
}

// ChecksWithChannels reports which of checkIDs have at least one bound channel, in one query.
func (s *Store) ChecksWithChannels(ctx context.Context, checkIDs []int64) (map[int64]bool, error) {
	bound := map[int64]bool{}
	if len(checkIDs) == 0 {
		return bound, nil
	}
	rows, err := s.DB.QueryContext(ctx,
		"SELECT DISTINCT check_id FROM check_channels WHERE check_id IN ("+placeholders(len(checkIDs), 1)+")",
		int64Args(checkIDs)...)
	if err != nil {
		return nil, err
	}
	_, err = readRows(rows, func(r *row) (struct{}, error) {
		bound[r.int64("check_id")] = true
		return struct{}{}, r.err
	})
	return bound, err
}

func (s *Store) UnbindChannel(ctx context.Context, checkID, channelID int64) error {
	_, err := s.DB.ExecContext(ctx,
		"DELETE FROM check_channels WHERE check_id = $1 AND channel_id = $2", checkID, channelID)
	return err
}

func (s *Store) BoundChannelIDs(ctx context.Context, checkID int64) ([]int64, error) {
	rows, err := s.DB.QueryContext(ctx,
		"SELECT channel_id FROM check_channels WHERE check_id = $1 ORDER BY channel_id", checkID)
	if err != nil {
		return nil, err
	}
	return readRows(rows, func(r *row) (int64, error) { return r.int64("channel_id"), r.err })
}

func (s *Store) ChannelsForCheck(ctx context.Context, checkID int64) ([]models.Channel, error) {
	rows, err := s.DB.QueryContext(ctx,
		"SELECT c.* FROM channels c JOIN check_channels cc ON cc.channel_id = c.id "+
			"WHERE cc.check_id = $1 ORDER BY c.id", checkID)
	if err != nil {
		return nil, err
	}
	return readRows(rows, decodeChannel)
}
