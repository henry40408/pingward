package api

import (
	"net/http"

	"github.com/henry40408/pingward/internal/ids"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/web"
)

// Every write handler reads its body before it resolves the resource, so a
// malformed body is a 400 for an existing and a missing resource alike; the
// caller was authenticated first. Validation comes last, behind ownership.

func readProjectForm(f *fields) web.ProjectForm {
	return web.ProjectForm{
		Name: f.required("name"), Description: f.optionalText("description"),
		ScanInterval: f.duration("scan_interval_secs", ""), NagInterval: f.duration("nag_interval_secs", ""),
	}
}

// readCheckForm: an omitted grace is none, not blank, which validation refuses.
func readCheckForm(f *fields) web.CheckForm {
	return web.CheckForm{
		Name: f.required("name"), Description: f.optionalText("description"),
		ScheduleKind: f.text("schedule_kind", "period"), PeriodSecs: f.duration("period_secs", ""),
		CronExpr: f.optionalText("cron_expr"), GraceSecs: f.duration("grace_secs", "0"),
		Timezone: f.text("timezone", "UTC"), ScanInterval: f.duration("scan_interval_secs", ""),
		MaxRuntime: f.duration("max_runtime_secs", ""), Nag: f.duration("nag_interval_secs", ""),
	}
}

func readChannelForm(f *fields) web.ChannelInput {
	return web.ChannelInput{
		Name: f.text("name", ""), Kind: f.text("kind", ""),
		WebhookURL: f.text("webhook_url", ""), SlackURL: f.text("slack_url", ""),
		TelegramToken: f.text("telegram_token", ""), TelegramChatID: f.text("telegram_chat_id", ""),
		NtfyBaseURL: f.text("ntfy_base_url", ""), NtfyTopic: f.text("ntfy_topic", ""),
		NtfyToken: f.text("ntfy_token", ""), NtfyTokenClear: f.flag("ntfy_token_clear"),
		PushoverToken: f.text("pushover_token", ""), PushoverUser: f.text("pushover_user", ""),
		EmailTo: f.text("email_to", ""),
	}
}

// body reads the JSON body into a form with read, or answers 400.
func body[T any](c *call, read func(*fields) T) (T, bool) {
	var zero T
	f, ok := readFields(c.w, c.r)
	if !ok {
		return zero, false
	}
	form := read(f)
	if refusal := f.refusal(); refusal != "" {
		badRequest(c.w, refusal)
		return zero, false
	}
	return form, true
}

func (a *API) createProject(c *call) {
	form, ok := body(c, readProjectForm)
	if !ok {
		return
	}
	settings, refusal := web.ValidateProject(form)
	if refusal != "" {
		badRequest(c.w, refusal)
		return
	}
	id, err := a.d.Store.CreateProject(c.writeCtx(), c.user.ID, settings, a.d.Now())
	if err != nil {
		a.internalError(c.w, err)
		return
	}
	a.writeProject(c, http.StatusCreated, id)
}

func (a *API) updateProject(c *call) {
	form, ok := body(c, readProjectForm)
	if !ok {
		return
	}
	if _, ok := c.resolveProject(c.id); !ok {
		return
	}
	settings, refusal := web.ValidateProject(form)
	if refusal != "" {
		badRequest(c.w, refusal)
		return
	}
	if err := a.d.Store.UpdateProject(c.writeCtx(), c.id, settings); err != nil {
		a.internalError(c.w, err)
		return
	}
	a.writeProject(c, http.StatusOK, c.id)
}

func (a *API) writeProject(c *call, status int, id int64) {
	project, err := a.d.Store.FindProject(c.r.Context(), id)
	if err != nil {
		a.internalError(c.w, err)
		return
	}
	if project == nil {
		notFound(c.w)
		return
	}
	writeJSON(c.w, status, projectDTO(*project))
}

func (a *API) deleteProject(c *call) {
	if _, ok := c.resolveProject(c.id); !ok {
		return
	}
	if err := a.d.Store.DeleteProject(c.writeCtx(), c.id); err != nil {
		a.internalError(c.w, err)
		return
	}
	c.w.WriteHeader(http.StatusNoContent)
}

func (a *API) createCheck(c *call) {
	form, ok := body(c, readCheckForm)
	if !ok {
		return
	}
	project, ok := c.resolveProject(c.id)
	if !ok {
		return
	}
	valid, refusal := web.ValidateCheck(form)
	if refusal != "" {
		badRequest(c.w, refusal)
		return
	}
	ctx := c.writeCtx()
	id, err := a.d.Store.CreateCheck(ctx, store.NewCheck{
		ProjectID: project.ID, Name: valid.Name, Description: valid.Description, PingUUID: ids.NewV4(),
		Kind: valid.Kind, PeriodSecs: valid.PeriodSecs, GraceSecs: valid.GraceSecs, CronExpr: valid.CronExpr,
		Timezone: valid.Timezone, ScanIntervalSecs: valid.ScanIntervalSecs, MaxRuntimeSecs: valid.MaxRuntimeSecs,
		NagIntervalSecs: valid.NagIntervalSecs,
	}, a.d.Now())
	if err == nil {
		err = a.d.Store.BindAllProjectChannels(ctx, id, project.ID)
	}
	if err != nil {
		a.internalError(c.w, err)
		return
	}
	a.writeCheck(c, http.StatusCreated, id)
}

