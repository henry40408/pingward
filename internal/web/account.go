package web

import (
	"net/http"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/henry40408/pingward/internal/apikey"
	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/duration"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/views"
)

// Sessions are named by handle, the SHA-256 of the id, because sessions.id is
// the cookie's bearer secret and is never rendered or put in a URL. A handle is
// resolved back to a row by a linear scan of the caller's own sessions.

// accountParts are the per-request extras of the account page.
type accountParts struct {
	newToken      string
	keyError      string
	passwordError string
	passwordFlash string
}

func (d *Deps) accountPage(w http.ResponseWriter, r *http.Request) {
	user, ok := d.requireUser(w, r)
	if !ok {
		return
	}
	parts := accountParts{}
	if message, taken := d.takeFlash(w, r, "password_changed"); taken {
		parts.passwordFlash = message
	}
	d.renderAccount(w, r, user, parts)
}

func (d *Deps) renderAccount(w http.ResponseWriter, r *http.Request, user *models.User, parts accountParts) {
	view, err := d.accountView(r, user, parts)
	if err != nil {
		internalError(w)
		return
	}
	writeHTML(w, http.StatusOK, views.Account(view))
}

func (d *Deps) accountView(r *http.Request, user *models.User, parts accountParts) (views.AccountView, error) {
	ctx, now := r.Context(), d.Now()
	// Reap past-the-cap rows so "not listed" means "gone".
	if _, err := d.Store.DeleteCappedSessionsForUser(ctx, user.ID, now); err != nil {
		return views.AccountView{}, err
	}
	stored, err := d.Store.ListSessionsForUser(ctx, user.ID, now)
	if err != nil {
		return views.AccountView{}, err
	}
	currentHandle, _ := d.currentSessionHandle(r)
	sessions := sessionRows(stored, currentHandle)
	keys, err := d.Store.ListAPIKeysForUser(ctx, user.ID)
	if err != nil {
		return views.AccountView{}, err
	}
	otherCount := 0
	for _, session := range sessions {
		if !session.Current {
			otherCount++
		}
	}
	return views.AccountView{
		CSRF: d.csrfToken(r), IsAdmin: user.IsAdmin, Sessions: sessions, OtherCount: otherCount,
		HasPassword: user.PasswordHash != nil, PasswordError: parts.passwordError, PasswordFlash: parts.passwordFlash,
		Keys: apiKeyRows(keys, now), NewToken: parts.newToken, KeyError: parts.keyError,
	}, nil
}

// sessionRows puts the current session first and keeps the store's newest-first
// order within each group.
func sessionRows(stored []models.Session, currentHandle string) []views.SessionRow {
	rows := make([]views.SessionRow, 0, len(stored))
	for _, session := range stored {
		handle := apikey.Hash(session.ID)
		rows = append(rows, views.SessionRow{
			Handle: handle, CreatedAt: session.CreatedAt, LastSeenAt: session.LastSeenAt, ExpiresAt: session.ExpiresAt,
			UserAgent: session.UserAgent, IP: session.IP, Current: currentHandle == handle, SSO: session.SSO,
		})
	}
	slices.SortStableFunc(rows, func(a, b views.SessionRow) int {
		switch {
		case a.Current == b.Current:
			return 0
		case a.Current:
			return -1
		default:
			return 1
		}
	})
	return rows
}

func apiKeyRows(keys []models.ApiKey, now time.Time) []views.APIKeyRow {
	rows := make([]views.APIKeyRow, 0, len(keys))
	for _, key := range keys {
		rows = append(rows, views.APIKeyRow{
			ID: key.ID, Name: key.Name, Prefix: key.Prefix, CreatedAt: key.CreatedAt, LastUsedAt: key.LastUsedAt,
			ExpiresAt: key.ExpiresAt, Expired: key.ExpiresAt != nil && !key.ExpiresAt.After(now),
		})
	}
	return rows
}

