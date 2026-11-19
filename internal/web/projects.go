package web

import (
	"fmt"
	"github.com/henry40408/pingward/internal/markdown"
	"net/http"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/henry40408/pingward/internal/duration"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/view"
	"github.com/henry40408/pingward/internal/views"
)

const maxDescriptionChars = 2000

// validateDescription trims, then enforces maxDescriptionChars.
func validateDescription(raw string) (string, string) {
	trimmed := strings.TrimFunc(raw, isRustSpace)
	if utf8.RuneCountInString(trimmed) > maxDescriptionChars {
		return "", fmt.Sprintf("Description must be at most %d characters.", maxDescriptionChars)
	}
	return trimmed, ""
}

// parseOptionalDuration treats blank as unset (inherit or off); refusal names the field.
func parseOptionalDuration(raw, field string) (value *int64, refusal string) {
	trimmed := strings.TrimFunc(raw, isRustSpace)
	if trimmed == "" {
		return nil, ""
	}
	seconds, ok := duration.Parse(trimmed)
	if !ok || seconds <= 0 {
		return nil, field + " must be a positive duration (e.g. 30, 5m, 1h30m)."
	}
	return &seconds, ""
}

// projectInput is a project form as submitted, kept verbatim so a refusal can
// re-render it.
type projectInput struct {
	name, description, scan, nag string
}

func readProjectInput(r *http.Request) projectInput {
	return projectInput{
		name: r.PostFormValue("name"), description: r.PostFormValue("description"),
		scan: r.PostFormValue("scan_interval_secs"), nag: r.PostFormValue("nag_interval_secs"),
	}
}

func validateProject(in projectInput) (store.ProjectSettings, string) {
	name := strings.TrimFunc(in.name, isRustSpace)
	if name == "" {
		return store.ProjectSettings{}, "Name is required."
	}
	description, refusal := validateDescription(in.description)
	if refusal != "" {
		return store.ProjectSettings{}, refusal
	}
	scan, refusal := parseOptionalDuration(in.scan, "Scan interval")
	if refusal != "" {
		return store.ProjectSettings{}, refusal
	}
	nag, refusal := parseOptionalDuration(in.nag, "Nag interval")
	if refusal != "" {
		return store.ProjectSettings{}, refusal
	}
	return store.ProjectSettings{Name: name, Description: description, ScanIntervalSecs: scan, NagIntervalSecs: nag}, ""
}

type formTarget struct{ heading, action string }

func (d *Deps) projectForm(r *http.Request, user *models.User, in projectInput, refusal string) string {
	return d.projectFormAt(r, user, formTarget{"New project", "/projects"}, in, refusal)
}

func (d *Deps) projectFormAt(r *http.Request, user *models.User, target formTarget, in projectInput, refusal string) string {
	return views.ProjectForm(views.ProjectFormView{
		CSRF: d.csrfToken(r), IsAdmin: user.IsAdmin, Heading: target.heading, Action: target.action,
		Name: in.name, Description: in.description, ScanIntervalSecs: in.scan, NagIntervalSecs: in.nag,
		Error: refusal,
	})
}

func (d *Deps) projectNew(w http.ResponseWriter, r *http.Request) {
	user, ok := d.requireUser(w, r)
	if !ok {
		return
	}
	writeHTML(w, http.StatusOK, d.projectForm(r, user, projectInput{}, ""))
}

func (d *Deps) projectCreate(w http.ResponseWriter, r *http.Request) {
	user, ok := d.requireUser(w, r)
	if !ok {
		return
	}
	in := readProjectInput(r)
	settings, refusal := validateProject(in)
	if refusal != "" {
		writeHTML(w, http.StatusOK, d.projectForm(r, user, in, refusal))
		return
	}
	id, err := d.Store.CreateProject(writeCtx(r), user.ID, settings, d.Now())
	if err != nil {
		internalError(w)
		return
	}
	redirect(w, r, fmt.Sprintf("/projects/%d", id))
}

// isRustSpace matches Rust's char::is_whitespace, which str::trim uses.
func isRustSpace(r rune) bool { return unicode.IsSpace(r) || unicode.Is(unicode.White_Space, r) }

func optionalDuration(seconds *int64) string {
	if seconds == nil {
		return ""
	}
	return duration.Format(*seconds)
}

