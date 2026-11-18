package scheduler

import (
	"context"
	"log/slog"
	"time"

	"github.com/henry40408/pingward/internal/db"
	"github.com/henry40408/pingward/internal/events"
	"github.com/henry40408/pingward/internal/notify"
	"github.com/henry40408/pingward/internal/store"
)

// Deps is what the scan and nag passes need.
type Deps struct {
	Store   *store.Store
	Logger  *slog.Logger
	BaseURL string
	// EnvDefaultScanSecs is the last resort of the interval cascade, after
	// check, project and global settings.
	EnvDefaultScanSecs int64
	// Events publishes each Down for the live tail; nil disables it.
	Events *events.Broker
	// Deliver sends an event to the check's channels. Each call runs in its own
	// untracked goroutine: a slow channel must not hold up the next pass.
	Deliver func(ctx context.Context, event notify.Event, now time.Time)
	Now     func() time.Time
}

// RunScanLoop runs scan, heartbeat, nag, then sleeps for the smallest scan
// interval, forever. now is read only here so the passes stay deterministic.
// ctx is checked only at the sleep, so an in-flight pass finishes before the
// caller closes the database.
func (d Deps) RunScanLoop(ctx context.Context) {
	passCtx := context.WithoutCancel(ctx)
	for {
		d.scanPass(passCtx)
		d.nagPass(passCtx)
		select {
		case <-time.After(time.Duration(d.nextIntervalSecs(passCtx)) * time.Second):
		case <-ctx.Done():
			d.Logger.Info("scan loop stopping")
			return
		}
	}
}

func (d Deps) scanPass(ctx context.Context) {
	now := d.Now()
	downs, err := d.ScanOnce(ctx, now)
	if err != nil {
		d.Logger.Error("scan_once failed", "error", err)
	}
	for _, event := range downs {
		if d.Events != nil && d.Events.Subscribers() > 0 {
			d.Events.Publish(event.CheckID)
		}
	}
	d.deliver(ctx, downs)
	// Heartbeat shown on /admin; a failed write is dropped unlogged and only
	// surfaces as a stale heartbeat.
	_ = d.Store.SetSetting(ctx, "last_scan_at", db.FormatTime(now))
}

func (d Deps) nagPass(ctx context.Context) {
	reminders, err := d.NagOnce(ctx, d.Now())
	if err != nil {
		d.Logger.Error("nag_once failed", "error", err)
	}
	d.deliver(ctx, reminders)
}

func (d Deps) deliver(ctx context.Context, batch []notify.Event) {
	if d.Deliver == nil {
		return
	}
	for _, event := range batch {
		go d.Deliver(ctx, event, d.Now())
	}
}

// nextIntervalSecs falls back to the environment default when a query fails.
func (d Deps) nextIntervalSecs(ctx context.Context) int64 {
	active, _ := d.Store.ListActiveChecks(ctx)
	projects, _ := d.Store.AllProjectScanIntervals(ctx)
	global, _ := d.intervalSetting(ctx, "scan_interval")
	return loopIntervalSecs(active, projects, global, d.EnvDefaultScanSecs)
}