// accountPassword requires the current password, which a hijacked cookie cannot
// supply. A passwordless forward-auth account is refused: a local password would
// be a way in that the gateway's sign-out could not end. Success revokes every
// other session; API keys survive.
func (d *Deps) accountPassword(w http.ResponseWriter, r *http.Request) {
	user, ok := d.requireUser(w, r)
	if !ok {
		return
	}
	// 403, not a message: the card is hidden, so only a crafted request gets here.
	if user.PasswordHash == nil {
		http.Error(w, "This account has no local password.", http.StatusForbidden)
		return
	}
	refuse := func(message string) {
		d.renderAccount(w, r, user, accountParts{passwordError: message})
	}
	outcome := d.reauthenticate(user, r.PostFormValue("current_password"), "password_change")
	if refusal := reauthRefusal(outcome, "Current password is incorrect."); refusal != "" {
		refuse(refusal)
		return
	}
	newPassword := r.PostFormValue("new_password")
	// Policy before the match check, so the more useful error wins.
	if refusal := auth.ValidatePassword(newPassword); refusal != "" {
		refuse(refusal)
		return
	}
	if newPassword != r.PostFormValue("confirm_password") {
		refuse("The new passwords do not match.")
		return
	}
	d.changePassword(w, r, user, newPassword)
}

func (d *Deps) changePassword(w http.ResponseWriter, r *http.Request, user *models.User, newPassword string) {
	ctx := writeCtx(r)
	hash, err := auth.HashPassword(newPassword)
	if err != nil {
		internalError(w)
		return
	}
	if err := d.Store.SetUserPassword(ctx, user.ID, hash); err != nil {
		internalError(w)
		return
	}
	revoked, keepHandle, err := d.revokeOtherSessions(r, user.ID)
	if err != nil {
		internalError(w)
		return
	}
	d.Elevations.RevokeUser(user.ID, keepHandle)
	d.Logger.Info("session.destroyed", "target", "session", "reason", "password_change", "user_id", user.ID, "count", revoked)
	targetType, detail := "user", "sessions_revoked="+strconv.FormatInt(revoked, 10)
	_, err = d.Store.RecordAudit(ctx, store.NewAudit{
		ActorUserID: user.ID, ActorUsername: user.Username, Action: "user.password_change",
		TargetType: &targetType, TargetID: &user.ID, Detail: &detail,
	}, d.Now())
	if err != nil {
		internalError(w)
		return
	}
	http.SetCookie(w, d.flashCookie("password_changed"))
	redirect(w, r, "/account")
}

// revokeOtherSessions drops every session of userID but the caller's. Without a
// verified cookie it drops all of them, the safe direction. keepHandle is "" in
// that case.
func (d *Deps) revokeOtherSessions(r *http.Request, userID int64) (revoked int64, keepHandle string, err error) {
	id, ok := d.sessionID(r)
	if !ok {
		revoked, err = d.Store.DeleteSessionsForUser(writeCtx(r), userID)
		return revoked, "", err
	}
	revoked, err = d.Store.DeleteOtherSessionsForUser(writeCtx(r), userID, id)
	return revoked, apikey.Hash(id), err
}

func (d *Deps) apiKeyCreate(w http.ResponseWriter, r *http.Request) {
	user, ok := d.requireUser(w, r)
	if !ok {
		return
	}
	refuse := func(message string) {
		d.renderAccount(w, r, user, accountParts{keyError: message})
	}
	// A key escapes both session caps and survives password resets, so a
	// borrowed browser would otherwise buy permanent access.
	outcome := d.reauthenticate(user, r.PostFormValue("current_password"), "api_key_create")
	if refusal := reauthRefusal(outcome, "Current password is incorrect."); refusal != "" {
		refuse(refusal)
		return
	}
	name := trimRustSpace(r.PostFormValue("name"))
	if name == "" {
		refuse("A name is required.")
		return
	}
	expiresAt, ok := d.keyExpiry(trimRustSpace(r.PostFormValue("expires_in")))
	if !ok {
		refuse("Expiry must be a duration like 30d, or blank for never.")
		return
	}
	key := apikey.Generate()
	_, err := d.Store.InsertAPIKey(writeCtx(r), store.NewAPIKey{
		UserID: user.ID, Name: name, TokenHash: key.Hash, Prefix: key.Prefix, ExpiresAt: expiresAt,
	}, d.Now())
	if err != nil {
		internalError(w)
		return
	}
	d.renderAccount(w, r, user, accountParts{newToken: key.Full})
}

