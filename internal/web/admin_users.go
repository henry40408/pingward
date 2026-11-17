package web

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"strconv"

	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
)

func usernameTaken(username string) string {
	return fmt.Sprintf("A user named \"%s\" already exists.", username)
}

// renderUserError is the shared re-render for user-management refusals.
func (d *Deps) renderUserError(w http.ResponseWriter, r *http.Request, admin *models.User, message string) {
	settings, err := d.loadSettingsFields(r)
	if err != nil {
		internalError(w)
		return
	}
	d.renderAdmin(w, r, admin, adminNotices{settings: settings, userError: message}, auditQuery{})
}

// auditUser records a user-management action against the target user.
func (d *Deps) auditUser(ctx context.Context, admin *models.User, action string, targetID int64, detail string) error {
	targetType := "user"
	entry := store.NewAudit{ActorUserID: admin.ID, ActorUsername: admin.Username, Action: action, TargetType: &targetType, TargetID: &targetID}
	if detail != "" {
		entry.Detail = &detail
	}
	_, err := d.Store.RecordAudit(ctx, entry, d.Now())
	return err
}

func (d *Deps) logSessionsDestroyed(reason string, userID, actorID int64, count *int64) {
	args := []any{"target", "session", "reason", reason, "user_id", userID}
	if count != nil {
		args = append(args, "count", *count)
	}
	args = append(args, "actor_user_id", actorID)
	d.Logger.Info("session.destroyed", args...)
}

// usersCreate checks run in this order: read-only validation, the duplicate
// pre-check, the elevation gate, then the first write. A doomed submission says
// why instead of demanding confirmation first.
func (d *Deps) usersCreate(w http.ResponseWriter, r *http.Request) {
	admin, ok := d.requireAdmin(w, r)
	if !ok {
		return
	}
	password := r.PostFormValue("password")
	username, refusal := auth.ValidateUsername(r.PostFormValue("username"))
	if refusal == "" {
		refusal = auth.ValidatePassword(password)
	}
	if refusal != "" {
		d.renderUserError(w, r, admin, refusal)
		return
	}
	existing, err := d.Store.FindUserByUsername(r.Context(), username)
	if err != nil {
		internalError(w)
		return
	}
	// Exact match, like the UNIQUE constraint: Admin and admin differ.
	if existing != nil {
		d.renderUserError(w, r, admin, usernameTaken(username))
		return
	}
	// A new account outlives this session. Keep directly above the first write.
	if !d.elevationOf(r, admin).allows() {
		d.adminLocked(w, r)
		return
	}
	d.createUser(w, r, admin, username, password)
}

func (d *Deps) createUser(w http.ResponseWriter, r *http.Request, admin *models.User, username, password string) {
	hash, err := auth.HashPassword(password)
	if err != nil {
		internalError(w)
		return
	}
	// Absent or empty both mean an unchecked box.
	isAdmin := r.PostFormValue("is_admin") != ""
	ctx := writeCtx(r)
	newID, err := d.Store.CreateUser(ctx, username, &hash, isAdmin, d.Now())
	// Two admins can race the pre-check; the constraint arbitrates, same message.
	if errors.Is(err, store.ErrUsernameTaken) {
		d.renderUserError(w, r, admin, usernameTaken(username))
		return
	}
	if err != nil {
		internalError(w)
		return
	}
	detail := "member"
	if isAdmin {
		detail = "admin"
	}
	if err := d.auditUser(ctx, admin, "user.create", newID, detail); err != nil {
		internalError(w)
		return
	}
	redirect(w, r, "/admin")
}

// wouldRemoveLastAdmin is defence in depth: the actor is another enabled admin,
// so it should be unreachable.
func (d *Deps) wouldRemoveLastAdmin(ctx context.Context, target *models.User) (bool, error) {
	if !target.IsAdmin || target.Disabled {
		return false, nil
	}
	enabled, err := d.Store.CountEnabledAdmins(ctx)
	return enabled <= 1, err
}

