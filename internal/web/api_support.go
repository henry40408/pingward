package web

import (
	"context"
	"net/http"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
)

// The REST API validates its JSON bodies with the HTML forms' rules, so a
// check or project is accepted or refused the same way through either door.

// ProjectForm is a project body as submitted, before validation.
type ProjectForm struct {
	Name, Description, ScanInterval, NagInterval string
}

// ValidateProject returns the settings to store, or the refusal.
func ValidateProject(form ProjectForm) (store.ProjectSettings, string) {
	return validateProject(projectInput{
		name: form.Name, description: form.Description, scan: form.ScanInterval, nag: form.NagInterval,
	})
}

// CheckForm is a check body as submitted, before validation.
type CheckForm struct {
	Name, Description, ScheduleKind, PeriodSecs, CronExpr string
	GraceSecs, Timezone, ScanInterval, MaxRuntime, Nag    string
}

// ValidateCheck returns every editable field of the check, or the refusal.
func ValidateCheck(form CheckForm) (store.UpdateCheck, string) {
	valid, refusal := validateCheck(checkInput{
		name: form.Name, description: form.Description, scheduleKind: form.ScheduleKind,
		periodSecs: form.PeriodSecs, cronExpr: form.CronExpr, graceSecs: form.GraceSecs,
		timezone: form.Timezone, scanInterval: form.ScanInterval, maxRuntime: form.MaxRuntime, nag: form.Nag,
	})
	if refusal != "" {
		return store.UpdateCheck{}, refusal
	}
	return store.UpdateCheck{
		Name: valid.name, Description: valid.description, Kind: valid.kind, PeriodSecs: valid.periodSecs,
		GraceSecs: valid.graceSecs, CronExpr: valid.cronExpr, Timezone: valid.timezone,
		ScanIntervalSecs: valid.scanInterval, MaxRuntimeSecs: valid.maxRuntime, NagIntervalSecs: valid.nag,
	}, ""
}

// RequireUser redirects to /login when nobody is signed in on the web session.
func (d *Deps) RequireUser(w http.ResponseWriter, r *http.Request) (*models.User, bool) {
	return d.requireUser(w, r)
}

// BindChannels makes the check's bindings equal wanted, honouring only ids of
// the check's own project.
func (d *Deps) BindChannels(ctx context.Context, check models.Check, wanted []int64) error {
	return d.bindChannels(ctx, check, wanted)
}