// keyExpiry reads blank as never and anything else as a positive duration from
// now.
func (d *Deps) keyExpiry(raw string) (expiresAt *time.Time, ok bool) {
	if raw == "" {
		return nil, true
	}
	secs, parsed := duration.Parse(raw)
	if !parsed || secs <= 0 {
		return nil, false
	}
	at := d.Now().Add(time.Duration(secs) * time.Second)
	return &at, true
}

func (d *Deps) apiKeyDelete(w http.ResponseWriter, r *http.Request) {
	user, ok := d.requireUser(w, r)
	if !ok {
		return
	}
	id, err := strconv.ParseInt(r.PathValue("id"), 10, 64)
	if err != nil {
		http.Error(w, "Invalid ID.", http.StatusBadRequest)
		return
	}
	if !isConfirmed(r) {
		d.confirmationPage(w, r, user.IsAdmin, confirmRevokeAPIKey, "/account/api-keys/"+strconv.FormatInt(id, 10)+"/delete", "/account")
		return
	}
	// Owner-scoped: a key the caller does not own is silently a no-op.
	if _, err := d.Store.DeleteAPIKey(writeCtx(r), id, user.ID); err != nil {
		internalError(w)
		return
	}
	redirect(w, r, "/account")
}

func (d *Deps) sessionRevoke(w http.ResponseWriter, r *http.Request) {
	user, ok := d.requireUser(w, r)
	if !ok {
		return
	}
	handle := r.PathValue("handle")
	sessions, err := d.Store.ListSessionsForUser(r.Context(), user.ID, d.Now())
	if err != nil {
		internalError(w)
		return
	}
	// Owner-scoped: an unknown or foreign handle is a silent no-op, never a 500.
	index := slices.IndexFunc(sessions, func(s models.Session) bool { return apikey.Hash(s.ID) == handle })
	if index < 0 {
		redirect(w, r, "/account")
		return
	}
	// After the lookup, so an unknown handle gets no confirmation page.
	if !isConfirmed(r) {
		d.confirmationPage(w, r, user.IsAdmin, confirmRevokeSession, "/account/sessions/"+handle+"/revoke", "/account")
		return
	}
	d.revokeSession(w, r, user, sessions[index])
}

func (d *Deps) revokeSession(w http.ResponseWriter, r *http.Request, user *models.User, target models.Session) {
	currentID, hasCurrent := d.sessionID(r)
	isCurrent := hasCurrent && currentID == target.ID
	if _, err := d.Store.DeleteSessionOwned(writeCtx(r), target.ID, user.ID); err != nil {
		internalError(w)
		return
	}
	d.Elevations.Revoke(apikey.Hash(target.ID))
	d.Logger.Info("session.destroyed", "target", "session", "reason", "revoked",
		"handle", auth.SessionLogHandle(target.ID), "user_id", user.ID, "is_current", isCurrent)
	if !isCurrent {
		redirect(w, r, "/account")
		return
	}
	// Removal must carry Path=/ like the original, or it clears nothing.
	removal := d.sessionRemovalCookie()
	removal.MaxAge = -1
	http.SetCookie(w, removal)
	redirect(w, r, "/login")
}

func (d *Deps) sessionsRevokeOthers(w http.ResponseWriter, r *http.Request) {
	user, ok := d.requireUser(w, r)
	if !ok {
		return
	}
	if !isConfirmed(r) {
		d.confirmationPage(w, r, user.IsAdmin, confirmRevokeOtherSessions, "/account/sessions/revoke-others", "/account")
		return
	}
	if id, hasSession := d.sessionID(r); hasSession {
		count, err := d.Store.DeleteOtherSessionsForUser(writeCtx(r), user.ID, id)
		if err != nil {
			internalError(w)
			return
		}
		d.Elevations.RevokeUser(user.ID, apikey.Hash(id))
		d.Logger.Info("session.destroyed", "target", "session", "reason", "revoke_others", "user_id", user.ID, "count", count)
	}
	redirect(w, r, "/account")
}

func trimRustSpace(s string) string { return strings.TrimFunc(s, isRustSpace) }
