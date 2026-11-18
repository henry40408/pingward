// Package prune deletes rows past their retention and expired sessions.
package prune

import (
	"context"
	"log/slog"
	"strconv"
	"strings"
	"time"

	"github.com/henry40408/pingward/internal/db"
	"github.com/henry40408/pingward/internal/store"
)

// Counts is how many rows one pass deleted.
type Counts struct {
	Pings         int64
	Notifications int64
	Audit         int64
	Sessions      int64
}

func (c Counts) empty() bool { return c == Counts{} }

// minCutoff is the earliest date chrono can represent; a retention reaching
// further back is out of range and prunes nothing.
var minCutoff = time.Date(-262143, 1, 1, 0, 0, 0, 0, time.UTC)

// parseDays is false (retention off) when unset, blank, non-numeric, or
// not positive.
func parseDays(setting *string) (int64, bool) {
	if setting == nil {
		return 0, false
	}
	days, err := strconv.ParseInt(strings.TrimSpace(*setting), 10, 64)
	if err != nil || days <= 0 {
		return 0, false
	}
	return days, true
}

// retentionCutoff is the text bound for created_at < cutoff. ok is false when
// retention is off or now-days is out of range, which fails safe.
func retentionCutoff(now time.Time, setting *string) (string, bool) {
	days, ok := parseDays(setting)
	if !ok {
		return "", false
	}
	maxDays := (now.Unix() - minCutoff.Unix()) / 86400
	if days > maxDays {
		slog.Warn("retention is out of range; skipping prune this run", "days", days)
		return "", false
	}
	return db.FormatTime(now.AddDate(0, 0, -int(days))), true
}

type table struct {
	settingKey  string
	deleteOlder func(s *store.Store, ctx context.Context, cutoff string) (int64, error)
}

var (
	pings         = table{"pings_retention_days", (*store.Store).DeletePingsBefore}
	notifications = table{"notifications_retention_days", (*store.Store).DeleteNotificationsBefore}
	audit         = table{"audit_retention_days", (*store.Store).DeleteAuditBefore}
)

// pruneTable returns the rows deleted; 0 when retention is off.
func pruneTable(ctx context.Context, s *store.Store, now time.Time, t table) (int64, error) {
	setting, err := s.GetSetting(ctx, t.settingKey)
	if err != nil {
		return 0, err
	}
	cutoff, ok := retentionCutoff(now, setting)
	if !ok {
		return 0, nil
	}
	return t.deleteOlder(s, ctx, cutoff)
}

// Once deletes pings, notifications and audit rows past their retention, plus
// expired sessions unconditionally. Every retention defaults to off, so an
// upgrade never starts deleting audit records.
func Once(ctx context.Context, s *store.Store, now time.Time) (Counts, error) {
	var counts Counts
	var err error
	if counts.Pings, err = pruneTable(ctx, s, now, pings); err != nil {
		return counts, err
	}
	if counts.Notifications, err = pruneTable(ctx, s, now, notifications); err != nil {
		return counts, err
	}
	if counts.Audit, err = pruneTable(ctx, s, now, audit); err != nil {
		return counts, err
	}
	counts.Sessions, err = s.DeleteExpiredSessions(ctx, now)
	return counts, err
}

// Loop is what RunLoop needs.
type Loop struct {
	Store        *store.Store
	Logger       *slog.Logger
	IntervalSecs int64
	Now          func() time.Time
}

// Run prunes now, then every IntervalSecs (at least 1). ctx is checked only at
// the sleep, so an in-flight pass finishes before the pool closes.
func (l Loop) Run(ctx context.Context) {
	interval := time.Duration(max(l.IntervalSecs, 1)) * time.Second
	passCtx := context.WithoutCancel(ctx)
	for {
		l.pass(passCtx)
		select {
		case <-time.After(interval):
		case <-ctx.Done():
			l.Logger.Info("prune loop stopping")
			return
		}
	}
}

func (l Loop) pass(ctx context.Context) {
	counts, err := Once(ctx, l.Store, l.Now())
	if err != nil {
		l.Logger.Error("prune_once failed", "error", err)
	} else {
		l.logCounts(counts)
	}
	// A failed heartbeat write is dropped unlogged, like last_scan_at.
	_ = l.Store.SetSetting(ctx, "last_prune_at", db.FormatTime(l.Now()))
}

func (l Loop) logCounts(counts Counts) {
	if counts.empty() {
		return
	}
	l.Logger.Info("pruned " + strconv.FormatInt(counts.Pings, 10) + " pings, " +
		strconv.FormatInt(counts.Notifications, 10) + " notifications, " +
		strconv.FormatInt(counts.Audit, 10) + " audit entries, " +
		strconv.FormatInt(counts.Sessions, 10) + " sessions")
	if counts.Sessions > 0 {
		// An aggregate: the store returns only a count.
		l.Logger.Info("session.destroyed", "target", "session", "reason", "expired", "count", counts.Sessions)
	}
}