func (d *Deps) renderProjectPage(w http.ResponseWriter, r *http.Request, who access, project *models.Project, test *views.TestResult) {
	ctx := r.Context()
	now := d.Now()
	checks, err := d.Store.ListChecksForProject(ctx, project.ID)
	if err != nil {
		internalError(w)
		return
	}
	checkIDs := make([]int64, len(checks))
	for i, check := range checks {
		checkIDs[i] = check.ID
	}
	withChannels, err := d.Store.ChecksWithChannels(ctx, checkIDs)
	if err != nil {
		internalError(w)
		return
	}
	channels, err := d.Store.ListChannelsForProject(ctx, project.ID)
	if err != nil {
		internalError(w)
		return
	}
	rows := make([]views.ProjectCheckRow, len(checks))
	for i, check := range checks {
		rows[i] = views.ProjectCheckRow{
			ID: check.ID, Name: check.Name, Status: string(view.Status(check, now)), Schedule: scheduleLabel(check),
			Description: markdown.TruncatePlain(check.Description, dashboardSummaryChars), NoChannel: !withChannels[check.ID],
		}
	}
	channelRows := make([]views.ProjectChannelRow, len(channels))
	for i, channel := range channels {
		channelRows[i] = views.ProjectChannelRow{ID: channel.ID, Name: channel.Name, Kind: string(channel.Kind)}
	}
	writeHTML(w, http.StatusOK, views.Project(views.ProjectView{
		CSRF: d.csrfToken(r), IsAdmin: who.user.IsAdmin, Admin: who.admin, Project: *project,
		DescriptionHTML: markdown.Render(project.Description), Checks: rows, Channels: channelRows, TestResult: test,
	}))
}

func (d *Deps) projectShow(admin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		who, ok := d.enter(w, r, admin)
		if !ok {
			return
		}
		id, ok := pathID(w, r, "id")
		if !ok {
			return
		}
		project, ok := d.projectFor(w, r, who, id)
		if !ok {
			return
		}
		d.renderProjectPage(w, r, who, project, nil)
	}
}

func (d *Deps) projectEdit(admin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		who, ok := d.enter(w, r, admin)
		if !ok {
			return
		}
		id, ok := pathID(w, r, "id")
		if !ok {
			return
		}
		project, ok := d.projectFor(w, r, who, id)
		if !ok {
			return
		}
		in := projectInput{
			name: project.Name, description: project.Description,
			scan: optionalDuration(project.ScanIntervalSecs), nag: optionalDuration(project.NagIntervalSecs),
		}
		target := formTarget{"Edit project", fmt.Sprintf("%s/projects/%d", who.base(), project.ID)}
		writeHTML(w, http.StatusOK, d.projectFormAt(r, who.user, target, in, ""))
	}
}

func (d *Deps) projectUpdate(admin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		who, ok := d.enter(w, r, admin)
		if !ok {
			return
		}
		id, ok := pathID(w, r, "id")
		if !ok {
			return
		}
		if _, ok := d.projectFor(w, r, who, id); !ok {
			return
		}
		in := readProjectInput(r)
		settings, refusal := validateProject(in)
		if refusal != "" {
			target := formTarget{"Edit project", fmt.Sprintf("%s/projects/%d", who.base(), id)}
			writeHTML(w, http.StatusOK, d.projectFormAt(r, who.user, target, in, refusal))
			return
		}
		if err := d.Store.UpdateProject(writeCtx(r), id, settings); err != nil {
			internalError(w)
			return
		}
		redirect(w, r, fmt.Sprintf("%s/projects/%d", who.base(), id))
	}
}

func (d *Deps) projectDelete(admin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		who, ok := d.enter(w, r, admin)
		if !ok {
			return
		}
		id, ok := pathID(w, r, "id")
		if !ok {
			return
		}
		if _, ok := d.projectFor(w, r, who, id); !ok {
			return
		}
		show := fmt.Sprintf("%s/projects/%d", who.base(), id)
		if !isConfirmed(r) {
			d.confirmationPage(w, r, who.user.IsAdmin, confirmDeleteProject, show+"/delete", show)
			return
		}
		if err := d.Store.DeleteProject(writeCtx(r), id); err != nil {
			internalError(w)
			return
		}
		if who.admin {
			redirect(w, r, "/admin")
			return
		}
		redirect(w, r, "/")
	}
}
