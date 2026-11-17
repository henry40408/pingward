package store

import (
	"fmt"

	"github.com/henry40408/pingward/internal/models"
)

func decodeUser(r *row) (models.User, error) {
	user := models.User{
		ID:           r.int64("id"),
		Username:     r.text("username"),
		PasswordHash: r.optionalText("password_hash"),
		IsAdmin:      r.bool("is_admin"),
		Disabled:     r.bool("disabled"),
		CreatedAt:    r.requiredTime("users", "created_at"),
	}
	return user, r.err
}

func decodeAPIKey(r *row) (models.ApiKey, error) {
	key := models.ApiKey{
		ID:         r.int64("id"),
		UserID:     r.int64("user_id"),
		Name:       r.text("name"),
		Prefix:     r.text("prefix"),
		CreatedAt:  r.requiredTime("api_keys", "created_at"),
		LastUsedAt: r.optionalTime("last_used_at"),
		ExpiresAt:  r.optionalTime("expires_at"),
	}
	return key, r.err
}

func decodeSession(r *row) (models.Session, error) {
	session := models.Session{
		ID:         r.text("id"),
		UserID:     r.int64("user_id"),
		CreatedAt:  r.optionalTime("created_at"),
		LastSeenAt: r.optionalTime("last_seen_at"),
		ExpiresAt:  r.requiredTime("sessions", "expires_at"),
		UserAgent:  r.optionalText("user_agent"),
		IP:         r.optionalText("ip"),
		SSO:        r.bool("sso"),
	}
	return session, r.err
}

func decodeAudit(r *row) (models.AuditLog, error) {
	entry := models.AuditLog{
		ID:            r.int64("id"),
		ActorUserID:   r.int64("actor_user_id"),
		ActorUsername: r.text("actor_username"),
		Action:        r.text("action"),
		TargetType:    r.optionalText("target_type"),
		TargetID:      r.optionalInt64("target_id"),
		TargetOwnerID: r.optionalInt64("target_owner_id"),
		Method:        r.optionalText("method"),
		Path:          r.optionalText("path"),
		Detail:        r.optionalText("detail"),
		CreatedAt:     r.requiredTime("audit_log", "created_at"),
	}
	return entry, r.err
}

func decodeProject(r *row) (models.Project, error) {
	project := models.Project{
		ID:               r.int64("id"),
		UserID:           r.int64("user_id"),
		Name:             r.text("name"),
		Description:      r.text("description"),
		ScanIntervalSecs: r.optionalInt64("scan_interval_secs"),
		NagIntervalSecs:  r.optionalInt64("nag_interval_secs"),
		CreatedAt:        r.requiredTime("projects", "created_at"),
	}
	return project, r.err
}

func decodeChannel(r *row) (models.Channel, error) {
	kind, err := models.ParseChannelKind(r.text("kind"))
	if err != nil {
		return models.Channel{}, fmt.Errorf("invalid channel kind: %w", err)
	}
	channel := models.Channel{
		ID:         r.int64("id"),
		ProjectID:  r.int64("project_id"),
		Kind:       kind,
		Name:       r.text("name"),
		ConfigJSON: r.text("config_json"),
		CreatedAt:  r.requiredTime("channels", "created_at"),
	}
	return channel, r.err
}

func decodeCheck(r *row) (models.Check, error) {
	kind, err := models.ParseScheduleKind(r.text("schedule_kind"))
	if err != nil {
		return models.Check{}, fmt.Errorf("invalid schedule_kind: %w", err)
	}
	status, err := models.ParseCheckStatus(r.text("status"))
	if err != nil {
		return models.Check{}, fmt.Errorf("invalid status: %w", err)
	}
	check := models.Check{
		ID:               r.int64("id"),
		ProjectID:        r.int64("project_id"),
		Name:             r.text("name"),
		Description:      r.text("description"),
		PingUUID:         r.text("ping_uuid"),
		ScheduleKind:     kind,
		PeriodSecs:       r.optionalInt64("period_secs"),
		GraceSecs:        r.int64("grace_secs"),
		CronExpr:         r.optionalText("cron_expr"),
		Timezone:         r.text("timezone"),
		Status:           status,
		LastPingAt:       r.optionalTime("last_ping_at"),
		LastStartAt:      r.optionalTime("last_start_at"),
		NextDueAt:        r.optionalTime("next_due_at"),
		ScanIntervalSecs: r.optionalInt64("scan_interval_secs"),
		MaxRuntimeSecs:   r.optionalInt64("max_runtime_secs"),
		NagIntervalSecs:  r.optionalInt64("nag_interval_secs"),
		LastAlertAt:      r.optionalTime("last_alert_at"),
		Acknowledged:     r.bool("acknowledged"),
		CreatedAt:        r.requiredTime("checks", "created_at"),
	}
	return check, r.err
}

func decodePing(r *row) (models.Ping, error) {
	kind, err := models.ParsePingKind(r.text("kind"))
	if err != nil {
		return models.Ping{}, fmt.Errorf("invalid ping kind: %w", err)
	}
	ping := models.Ping{
		ID:        r.int64("id"),
		CheckID:   r.int64("check_id"),
		Kind:      kind,
		ExitCode:  r.optionalInt64("exit_code"),
		Body:      r.text("body"),
		SourceIP:  r.optionalText("source_ip"),
		CreatedAt: r.requiredTime("pings", "created_at"),
	}
	return ping, r.err
}

// decodePingSummary is the narrow counterpart of decodePing; keep the two in step.
func decodePingSummary(r *row) (models.PingSummary, error) {
	kind, err := models.ParsePingKind(r.text("kind"))
	if err != nil {
		return models.PingSummary{}, fmt.Errorf("invalid ping kind: %w", err)
	}
	summary := models.PingSummary{
		ID:        r.int64("id"),
		CheckID:   r.int64("check_id"),
		Kind:      kind,
		CreatedAt: r.requiredTime("pings", "created_at"),
	}
	return summary, r.err
}

func decodeNotification(r *row) (models.Notification, error) {
	event, err := models.ParseEventKind(r.text("event"))
	if err != nil {
		return models.Notification{}, fmt.Errorf("invalid notification event: %w", err)
	}
	status, err := models.ParseNotifyStatus(r.text("status"))
	if err != nil {
		return models.Notification{}, fmt.Errorf("invalid notification status: %w", err)
	}
	notification := models.Notification{
		ID:        r.int64("id"),
		CheckID:   r.int64("check_id"),
		ChannelID: r.int64("channel_id"),
		Event:     event,
		Status:    status,
		Error:     r.optionalText("error"),
		CreatedAt: r.requiredTime("notifications", "created_at"),
	}
	return notification, r.err
}