func (a *API) updateCheck(c *call) {
	form, ok := body(c, readCheckForm)
	if !ok {
		return
	}
	if _, ok := c.resolveCheck(c.id); !ok {
		return
	}
	valid, refusal := web.ValidateCheck(form)
	if refusal != "" {
		badRequest(c.w, refusal)
		return
	}
	if err := a.d.Store.UpdateCheckSchedule(c.writeCtx(), c.id, valid); err != nil {
		a.internalError(c.w, err)
		return
	}
	a.writeCheck(c, http.StatusOK, c.id)
}

func (a *API) writeCheck(c *call, status int, id int64) {
	check, err := a.d.Store.FindCheck(c.r.Context(), id)
	if err != nil {
		a.internalError(c.w, err)
		return
	}
	if check == nil {
		notFound(c.w)
		return
	}
	writeJSON(c.w, status, checkDTO(*check))
}

func (a *API) deleteCheck(c *call) {
	if _, ok := c.resolveCheck(c.id); !ok {
		return
	}
	if err := a.d.Store.DeleteCheck(c.writeCtx(), c.id); err != nil {
		a.internalError(c.w, err)
		return
	}
	c.w.WriteHeader(http.StatusNoContent)
}

// checkAction is a one-step state change of a check that answers with the check.
func (a *API) checkAction(apply func(c *call) error) func(*call) {
	return func(c *call) {
		if _, ok := c.resolveCheck(c.id); !ok {
			return
		}
		if err := apply(c); err != nil {
			a.internalError(c.w, err)
			return
		}
		a.writeCheck(c, http.StatusOK, c.id)
	}
}

func (a *API) pauseCheck() func(*call) {
	return a.checkAction(func(c *call) error {
		return a.d.Store.SetStatus(c.writeCtx(), c.id, models.StatusPaused)
	})
}

func (a *API) resumeCheck() func(*call) {
	return a.checkAction(func(c *call) error {
		return a.d.Store.SetStatus(c.writeCtx(), c.id, models.StatusNew)
	})
}

func (a *API) ackCheck() func(*call) {
	return a.checkAction(func(c *call) error {
		return a.d.Store.Acknowledge(c.writeCtx(), c.id)
	})
}

func (a *API) regenerateCheck() func(*call) {
	return a.checkAction(func(c *call) error {
		return a.d.Store.RegenerateUUID(c.writeCtx(), c.id, ids.NewV4())
	})
}

func (a *API) setCheckChannels(c *call) {
	wanted, ok := body(c, func(f *fields) []int64 { return f.ids("channel_ids") })
	if !ok {
		return
	}
	check, ok := c.resolveCheck(c.id)
	if !ok {
		return
	}
	if err := a.d.BindChannels(c.writeCtx(), *check, wanted); err != nil {
		a.internalError(c.w, err)
		return
	}
	bound, err := a.d.Store.BoundChannelIDs(c.r.Context(), c.id)
	if err != nil {
		a.internalError(c.w, err)
		return
	}
	if bound == nil {
		bound = []int64{}
	}
	writeJSON(c.w, http.StatusOK, BoundChannels{ChannelIDs: bound})
}

func (a *API) createChannel(c *call) {
	form, ok := body(c, readChannelForm)
	if !ok {
		return
	}
	project, ok := c.resolveProject(c.id)
	if !ok {
		return
	}
	kind, name, config, refusal := web.ValidateChannel(form)
	if refusal != "" {
		badRequest(c.w, refusal)
		return
	}
	id, err := a.d.Store.CreateChannel(c.writeCtx(), store.NewChannel{
		ProjectID: project.ID, Kind: kind, Name: name, ConfigJSON: config,
	}, a.d.Now())
	if err != nil {
		a.internalError(c.w, err)
		return
	}
	a.writeChannel(c, http.StatusCreated, id)
}

// updateChannel merges: a blank field keeps the stored value, and the kind is immutable.
func (a *API) updateChannel(c *call) {
	form, ok := body(c, readChannelForm)
	if !ok {
		return
	}
	existing, ok := c.resolveChannel(c.id)
	if !ok {
		return
	}
	_, name, config, refusal := web.ValidateChannelUpdate(form, existing)
	if refusal != "" {
		badRequest(c.w, refusal)
		return
	}
	if err := a.d.Store.UpdateChannel(c.writeCtx(), c.id, name, config); err != nil {
		a.internalError(c.w, err)
		return
	}
	a.writeChannel(c, http.StatusOK, c.id)
}

func (a *API) writeChannel(c *call, status int, id int64) {
	channel, err := a.d.Store.FindChannel(c.r.Context(), id)
	if err != nil {
		a.internalError(c.w, err)
		return
	}
	if channel == nil {
		notFound(c.w)
		return
	}
	writeJSON(c.w, status, channelDTO(*channel))
}

func (a *API) deleteChannel(c *call) {
	if _, ok := c.resolveChannel(c.id); !ok {
		return
	}
	if err := a.d.Store.DeleteChannel(c.writeCtx(), c.id); err != nil {
		a.internalError(c.w, err)
		return
	}
	c.w.WriteHeader(http.StatusNoContent)
}
