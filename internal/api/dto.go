package api

import (
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
)

// The DTOs keep JSON tags off the domain types. Field order is the wire order.
// ChannelDTO has no config: it holds secrets.

type ProjectDTO struct {
	ID               int64  `json:"id"`
	OwnerID          int64  `json:"owner_id"`
	Name             string `json:"name"`
	Description      string `json:"description"`
	ScanIntervalSecs *int64 `json:"scan_interval_secs"`
	NagIntervalSecs  *int64 `json:"nag_interval_secs"`
	CreatedAt        Time   `json:"created_at"`
}

func projectDTO(p models.Project) ProjectDTO {
	return ProjectDTO{
		ID: p.ID, OwnerID: p.UserID, Name: p.Name, Description: p.Description,
		ScanIntervalSecs: p.ScanIntervalSecs, NagIntervalSecs: p.NagIntervalSecs, CreatedAt: timeOf(p.CreatedAt),
	}
}

type CheckDTO struct {
	ID               int64   `json:"id"`
	ProjectID        int64   `json:"project_id"`
	Name             string  `json:"name"`
	Description      string  `json:"description"`
	PingUUID         string  `json:"ping_uuid"`
	Status           string  `json:"status"`
	ScheduleKind     string  `json:"schedule_kind"`
	PeriodSecs       *int64  `json:"period_secs"`
	GraceSecs        int64   `json:"grace_secs"`
	CronExpr         *string `json:"cron_expr"`
	Timezone         string  `json:"timezone"`
	ScanIntervalSecs *int64  `json:"scan_interval_secs"`
	MaxRuntimeSecs   *int64  `json:"max_runtime_secs"`
	NagIntervalSecs  *int64  `json:"nag_interval_secs"`
	LastPingAt       *Time   `json:"last_ping_at"`
	LastStartAt      *Time   `json:"last_start_at"`
	NextDueAt        *Time   `json:"next_due_at"`
	Acknowledged     bool    `json:"acknowledged"`
	CreatedAt        Time    `json:"created_at"`
}

func checkDTO(c models.Check) CheckDTO {
	return CheckDTO{
		ID: c.ID, ProjectID: c.ProjectID, Name: c.Name, Description: c.Description, PingUUID: c.PingUUID,
		Status: string(c.Status), ScheduleKind: string(c.ScheduleKind), PeriodSecs: c.PeriodSecs,
		GraceSecs: c.GraceSecs, CronExpr: c.CronExpr, Timezone: c.Timezone,
		ScanIntervalSecs: c.ScanIntervalSecs, MaxRuntimeSecs: c.MaxRuntimeSecs, NagIntervalSecs: c.NagIntervalSecs,
		LastPingAt: optionalTime(c.LastPingAt), LastStartAt: optionalTime(c.LastStartAt),
		NextDueAt: optionalTime(c.NextDueAt), Acknowledged: c.Acknowledged, CreatedAt: timeOf(c.CreatedAt),
	}
}

type ChannelDTO struct {
	ID        int64  `json:"id"`
	ProjectID int64  `json:"project_id"`
	Kind      string `json:"kind"`
	Name      string `json:"name"`
	CreatedAt Time   `json:"created_at"`
}

func channelDTO(c models.Channel) ChannelDTO {
	return ChannelDTO{ID: c.ID, ProjectID: c.ProjectID, Kind: string(c.Kind), Name: c.Name, CreatedAt: timeOf(c.CreatedAt)}
}

type PingDTO struct {
	ID        int64   `json:"id"`
	CheckID   int64   `json:"check_id"`
	Kind      string  `json:"kind"`
	ExitCode  *int64  `json:"exit_code"`
	Body      string  `json:"body"`
	SourceIP  *string `json:"source_ip"`
	CreatedAt Time    `json:"created_at"`
}

func pingDTO(p models.Ping) PingDTO {
	return PingDTO{
		ID: p.ID, CheckID: p.CheckID, Kind: string(p.Kind), ExitCode: p.ExitCode, Body: p.Body,
		SourceIP: p.SourceIP, CreatedAt: timeOf(p.CreatedAt),
	}
}

type NotificationDTO struct {
	ID        int64   `json:"id"`
	CheckID   int64   `json:"check_id"`
	ChannelID int64   `json:"channel_id"`
	Event     string  `json:"event"`
	Status    string  `json:"status"`
	Error     *string `json:"error"`
	CreatedAt Time    `json:"created_at"`
}

func notificationDTO(n models.Notification) NotificationDTO {
	return NotificationDTO{
		ID: n.ID, CheckID: n.CheckID, ChannelID: n.ChannelID, Event: string(n.Event),
		Status: string(n.Status), Error: n.Error, CreatedAt: timeOf(n.CreatedAt),
	}
}

// APIKeyDTO is one of the caller's keys. The secret is never included, only its prefix.
type APIKeyDTO struct {
	ID         int64  `json:"id"`
	Name       string `json:"name"`
	Prefix     string `json:"prefix"`
	CreatedAt  Time   `json:"created_at"`
	LastUsedAt *Time  `json:"last_used_at"`
	ExpiresAt  *Time  `json:"expires_at"`
}

func apiKeyDTO(k models.ApiKey) APIKeyDTO {
	return APIKeyDTO{
		ID: k.ID, Name: k.Name, Prefix: k.Prefix, CreatedAt: timeOf(k.CreatedAt),
		LastUsedAt: optionalTime(k.LastUsedAt), ExpiresAt: optionalTime(k.ExpiresAt),
	}
}

// Page is a keyset-paginated slice, newest first. NextBefore is the id to pass
// as ?before= for the older page, NextAfter the id for ?after= toward newer;
// each is null when there is no such page.
type Page[T any] struct {
	Items      []T    `json:"items"`
	HasNewer   bool   `json:"has_newer"`
	HasOlder   bool   `json:"has_older"`
	NextBefore *int64 `json:"next_before"`
	NextAfter  *int64 `json:"next_after"`
}

func newPage[M, T any](page store.Page[M], idOf func(M) int64, toDTO func(M) T) Page[T] {
	out := Page[T]{Items: make([]T, 0, len(page.Items)), HasNewer: page.HasNewer, HasOlder: page.HasOlder}
	for _, item := range page.Items {
		out.Items = append(out.Items, toDTO(item))
	}
	if page.HasOlder && len(page.Items) > 0 {
		id := idOf(page.Items[len(page.Items)-1])
		out.NextBefore = &id
	}
	if page.HasNewer && len(page.Items) > 0 {
		id := idOf(page.Items[0])
		out.NextAfter = &id
	}
	return out
}

// BoundChannels is the channel ids bound to a check after a bind update.
type BoundChannels struct {
	ChannelIDs []int64 `json:"channel_ids"`
}

func list[M, T any](items []M, toDTO func(M) T) []T {
	out := make([]T, 0, len(items))
	for _, item := range items {
		out = append(out, toDTO(item))
	}
	return out
}
