package web

import (
	"context"
	"fmt"
	"net/http"
	"slices"
	"strconv"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/notify"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/views"
)

func readChannelInput(r *http.Request) ChannelInput {
	field := r.PostFormValue
	return ChannelInput{
		Name: field("name"), Kind: field("kind"),
		WebhookURL: field("webhook_url"), SlackURL: field("slack_url"),
		TelegramToken: field("telegram_token"), TelegramChatID: field("telegram_chat_id"),
		NtfyBaseURL: field("ntfy_base_url"), NtfyTopic: field("ntfy_topic"), NtfyToken: field("ntfy_token"),
		NtfyTokenClear: field("ntfy_token_clear") == "true",
		PushoverToken:  field("pushover_token"), PushoverUser: field("pushover_user"), EmailTo: field("email_to"),
	}
}

// channelEditView is the edit page's only view of a stored config, so a
// secret cannot leak by type.
func channelEditView(channel models.Channel) *views.ChannelEditView {
	stored := parseStoredConfig(&channel)
	value := func(key string) string {
		text, _ := stored[key].(string)
		return text
	}
	return &views.ChannelEditView{
		ID: channel.ID, Kind: string(channel.Kind), Name: channel.Name,
		TelegramChatID: value("chat_id"), NtfyBaseURL: value("base_url"), NtfyTopic: value("topic"), EmailTo: value("to"),
		HasWebhookURL: value("url") != "", HasSlackURL: value("url") != "",
		HasTelegramToken: value("token") != "", HasNtfyToken: value("token") != "",
		HasPushoverToken: value("token") != "", HasPushoverUser: value("user") != "",
	}
}

// channelFormPage is the one form for create and edit; edit is the only difference.
type channelFormPage struct {
	projectID int64
	refusal   string
	edit      *views.ChannelEditView
}

func (d *Deps) writeChannelForm(w http.ResponseWriter, r *http.Request, who access, page channelFormPage) {
	writeHTML(w, http.StatusOK, views.ChannelForm(views.ChannelFormView{
		CSRF: d.csrfToken(r), IsAdmin: who.user.IsAdmin, Admin: who.admin, ProjectID: page.projectID,
		Error: page.refusal, SMTPAvailable: d.Config.SMTP != nil, Edit: page.edit,
	}))
}

func (d *Deps) channelNew(admin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		who, ok := d.enter(w, r, admin)
		if !ok {
			return
		}
		projectID, ok := pathID(w, r, "pid")
		if !ok {
			return
		}
		if _, ok := d.projectFor(w, r, who, projectID); !ok {
			return
		}
		d.writeChannelForm(w, r, who, channelFormPage{projectID: projectID})
	}
}

func (d *Deps) channelCreate(admin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		who, ok := d.enter(w, r, admin)
		if !ok {
			return
		}
		projectID, ok := pathID(w, r, "pid")
		if !ok {
			return
		}
		if _, ok := d.projectFor(w, r, who, projectID); !ok {
			return
		}
		kind, name, config, refusal := ValidateChannel(readChannelInput(r))
		if refusal != "" {
			d.writeChannelForm(w, r, who, channelFormPage{projectID: projectID, refusal: refusal})
			return
		}
		_, err := d.Store.CreateChannel(writeCtx(r), store.NewChannel{
			ProjectID: projectID, Kind: kind, Name: name, ConfigJSON: config,
		}, d.Now())
		if err != nil {
			internalError(w)
			return
		}
		redirect(w, r, fmt.Sprintf("%s/projects/%d", who.base(), projectID))
	}
}

func (d *Deps) channelEdit(admin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		who, ok := d.enter(w, r, admin)
		if !ok {
			return
		}
		id, ok := pathID(w, r, "id")
		if !ok {
			return
		}
		channel, ok := d.channelFor(w, r, who, id)
		if !ok {
			return
		}
		d.writeChannelForm(w, r, who, channelFormPage{projectID: channel.ProjectID, edit: channelEditView(*channel)})
	}
}

func (d *Deps) channelUpdate(admin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		who, ok := d.enter(w, r, admin)
		if !ok {
			return
		}
		id, ok := pathID(w, r, "id")
		if !ok {
			return
		}
		channel, ok := d.channelFor(w, r, who, id)
		if !ok {
			return
		}
		_, name, config, refusal := ValidateChannelUpdate(readChannelInput(r), channel)
		if refusal != "" {
			// From the stored channel, so a typed secret is never echoed back.
			d.writeChannelForm(w, r, who, channelFormPage{projectID: channel.ProjectID, refusal: refusal, edit: channelEditView(*channel)})
			return
		}
		if err := d.Store.UpdateChannel(writeCtx(r), channel.ID, name, config); err != nil {
			internalError(w)
			return
		}
		redirect(w, r, fmt.Sprintf("%s/projects/%d", who.base(), channel.ProjectID))
	}
}

