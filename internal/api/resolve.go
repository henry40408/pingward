package api

import (
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
)

// resolveProject, resolveCheck and resolveChannel are the ownership choke
// point: 404 for another user's resource, whether it exists or not. An admin
// key gets through, and every such access is audited, reads included.

func (c *call) resolveProject(id int64) (*models.Project, bool) {
	project, err := c.a.d.Store.FindProject(c.r.Context(), id)
	if err != nil {
		c.a.internalError(c.w, err)
		return nil, false
	}
	if project == nil {
		notFound(c.w)
		return nil, false
	}
	if !c.admitted(project.UserID == c.user.ID, "project", project.ID, &project.UserID) {
		return nil, false
	}
	return project, true
}

func (c *call) resolveCheck(id int64) (*models.Check, bool) {
	check, err := c.a.d.Store.FindCheck(c.r.Context(), id)
	if err != nil {
		c.a.internalError(c.w, err)
		return nil, false
	}
	if check == nil {
		notFound(c.w)
		return nil, false
	}
	owner, ok := c.projectOwner(check.ProjectID)
	if !ok {
		return nil, false
	}
	if !c.admitted(owner != nil && *owner == c.user.ID, "check", check.ID, owner) {
		return nil, false
	}
	return check, true
}

func (c *call) resolveChannel(id int64) (*models.Channel, bool) {
	channel, err := c.a.d.Store.FindChannel(c.r.Context(), id)
	if err != nil {
		c.a.internalError(c.w, err)
		return nil, false
	}
	if channel == nil {
		notFound(c.w)
		return nil, false
	}
	owner, ok := c.projectOwner(channel.ProjectID)
	if !ok {
		return nil, false
	}
	if !c.admitted(owner != nil && *owner == c.user.ID, "channel", channel.ID, owner) {
		return nil, false
	}
	return channel, true
}

// projectOwner is nil when the project is gone.
func (c *call) projectOwner(projectID int64) (*int64, bool) {
	project, err := c.a.d.Store.FindProject(c.r.Context(), projectID)
	if err != nil {
		c.a.internalError(c.w, err)
		return nil, false
	}
	if project == nil {
		return nil, true
	}
	return &project.UserID, true
}

// admitted lets the owner through, and an admin through with an audit entry;
// anyone else gets a 404 that looks like a missing resource.
func (c *call) admitted(isOwner bool, targetType string, targetID int64, owner *int64) bool {
	if isOwner {
		return true
	}
	if !c.user.IsAdmin {
		notFound(c.w)
		return false
	}
	method, path := c.r.Method, c.r.URL.Path
	_, err := c.a.d.Store.RecordAudit(c.writeCtx(), store.NewAudit{
		ActorUserID: c.user.ID, ActorUsername: c.user.Username, Action: "admin.api.access",
		TargetType: &targetType, TargetID: &targetID, TargetOwnerID: owner, Method: &method, Path: &path,
	}, c.a.d.Now())
	if err != nil {
		c.a.internalError(c.w, err)
		return false
	}
	return true
}
