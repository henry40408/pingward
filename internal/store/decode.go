package store

import "github.com/henry40408/pingward/internal/models"

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