func (d *Deps) channelDelete(admin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		who, ok := d.enter(w, r, admin)
		if !ok {
			return
		}
		id, ok := pathID(w, r, "id")
		if !ok {
			return
		}
		channel, ok := d.channelFor(w, r, who, id)
		if !ok {
			return
		}
		project := fmt.Sprintf("%s/projects/%d", who.base(), channel.ProjectID)
		if !isConfirmed(r) {
			d.confirmationPage(w, r, who.user.IsAdmin, confirmDeleteChannel, fmt.Sprintf("%s/channels/%d/delete", who.base(), id), project)
			return
		}
		if err := d.Store.DeleteChannel(writeCtx(r), id); err != nil {
			internalError(w)
			return
		}
		redirect(w, r, project)
	}
}

// runChannelTest sends once (no retry) and records nothing in the notification history.
func (d *Deps) runChannelTest(ctx context.Context, channel models.Channel) views.TestResult {
	var projectName *string
	if project, err := d.Store.FindProject(ctx, channel.ProjectID); err == nil && project != nil {
		projectName = &project.Name
	}
	var timezone *string
	if zone := d.Store.DisplayTimezone(ctx); zone != "" {
		timezone = &zone
	}
	event := notify.Event{
		CheckName: channel.Name, Kind: models.EventTest, At: d.Now(), ProjectID: channel.ProjectID,
		Detail: notify.EventDetail{ProjectName: projectName}.WithDisplayTimezone(timezone),
	}
	notifier, ok := notify.NotifierFor(channel, d.Config.SMTP)
	if !ok {
		return views.TestResult{Message: "channel configuration is incomplete"}
	}
	if err := notifier.Send(ctx, event); err != nil {
		return views.TestResult{Message: "Test notification failed: " + err.Error()}
	}
	return views.TestResult{OK: true, Message: fmt.Sprintf("Test notification sent to %q", channel.Name)}
}

func (d *Deps) channelTest(admin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		who, ok := d.enter(w, r, admin)
		if !ok {
			return
		}
		id, ok := pathID(w, r, "id")
		if !ok {
			return
		}
		channel, ok := d.channelFor(w, r, who, id)
		if !ok {
			return
		}
		project, err := d.Store.FindProject(r.Context(), channel.ProjectID)
		if err != nil {
			internalError(w)
			return
		}
		if project == nil {
			notFound(w)
			return
		}
		result := d.runChannelTest(r.Context(), *channel)
		d.renderProjectPage(w, r, who, project, &result)
	}
}

// readChannelIDs: a non-numeric id is a 400.
func readChannelIDs(w http.ResponseWriter, r *http.Request) ([]int64, bool) {
	var channelIDs []int64
	for _, raw := range r.PostForm["channel_ids"] {
		id, err := strconv.ParseInt(raw, 10, 64)
		if err != nil {
			http.Error(w, "invalid channel_ids", http.StatusBadRequest)
			return nil, false
		}
		channelIDs = append(channelIDs, id)
	}
	return channelIDs, true
}

// bindChannels makes the check's bindings equal the wanted set; only ids of
// the check's own project are honoured.
func (d *Deps) bindChannels(ctx context.Context, check models.Check, wanted []int64) error {
	channels, err := d.Store.ListChannelsForProject(ctx, check.ProjectID)
	if err != nil {
		return err
	}
	current, err := d.Store.BoundChannelIDs(ctx, check.ID)
	if err != nil {
		return err
	}
	valid := func(id int64) bool {
		return slices.ContainsFunc(channels, func(c models.Channel) bool { return c.ID == id })
	}
	desired := slices.DeleteFunc(slices.Clone(wanted), func(id int64) bool { return !valid(id) })
	for _, id := range desired {
		if !slices.Contains(current, id) {
			if err := d.Store.BindChannel(ctx, check.ID, id); err != nil {
				return err
			}
		}
	}
	for _, id := range current {
		if !slices.Contains(desired, id) {
			if err := d.Store.UnbindChannel(ctx, check.ID, id); err != nil {
				return err
			}
		}
	}
	return nil
}

func (d *Deps) checkSetChannels(admin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		who, ok := d.enter(w, r, admin)
		if !ok {
			return
		}
		id, ok := pathID(w, r, "id")
		if !ok {
			return
		}
		check, ok := d.checkFor(w, r, who, id)
		if !ok {
			return
		}
		if err := r.ParseForm(); err != nil {
			http.Error(w, "invalid form", http.StatusBadRequest)
			return
		}
		wanted, ok := readChannelIDs(w, r)
		if !ok {
			return
		}
		if err := d.bindChannels(writeCtx(r), *check, wanted); err != nil {
			internalError(w)
			return
		}
		http.SetCookie(w, d.flashCookie("channels"))
		redirect(w, r, fmt.Sprintf("%s/checks/%d", who.base(), id))
	}
}
