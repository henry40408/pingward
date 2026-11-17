package web

import (
	"fmt"
	"net/http"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/henry40408/pingward/internal/duration"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/views"
)

const maxDescriptionChars = 2000

// validateDescription trims, then enforces maxDescriptionChars.
func validateDescription(raw string) (string, string) {
	trimmed := strings.TrimFunc(raw, isRustSpace)
	if utf8.RuneCountInString(trimmed) > maxDescriptionChars {
		return "", fmt.Sprintf("description must be at most %d characters", maxDescriptionChars)
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
		return nil, field + " must be a positive duration (e.g. 30, 5m, 1h30m)"
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
		return store.ProjectSettings{}, "name is required"
	}
	description, refusal := validateDescription(in.description)
	if refusal != "" {
		return store.ProjectSettings{}, refusal
	}
	scan, refusal := parseOptionalDuration(in.scan, "scan interval")
	if refusal != "" {
		return store.ProjectSettings{}, refusal
	}
	nag, refusal := parseOptionalDuration(in.nag, "nag interval")
	if refusal != "" {
		return store.ProjectSettings{}, refusal
	}
	return store.ProjectSettings{Name: name, Description: description, ScanIntervalSecs: scan, NagIntervalSecs: nag}, ""
}

func (d *Deps) projectForm(r *http.Request, user *models.User, in projectInput, refusal string) string {
	return views.ProjectForm(views.ProjectFormView{
		CSRF: d.csrfToken(r), IsAdmin: user.IsAdmin, Heading: "New project", Action: "/projects",
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
	id, err := d.Store.CreateProject(r.Context(), user.ID, settings, d.Now())
	if err != nil {
		internalError(w)
		return
	}
	redirect(w, r, fmt.Sprintf("/projects/%d", id))
}

// isRustSpace matches Rust's char::is_whitespace, which str::trim uses.
func isRustSpace(r rune) bool { return unicode.IsSpace(r) || unicode.Is(unicode.White_Space, r) }
