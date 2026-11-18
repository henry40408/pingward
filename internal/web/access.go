package web

import (
	"net/http"
	"strconv"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
)

// access is who is acting and through which door. An admin door reaches any
// user's resources and audits what it changes; an owner door reaches only the
// caller's own, and answers 404 for the rest so existence stays hidden.
type access struct {
	user  *models.User
	admin bool
}

// base is the route prefix every link on an admin page carries.
func (a access) base() string {
	if a.admin {
		return "/admin"
	}
	return ""
}

// enter authenticates the caller for the given door.
func (d *Deps) enter(w http.ResponseWriter, r *http.Request, admin bool) (access, bool) {
	if admin {
		user, ok := d.requireAdmin(w, r)
		return access{user: user, admin: true}, ok
	}
	user, ok := d.requireUser(w, r)
	return access{user: user}, ok
}

// pathID reads a numeric path segment; a non-number is 400 like axum's Path<i64>.
func pathID(w http.ResponseWriter, r *http.Request, name string) (int64, bool) {
	id, err := strconv.ParseInt(r.PathValue(name), 10, 64)
	if err != nil {
		http.Error(w, "invalid "+name, http.StatusBadRequest)
		return 0, false
	}
	return id, true
}

// auditsAsMutation: page opens would bury what matters, and the credential
// read audits itself as admin.ping_url_reveal.
func auditsAsMutation(method string) bool { return method != http.MethodGet }

// auditAdminAccess records an admin's non-GET touch of another user's resource.
func (d *Deps) auditAdminAccess(r *http.Request, admin *models.User, targetType string, targetID int64, ownerID *int64) error {
	if !auditsAsMutation(r.Method) {
		return nil
	}
	method, path := r.Method, r.URL.Path
	_, err := d.Store.RecordAudit(r.Context(), store.NewAudit{
		ActorUserID: admin.ID, ActorUsername: admin.Username, Action: "admin.access",
		TargetType: &targetType, TargetID: &targetID, TargetOwnerID: ownerID,
		Method: &method, Path: &path,
	}, d.Now())
	return err
}

// projectFor resolves a project for the caller: 404 when missing, or (owner
// door) when not theirs.
func (d *Deps) projectFor(w http.ResponseWriter, r *http.Request, who access, id int64) (*models.Project, bool) {
	project, err := d.Store.FindProject(r.Context(), id)
	if err != nil {
		internalError(w)
		return nil, false
	}
	if project == nil || (!who.admin && project.UserID != who.user.ID) {
		notFound(w)
		return nil, false
	}
	if who.admin {
		if err := d.auditAdminAccess(r, who.user, "project", project.ID, &project.UserID); err != nil {
			internalError(w)
			return nil, false
		}
	}
	return project, true
}

// checkFor resolves a check via its project, as projectFor does.
func (d *Deps) checkFor(w http.ResponseWriter, r *http.Request, who access, id int64) (*models.Check, bool) {
	check, err := d.Store.FindCheck(r.Context(), id)
	if err != nil {
		internalError(w)
		return nil, false
	}
	if check == nil {
		notFound(w)
		return nil, false
	}
	owner, ok := d.projectOwner(w, r, who, check.ProjectID)
	if !ok {
		return nil, false
	}
	if who.admin {
		if err := d.auditAdminAccess(r, who.user, "check", check.ID, owner); err != nil {
			internalError(w)
			return nil, false
		}
	}
	return check, true
}

// channelFor resolves a channel via its project, as projectFor does.
func (d *Deps) channelFor(w http.ResponseWriter, r *http.Request, who access, id int64) (*models.Channel, bool) {
	channel, err := d.Store.FindChannel(r.Context(), id)
	if err != nil {
		internalError(w)
		return nil, false
	}
	if channel == nil {
		notFound(w)
		return nil, false
	}
	owner, ok := d.projectOwner(w, r, who, channel.ProjectID)
	if !ok {
		return nil, false
	}
	if who.admin {
		if err := d.auditAdminAccess(r, who.user, "channel", channel.ID, owner); err != nil {
			internalError(w)
			return nil, false
		}
	}
	return channel, true
}

// projectOwner is the owner id of a child's project. A missing project is
// 404 for the owner door and an unknown owner for the admin door, as in Rust.
func (d *Deps) projectOwner(w http.ResponseWriter, r *http.Request, who access, projectID int64) (*int64, bool) {
	project, err := d.Store.FindProject(r.Context(), projectID)
	if err != nil {
		internalError(w)
		return nil, false
	}
	if who.admin {
		if project == nil {
			return nil, true
		}
		return &project.UserID, true
	}
	if project == nil || project.UserID != who.user.ID {
		notFound(w)
		return nil, false
	}
	return &project.UserID, true
}
