package api

import (
	"fmt"
	"net/http"
	"net/url"
	"strconv"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
)

const (
	defaultPageLimit = 20
	maxPageLimit     = 100
)

type pageParams struct {
	cursor store.PageCursor
	limit  int64
}

// readPageParams reads ?before, ?after and ?limit. before wins over after; the
// limit is clamped to 1..100. A value that is not an integer is a 400 in plain
// text, as axum's Query rejection was.
func readPageParams(w http.ResponseWriter, query url.Values) (pageParams, bool) {
	params, err := parsePageParams(query)
	if err != nil {
		plainText(w, http.StatusBadRequest, "Failed to deserialize query string: "+err.Error())
		return pageParams{}, false
	}
	return params, true
}

func parsePageParams(query url.Values) (pageParams, error) {
	before, err := queryInt(query, "before")
	if err != nil {
		return pageParams{}, err
	}
	after, err := queryInt(query, "after")
	if err != nil {
		return pageParams{}, err
	}
	limit, err := queryInt(query, "limit")
	if err != nil {
		return pageParams{}, err
	}
	return pageParamsOf(before, after, limit), nil
}

func pageParamsOf(before, after, limit *int64) pageParams {
	params := pageParams{limit: defaultPageLimit}
	if limit != nil {
		params.limit = min(max(*limit, 1), maxPageLimit)
	}
	switch {
	case before != nil:
		params.cursor = store.Before(*before)
	case after != nil:
		params.cursor = store.After(*after)
	}
	return params
}

// queryInt is nil when the parameter is absent.
func queryInt(query url.Values, name string) (*int64, error) {
	values := query[name]
	if len(values) == 0 {
		return nil, nil
	}
	if len(values) > 1 {
		return nil, fmt.Errorf("duplicate field `%s`", name)
	}
	number, err := strconv.ParseInt(values[0], 10, 64)
	if err != nil {
		return nil, fmt.Errorf("%s: invalid digit found in string", name)
	}
	return &number, nil
}

func (a *API) listProjects(c *call) {
	projects, err := a.d.Store.ListProjectsForUser(c.r.Context(), c.user.ID)
	if err != nil {
		a.internalError(c.w, err)
		return
	}
	writeJSON(c.w, http.StatusOK, list(projects, projectDTO))
}

func (a *API) getProject(c *call) {
	project, ok := c.resolveProject(c.id)
	if !ok {
		return
	}
	writeJSON(c.w, http.StatusOK, projectDTO(*project))
}

func (a *API) listProjectChecks(c *call) {
	project, ok := c.resolveProject(c.id)
	if !ok {
		return
	}
	checks, err := a.d.Store.ListChecksForProject(c.r.Context(), project.ID)
	if err != nil {
		a.internalError(c.w, err)
		return
	}
	writeJSON(c.w, http.StatusOK, list(checks, checkDTO))
}

func (a *API) listProjectChannels(c *call) {
	project, ok := c.resolveProject(c.id)
	if !ok {
		return
	}
	channels, err := a.d.Store.ListChannelsForProject(c.r.Context(), project.ID)
	if err != nil {
		a.internalError(c.w, err)
		return
	}
	writeJSON(c.w, http.StatusOK, list(channels, channelDTO))
}

func (a *API) getCheck(c *call) {
	check, ok := c.resolveCheck(c.id)
	if !ok {
		return
	}
	writeJSON(c.w, http.StatusOK, checkDTO(*check))
}

func (a *API) listCheckPings(c *call) {
	params, ok := readPageParams(c.w, c.r.URL.Query())
	if !ok {
		return
	}
	check, ok := c.resolveCheck(c.id)
	if !ok {
		return
	}
	page, err := a.d.Store.ListPingsPage(c.r.Context(), check.ID, params.cursor, params.limit, store.PingFilter{})
	if err != nil {
		a.internalError(c.w, err)
		return
	}
	writeJSON(c.w, http.StatusOK, newPage(page, func(p models.Ping) int64 { return p.ID }, pingDTO))
}

func (a *API) listCheckNotifications(c *call) {
	params, ok := readPageParams(c.w, c.r.URL.Query())
	if !ok {
		return
	}
	check, ok := c.resolveCheck(c.id)
	if !ok {
		return
	}
	page, err := a.d.Store.ListNotificationsPage(c.r.Context(), check.ID, params.cursor, params.limit, store.NotifFilter{})
	if err != nil {
		a.internalError(c.w, err)
		return
	}
	writeJSON(c.w, http.StatusOK, newPage(page, func(n models.Notification) int64 { return n.ID }, notificationDTO))
}

func (a *API) getChannel(c *call) {
	channel, ok := c.resolveChannel(c.id)
	if !ok {
		return
	}
	writeJSON(c.w, http.StatusOK, channelDTO(*channel))
}

func (a *API) listKeys(c *call) {
	keys, err := a.d.Store.ListAPIKeysForUser(c.r.Context(), c.user.ID)
	if err != nil {
		a.internalError(c.w, err)
		return
	}
	writeJSON(c.w, http.StatusOK, list(keys, apiKeyDTO))
}