// userTarget resolves the target of a delete, demote or disable. It answers the
// response itself for the self guard, an unknown id, and the last-admin guard,
// so the caller continues only when removal is permitted.
func (d *Deps) userTarget(w http.ResponseWriter, r *http.Request, admin *models.User, guardLastAdmin func(*models.User) bool) (*models.User, bool) {
	id, ok := pathID(w, r, "id")
	if !ok {
		return nil, false
	}
	if id == admin.ID {
		d.usersBlocked(w, r)
		return nil, false
	}
	target, err := d.Store.FindUserByID(r.Context(), id)
	if err != nil {
		internalError(w)
		return nil, false
	}
	if target == nil {
		redirect(w, r, "/admin")
		return nil, false
	}
	if guardLastAdmin(target) {
		last, err := d.wouldRemoveLastAdmin(r.Context(), target)
		if err != nil {
			internalError(w)
			return nil, false
		}
		if last {
			d.usersBlocked(w, r)
			return nil, false
		}
	}
	return target, true
}

func (d *Deps) usersDelete(w http.ResponseWriter, r *http.Request) {
	admin, ok := d.requireAdmin(w, r)
	if !ok {
		return
	}
	target, ok := d.userTarget(w, r, admin, func(*models.User) bool { return true })
	if !ok {
		return
	}
	// Below both refusals, so a blocked delete is not asked to confirm first.
	if !isConfirmed(r) {
		d.confirmationPage(w, r, true, confirmDeleteUser, userAction(target.ID, "delete"), "/admin")
		return
	}
	ctx := writeCtx(r)
	if err := d.Store.DeleteUser(ctx, target.ID); err != nil {
		internalError(w)
		return
	}
	d.Elevations.RevokeUser(target.ID, "")
	// No count: sessions go via ON DELETE CASCADE.
	d.logSessionsDestroyed("user_deleted", target.ID, admin.ID, nil)
	if err := d.auditUser(ctx, admin, "user.delete", target.ID, ""); err != nil {
		internalError(w)
		return
	}
	redirect(w, r, "/admin")
}

func userAction(id int64, verb string) string {
	return "/admin/users/" + strconv.FormatInt(id, 10) + "/" + verb
}

// usersSetPassword mints a credential this admin knows, outliving the session
// that set it, so the elevation gate runs before anything else.
func (d *Deps) usersSetPassword(w http.ResponseWriter, r *http.Request) {
	admin, ok := d.requireAdmin(w, r)
	if !ok {
		return
	}
	id, ok := pathID(w, r, "id")
	if !ok {
		return
	}
	if !d.elevationOf(r, admin).allows() {
		d.adminLocked(w, r)
		return
	}
	password := r.PostFormValue("password")
	// Rendered, not a silent redirect: an unchanged page would read as success.
	if refusal := auth.ValidatePassword(password); refusal != "" {
		d.renderUserError(w, r, admin, refusal)
		return
	}
	target, err := d.Store.FindUserByID(r.Context(), id)
	if err != nil {
		internalError(w)
		return
	}
	if target == nil {
		redirect(w, r, "/admin")
		return
	}
	d.resetPassword(w, r, admin, target, password)
}

func (d *Deps) resetPassword(w http.ResponseWriter, r *http.Request, admin, target *models.User, password string) {
	hash, err := auth.HashPassword(password)
	if err != nil {
		internalError(w)
		return
	}
	ctx := writeCtx(r)
	if err := d.Store.SetUserPassword(ctx, target.ID, hash); err != nil {
		internalError(w)
		return
	}
	revoked, err := d.endSessionsAfterReset(r, admin, target.ID)
	if err != nil {
		internalError(w)
		return
	}
	d.logSessionsDestroyed("password_reset", target.ID, admin.ID, &revoked)
	if err := d.auditUser(ctx, admin, "user.password_reset", target.ID, "sessions_revoked="+strconv.FormatInt(revoked, 10)); err != nil {
		internalError(w)
		return
	}
	keys, err := d.liveAPIKeyCount(ctx, target.ID)
	if err != nil {
		internalError(w)
		return
	}
	// A disabled account's keys are already inert.
	if keys > 0 && !target.Disabled {
		d.setPasswordResetKeysFlash(w, revoked, keys)
	}
	redirect(w, r, "/admin")
}

// endSessionsAfterReset ends sessions, or an intruder's cookie survives the
// reset. A self-reset spares the current session; evicting an attacker on it
// also needs a logout.
func (d *Deps) endSessionsAfterReset(r *http.Request, admin *models.User, targetID int64) (int64, error) {
	if targetID == admin.ID {
		revoked, keep, err := d.revokeOtherSessions(r, targetID)
		d.Elevations.RevokeUser(targetID, keep)
		return revoked, err
	}
	revoked, err := d.Store.DeleteSessionsForUser(writeCtx(r), targetID)
	d.Elevations.RevokeUser(targetID, "")
	return revoked, err
}

