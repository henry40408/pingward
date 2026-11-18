// Package models holds the rows the store reads and writes. Text enums keep
// the exact strings stored in the database.
package models

import (
	"fmt"
	"time"
)

type ScheduleKind string

const (
	SchedulePeriod ScheduleKind = "period"
	ScheduleCron   ScheduleKind = "cron"
)

type CheckStatus string

const (
	StatusNew    CheckStatus = "new"
	StatusUp     CheckStatus = "up"
	StatusDown   CheckStatus = "down"
	StatusPaused CheckStatus = "paused"
)

type PingKind string

const (
	PingSuccess  PingKind = "success"
	PingFail     PingKind = "fail"
	PingStart    PingKind = "start"
	PingLog      PingKind = "log"
	PingExitcode PingKind = "exitcode"
)

type ChannelKind string

const (
	ChannelWebhook  ChannelKind = "webhook"
	ChannelTelegram ChannelKind = "telegram"
	ChannelSlack    ChannelKind = "slack"
	ChannelNtfy     ChannelKind = "ntfy"
	ChannelPushover ChannelKind = "pushover"
	ChannelEmail    ChannelKind = "email"
)

type NotifyStatus string

const (
	NotifyOK    NotifyStatus = "ok"
	NotifyError NotifyStatus = "error"
)

// EventKind is what a notification reports. It lives here, not in the
// notifier package, so the store can decode it without importing the notifiers.
type EventKind string

const (
	EventDown     EventKind = "down"
	EventUp       EventKind = "up"
	EventReminder EventKind = "reminder"
	EventTest     EventKind = "test"
)

func ParseScheduleKind(text string) (ScheduleKind, error) {
	return parseEnum("ScheduleKind", text, SchedulePeriod, ScheduleCron)
}

func ParseCheckStatus(text string) (CheckStatus, error) {
	return parseEnum("CheckStatus", text, StatusNew, StatusUp, StatusDown, StatusPaused)
}

func ParsePingKind(text string) (PingKind, error) {
	return parseEnum("PingKind", text, PingSuccess, PingFail, PingStart, PingLog, PingExitcode)
}

func ParseChannelKind(text string) (ChannelKind, error) {
	return parseEnum("ChannelKind", text,
		ChannelWebhook, ChannelTelegram, ChannelSlack, ChannelNtfy, ChannelPushover, ChannelEmail)
}

func ParseNotifyStatus(text string) (NotifyStatus, error) {
	return parseEnum("NotifyStatus", text, NotifyOK, NotifyError)
}

func ParseEventKind(text string) (EventKind, error) {
	return parseEnum("EventKind", text, EventDown, EventUp, EventReminder, EventTest)
}

func parseEnum[T ~string](name, text string, allowed ...T) (T, error) {
	for _, value := range allowed {
		if string(value) == text {
			return value, nil
		}
	}
	return "", fmt.Errorf("invalid %s: %s", name, text)
}

type Check struct {
	ID               int64
	ProjectID        int64
	Name             string
	Description      string
	PingUUID         string
	ScheduleKind     ScheduleKind
	PeriodSecs       *int64
	GraceSecs        int64
	CronExpr         *string
	Timezone         string
	Status           CheckStatus
	LastPingAt       *time.Time
	LastStartAt      *time.Time
	NextDueAt        *time.Time
	ScanIntervalSecs *int64
	MaxRuntimeSecs   *int64
	NagIntervalSecs  *int64
	LastAlertAt      *time.Time
	Acknowledged     bool
	CreatedAt        time.Time
}

type User struct {
	ID           int64
	Username     string
	PasswordHash *string
	IsAdmin      bool
	Disabled     bool
	CreatedAt    time.Time
}

type Project struct {
	ID               int64
	UserID           int64
	Name             string
	Description      string
	ScanIntervalSecs *int64
	NagIntervalSecs  *int64
	CreatedAt        time.Time
}

type Channel struct {
	ID         int64
	ProjectID  int64
	Kind       ChannelKind
	Name       string
	ConfigJSON string
	CreatedAt  time.Time
}

type Ping struct {
	ID        int64
	CheckID   int64
	Kind      PingKind
	ExitCode  *int64
	Body      string
	SourceIP  *string
	CreatedAt time.Time
}

// PingSummary is the subset of a Ping the heartbeat strip and run-duration
// pairing read. It omits Body (up to 10 KiB each), whose decoding dominated
// dashboard render time.
type PingSummary struct {
	ID        int64
	CheckID   int64
	Kind      PingKind
	CreatedAt time.Time
}

func (p Ping) Summary() PingSummary {
	return PingSummary{ID: p.ID, CheckID: p.CheckID, Kind: p.Kind, CreatedAt: p.CreatedAt}
}

type Notification struct {
	ID        int64
	CheckID   int64
	ChannelID int64
	Event     EventKind
	Status    NotifyStatus
	Error     *string
	CreatedAt time.Time
}

// ApiKey is a stored API key's metadata; only the token's SHA-256 hash is
// persisted. Prefix is a non-secret display fragment.
type ApiKey struct {
	ID         int64
	UserID     int64
	Name       string
	Prefix     string
	CreatedAt  time.Time
	LastUsedAt *time.Time
	ExpiresAt  *time.Time
}

// Session is one stored login session. ID is the cookie's bearer secret and
// must never be rendered: the UI identifies a session by the SHA-256 hash of it.
type Session struct {
	ID         string
	UserID     int64
	CreatedAt  *time.Time
	LastSeenAt *time.Time
	ExpiresAt  time.Time
	UserAgent  *string
	IP         *string
	SSO        bool
}

type AuditLog struct {
	ID            int64
	ActorUserID   int64
	ActorUsername string
	Action        string
	TargetType    *string
	TargetID      *int64
	TargetOwnerID *int64
	Method        *string
	Path          *string
	Detail        *string
	CreatedAt     time.Time
}
