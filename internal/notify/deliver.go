package notify

import (
	"context"
	"log/slog"
	"time"

	"github.com/henry40408/pingward/internal/config"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
)

// RetryPolicy retries any error with exponential backoff.
type RetryPolicy struct {
	MaxAttempts int
	BaseBackoff time.Duration
}

// DefaultRetryPolicy is three attempts, 500ms then 1s apart.
var DefaultRetryPolicy = RetryPolicy{MaxAttempts: 3, BaseBackoff: 500 * time.Millisecond}

// SendWithRetry returns the last error when every attempt fails.
func SendWithRetry(ctx context.Context, notifier Notifier, ev Event, policy RetryPolicy) error {
	attempts := max(policy.MaxAttempts, 1)
	var last error
	for attempt := 0; attempt < attempts; attempt++ {
		last = notifier.Send(ctx, ev)
		if last == nil {
			return nil
		}
		if attempt+1 == attempts {
			break
		}
		select {
		case <-time.After(policy.BaseBackoff << attempt):
		case <-ctx.Done():
			return last
		}
	}
	return last
}

// DeliveryStore is the part of the store delivery needs.
type DeliveryStore interface {
	ChannelsForCheck(ctx context.Context, checkID int64) ([]models.Channel, error)
	RecordNotification(ctx context.Context, notification store.NewNotification, now time.Time) error
}

// Deliverer carries events to every channel bound to a check.
type Deliverer struct {
	Store DeliveryStore
	Retry RetryPolicy
	SMTP  *config.SMTPConfig
}

// DeliverEvent delivers to each bound channel with retry and records every
// outcome. Failures are recorded, never returned: a failing channel must not
// affect check state.
func (d Deliverer) DeliverEvent(ctx context.Context, ev Event, now time.Time) {
	channels, err := d.Store.ChannelsForCheck(ctx, ev.CheckID)
	if err != nil {
		slog.Error("failed to load channels", "check_id", ev.CheckID, "error", err)
		return
	}
	if len(channels) == 0 {
		slog.Debug("no channels bound; nothing to deliver", "check", ev.CheckName, "event", string(ev.Kind))
		return
	}
	for _, channel := range channels {
		notifier, ok := NotifierFor(channel, d.SMTP)
		if !ok {
			continue
		}
		d.record(ctx, ev, channel, SendWithRetry(ctx, notifier, ev, d.Retry), now)
	}
}

func (d Deliverer) record(ctx context.Context, ev Event, channel models.Channel, sendErr error, now time.Time) {
	notification := store.NewNotification{
		CheckID: ev.CheckID, ChannelID: channel.ID, Event: ev.Kind, Status: models.NotifyOK,
	}
	if sendErr != nil {
		text := sendErr.Error()
		notification.Status = models.NotifyError
		notification.Error = &text
	}
	if err := d.Store.RecordNotification(ctx, notification, now); err != nil {
		slog.Error("failed to record notification", "check_id", ev.CheckID, "channel_id", channel.ID, "error", err)
	}
}