// liveAPIKeyCount skips expired keys: key validation already refuses them.
func (d *Deps) liveAPIKeyCount(ctx context.Context, userID int64) (int64, error) {
	keys, err := d.Store.ListAPIKeysForUser(ctx, userID)
	if err != nil {
		return 0, err
	}
	now, live := d.Now(), int64(0)
	for _, key := range keys {
		if key.ExpiresAt == nil || key.ExpiresAt.After(now) {
			live++
		}
	}
	return live, nil
}

func (d *Deps) usersToggleAdmin(w http.ResponseWriter, r *http.Request) {
	admin, ok := d.requireAdmin(w, r)
	if !ok {
		return
	}
	id, ok := pathID(w, r, "id")
	if !ok {
		return
	}
	// Would lock this admin out of /admin on the very next request.
	if id == admin.ID {
		d.usersBlocked(w, r)
		return
	}
	target, err := d.Store.FindUserByID(r.Context(), id)
	if err != nil {
		internalError(w)
		return
	}
	if target == nil {
		redirect(w, r, "/admin")
		return
	}
	d.setAdmin(w, r, admin, target, !target.IsAdmin)
}

func (d *Deps) setAdmin(w http.ResponseWriter, r *http.Request, admin, target *models.User, promote bool) {
	// Gated only when granting: removing access must not need a password.
	if promote && !d.elevationOf(r, admin).allows() {
		d.adminLocked(w, r)
		return
	}
	if !promote {
		last, err := d.wouldRemoveLastAdmin(r.Context(), target)
		if err != nil {
			internalError(w)
			return
		}
		if last {
			d.usersBlocked(w, r)
			return
		}
		// Demotion is confirmed; promotion already passed the elevation gate.
		if !isConfirmed(r) {
			d.confirmationPage(w, r, true, confirmDemoteAdmin, userAction(target.ID, "admin"), "/admin")
			return
		}
	}
	ctx := writeCtx(r)
	if err := d.Store.SetUserAdmin(ctx, target.ID, promote); err != nil {
		internalError(w)
		return
	}
	detail := "demote"
	if promote {
		detail = "promote"
	}
	if err := d.auditUser(ctx, admin, "user.set_admin", target.ID, detail); err != nil {
		internalError(w)
		return
	}
	redirect(w, r, "/admin")
}

func (d *Deps) usersSetDisabled(w http.ResponseWriter, r *http.Request) {
	admin, ok := d.requireAdmin(w, r)
	if !ok {
		return
	}
	// Only disabling can strand the last admin.
	target, ok := d.userTarget(w, r, admin, func(target *models.User) bool { return !target.Disabled })
	if !ok {
		return
	}
	disable := !target.Disabled
	// Disable only, matching the template: re-enabling needs no ceremony.
	if disable && !isConfirmed(r) {
		d.confirmationPage(w, r, true, confirmDisableUser, userAction(target.ID, "disabled"), "/admin")
		return
	}
	ctx := writeCtx(r)
	if err := d.Store.SetUserDisabled(ctx, target.ID, disable); err != nil {
		internalError(w)
		return
	}
	detail := "enable"
	if disable {
		revoked, err := d.disableSessions(ctx, admin, target.ID)
		if err != nil {
			internalError(w)
			return
		}
		detail = "disable sessions_revoked=" + strconv.FormatInt(revoked, 10)
	}
	if err := d.auditUser(ctx, admin, "user.set_disabled", target.ID, detail); err != nil {
		internalError(w)
		return
	}
	redirect(w, r, "/admin")
}

// disableSessions ends the sessions on disable: otherwise disable then enable
// would resurrect them.
func (d *Deps) disableSessions(ctx context.Context, admin *models.User, userID int64) (int64, error) {
	revoked, err := d.Store.DeleteSessionsForUser(ctx, userID)
	if err != nil {
		return 0, err
	}
	d.Elevations.RevokeUser(userID, "")
	d.logSessionsDestroyed("user_disabled", userID, admin.ID, &revoked)
	return revoked, nil
}
