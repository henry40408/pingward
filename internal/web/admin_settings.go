package web

import (
	"context"
	"fmt"
	"net/http"
	"strconv"
	"strings"

	"github.com/henry40408/pingward/internal/duration"
	"github.com/henry40408/pingward/internal/store"
)

type settingKind int

const (
	settingDuration settingKind = iota // raw seconds or 5m, 1h30m
	settingDays                        // a plain positive integer
	settingTimezone                    // an IANA name
)

// setting is one validated key and the text to store ("" clears it).
type setting struct{ key, value string }

func readSettingsFields(r *http.Request) settingsFields {
	field := r.PostFormValue
	return settingsFields{
		scanInterval: field("scan_interval"), nagInterval: field("nag_interval"),
		pingsRetentionDays: field("pings_retention_days"), notificationsRetentionDays: field("notifications_retention_days"),
		auditRetentionDays: field("audit_retention_days"), displayTimezone: field("display_timezone"),
	}
}

func (d *Deps) settingsSave(w http.ResponseWriter, r *http.Request) {
	admin, ok := d.requireAdmin(w, r)
	if !ok {
		return
	}
	form := readSettingsFields(r)
	parsed, err := validateSettings(form)
	if err != nil {
		// The audit card just comes back on its default latest page.
		d.renderAdmin(w, r, admin, adminNotices{settings: form, settingsError: err.Error()}, auditQuery{})
		return
	}
	ctx := writeCtx(r)
	// Diffed before writing, so the detail names the fields actually touched.
	changed, err := d.changedSettings(ctx, parsed)
	if err != nil {
		internalError(w)
		return
	}
	for _, entry := range parsed {
		if err := d.Store.SetSetting(ctx, entry.key, entry.value); err != nil {
			internalError(w)
			return
		}
	}
	if len(changed) > 0 {
		if err := d.auditSettingsUpdate(ctx, admin.ID, admin.Username, strings.Join(changed, " ")); err != nil {
			internalError(w)
			return
		}
	}
	http.SetCookie(w, d.flashCookie("settings"))
	redirect(w, r, "/admin")
}

// auditSettingsUpdate matters because shortening audit_retention_days could
// erase an admin's trail.
func (d *Deps) auditSettingsUpdate(ctx context.Context, adminID int64, adminName, detail string) error {
	method, path := http.MethodPost, "/admin/settings"
	_, err := d.Store.RecordAudit(ctx, store.NewAudit{
		ActorUserID: adminID, ActorUsername: adminName, Action: "settings.update",
		Method: &method, Path: &path, Detail: &detail,
	}, d.Now())
	return err
}

// validateSettings is atomic: it checks and normalises every field before the
// caller writes any.
func validateSettings(form settingsFields) ([]setting, error) {
	fields := []struct {
		key, raw, label string
		kind            settingKind
	}{
		{"scan_interval", form.scanInterval, "Global scan interval", settingDuration},
		{"nag_interval", form.nagInterval, "Global nag interval", settingDuration},
		{"pings_retention_days", form.pingsRetentionDays, "Pings retention", settingDays},
		{"notifications_retention_days", form.notificationsRetentionDays, "Notifications retention", settingDays},
		{"audit_retention_days", form.auditRetentionDays, "Audit trail retention", settingDays},
		{"display_timezone", form.displayTimezone, "Notification timezone", settingTimezone},
	}
	parsed := make([]setting, 0, len(fields))
	for _, field := range fields {
		value, err := validateSetting(field.raw, field.label, field.kind)
		if err != nil {
			return nil, err
		}
		parsed = append(parsed, setting{field.key, value})
	}
	return parsed, nil
}

func validateSetting(raw, label string, kind settingKind) (string, error) {
	text := trimRustSpace(raw)
	if text == "" {
		return "", nil // blank clears the setting
	}
	switch kind {
	case settingDuration:
		seconds, ok := duration.Parse(text)
		if !ok || seconds <= 0 {
			return "", fmt.Errorf("%s must be a positive duration (e.g. 30, 5m, 1h30m)", label)
		}
		return strconv.FormatInt(seconds, 10), nil
	case settingDays:
		days, err := strconv.ParseInt(text, 10, 64)
		if err != nil || days <= 0 {
			return "", fmt.Errorf("%s must be a positive integer", label)
		}
		return strconv.FormatInt(days, 10), nil
	}
	return validateTimezone(text)
}

// changedSettings is "key=value" for each setting whose stored value differs.
func (d *Deps) changedSettings(ctx context.Context, parsed []setting) ([]string, error) {
	var changed []string
	for _, entry := range parsed {
		previous, err := d.Store.GetSetting(ctx, entry.key)
		if err != nil {
			return nil, err
		}
		if deref(previous) == entry.value {
			continue
		}
		shown := entry.value
		if shown == "" {
			shown = "unset"
		}
		changed = append(changed, entry.key+"="+shown)
	}
	return changed, nil
}
