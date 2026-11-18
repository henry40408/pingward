package store

import (
	"context"
	"time"

	"github.com/henry40408/pingward/internal/models"
)

type NewNotification struct {
	CheckID   int64
	ChannelID int64
	Event     models.EventKind
	Status    models.NotifyStatus
	Error     *string
}

func (s *Store) RecordNotification(ctx context.Context, notification NewNotification, now time.Time) error {
	_, err := s.DB.ExecContext(ctx,
		"INSERT INTO notifications (check_id, channel_id, event, status, error, created_at) "+
			"VALUES ($1,$2,$3,$4,$5,$6)",
		notification.CheckID, notification.ChannelID, string(notification.Event),
		string(notification.Status), notification.Error, timeText(now))
	return err
}

func (s *Store) ListRecentNotifications(ctx context.Context, checkID, limit int64) ([]models.Notification, error) {
	rows, err := s.DB.QueryContext(ctx,
		"SELECT * FROM notifications WHERE check_id = $1 ORDER BY id DESC LIMIT $2", checkID, limit)
	if err != nil {
		return nil, err
	}
	return readRows(rows, decodeNotification)
}

func (s *Store) DeleteNotificationsBefore(ctx context.Context, cutoff string) (int64, error) {
	return s.deleteCount(ctx, "DELETE FROM notifications WHERE created_at < $1", cutoff)
}
