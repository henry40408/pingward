package web

import (
	"fmt"
	"net/http"
	"strings"

	"github.com/henry40408/pingward/internal/cron"
	"github.com/henry40408/pingward/internal/duration"
	"github.com/henry40408/pingward/internal/ids"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/tz"
	"github.com/henry40408/pingward/internal/views"
)

// checkInput is a check form as submitted, kept verbatim so a refusal can
// re-render it.
type checkInput struct {
	name, description, scheduleKind, periodSecs, cronExpr string
	graceSecs, timezone, scanInterval, maxRuntime, nag    string
}

func readCheckInput(r *http.Request) checkInput {
	return checkInput{
		name: r.PostFormValue("name"), description: r.PostFormValue("description"),
		scheduleKind: r.PostFormValue("schedule_kind"), periodSecs: r.PostFormValue("period_secs"),
		cronExpr: r.PostFormValue("cron_expr"), graceSecs: r.PostFormValue("grace_secs"),
		timezone: r.PostFormValue("timezone"), scanInterval: r.PostFormValue("scan_interval_secs"),
		maxRuntime: r.PostFormValue("max_runtime_secs"), nag: r.PostFormValue("nag_interval_secs"),
	}
}

// emptyCheckInput is the new-check form's starting point.
func emptyCheckInput() checkInput {
	return checkInput{scheduleKind: "period", graceSecs: "5m", timezone: "UTC"}
}

func checkInputOf(check models.Check) checkInput {
	cronExpr := ""
	if check.CronExpr != nil {
		cronExpr = *check.CronExpr
	}
	return checkInput{
		name: check.Name, description: check.Description, scheduleKind: string(check.ScheduleKind),
		periodSecs: optionalDuration(check.PeriodSecs), cronExpr: cronExpr,
		graceSecs: duration.Format(check.GraceSecs), timezone: check.Timezone,
		scanInterval: optionalDuration(check.ScanIntervalSecs), maxRuntime: optionalDuration(check.MaxRuntimeSecs),
		nag: optionalDuration(check.NagIntervalSecs),
	}
}

// validatedCheck is a check form that passed validation.
type validatedCheck struct {
	name, description string
	kind              models.ScheduleKind
	periodSecs        *int64
	graceSecs         int64
	cronExpr          *string
	timezone          string
	scanInterval      *int64
	maxRuntime        *int64
	nag               *int64
}

// validateCheck refuses what would later fail silently: an unknown timezone
// would fall back to UTC when the deadline is computed.
func validateCheck(in checkInput) (validatedCheck, string) {
	name := trimRustSpace(in.name)
	if name == "" {
		return validatedCheck{}, "Name is required."
	}
	description, refusal := validateDescription(in.description)
	if refusal != "" {
		return validatedCheck{}, refusal
	}
	grace, ok := duration.Parse(in.graceSecs)
	if !ok {
		return validatedCheck{}, "Grace period must be a duration (e.g. 30, 5m, 1h30m)."
	}
	if grace < 0 {
		return validatedCheck{}, "Grace period must be 0 or more."
	}
	kind, err := models.ParseScheduleKind(in.scheduleKind)
	if err != nil {
		return validatedCheck{}, "Invalid schedule kind."
	}
	schedule, refusal := validateSchedule(kind, in)
	if refusal != "" {
		return validatedCheck{}, refusal
	}
	timezone, err := validateTimezone(in.timezone)
	if err != nil {
		return validatedCheck{}, err.Error()
	}
	overrides, refusal := parseCheckOverrides(in)
	if refusal != "" {
		return validatedCheck{}, refusal
	}
	return validatedCheck{
		name: name, description: description, kind: kind, periodSecs: schedule.periodSecs, graceSecs: grace,
		cronExpr: schedule.cronExpr, timezone: timezone,
		scanInterval: overrides.scan, maxRuntime: overrides.maxRuntime, nag: overrides.nag,
	}, ""
}

// sentence ends text with a period unless it already ends one, so the cron
// parser's mixed-punctuation details read consistently after our prefix.
func sentence(text string) string {
	if strings.HasSuffix(text, ".") {
		return text
	}
	return text + "."
}

type scheduleFields struct {
	periodSecs *int64
	cronExpr   *string
}

func validateSchedule(kind models.ScheduleKind, in checkInput) (scheduleFields, string) {
	if kind == models.ScheduleCron {
		expr := trimRustSpace(in.cronExpr)
		if expr == "" {
			return scheduleFields{}, "Cron expression is required in cron mode."
		}
		if _, err := cron.Parse(expr); err != nil {
			return scheduleFields{}, "Invalid cron expression: " + sentence(err.Error())
		}
		return scheduleFields{cronExpr: &expr}, ""
	}
	if trimRustSpace(in.periodSecs) == "" {
		return scheduleFields{}, "Period is required in period mode."
	}
	seconds, ok := duration.Parse(in.periodSecs)
	if !ok {
		return scheduleFields{}, "Period must be a duration (e.g. 30, 5m, 1h30m)."
	}
	if seconds <= 0 {
		return scheduleFields{}, "Period must be greater than 0."
	}
	return scheduleFields{periodSecs: &seconds}, ""
}

type checkOverrides struct{ scan, maxRuntime, nag *int64 }

// parseCheckOverrides refuses a non-blank override that is not a positive
// duration, rather than dropping it.
func parseCheckOverrides(in checkInput) (checkOverrides, string) {
	var overrides checkOverrides
	var refusal string
	if overrides.scan, refusal = parseOptionalDuration(in.scanInterval, "Scan interval"); refusal != "" {
		return checkOverrides{}, refusal
	}
	if overrides.maxRuntime, refusal = parseOptionalDuration(in.maxRuntime, "Max runtime"); refusal != "" {
		return checkOverrides{}, refusal
	}
	if overrides.nag, refusal = parseOptionalDuration(in.nag, "Nag interval"); refusal != "" {
		return checkOverrides{}, refusal
	}
	return overrides, ""
}

func (d *Deps) checkForm(r *http.Request, who access, target formTarget, in checkInput, refusal string) string {
	return views.CheckForm(views.CheckFormView{
		CSRF: d.csrfToken(r), IsAdmin: who.user.IsAdmin, Heading: target.heading, Action: target.action, Error: refusal,
		Name: in.name, Description: in.description, ScheduleKind: in.scheduleKind, PeriodSecs: in.periodSecs,
		CronExpr: in.cronExpr, GraceSecs: in.graceSecs, Timezone: in.timezone,
		ScanIntervalSecs: in.scanInterval, MaxRuntimeSecs: in.maxRuntime, NagIntervalSecs: in.nag,
		Timezones: tz.Names(),
	})
}

func (d *Deps) checkNew(admin bool) http.HandlerFunc {
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
		target := formTarget{"New check", fmt.Sprintf("%s/projects/%d/checks", who.base(), projectID)}
		writeHTML(w, http.StatusOK, d.checkForm(r, who, target, emptyCheckInput(), ""))
	}
}

func (d *Deps) checkCreate(admin bool) http.HandlerFunc {
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
		in := readCheckInput(r)
		valid, refusal := validateCheck(in)
		if refusal != "" {
			target := formTarget{"New check", fmt.Sprintf("%s/projects/%d/checks", who.base(), projectID)}
			writeHTML(w, http.StatusOK, d.checkForm(r, who, target, in, refusal))
			return
		}
		id, err := d.createCheck(r, projectID, valid)
		if err != nil {
			internalError(w)
			return
		}
		redirect(w, r, fmt.Sprintf("%s/checks/%d", who.base(), id))
	}
}

// createCheck binds the new check to every channel its project has.
func (d *Deps) createCheck(r *http.Request, projectID int64, valid validatedCheck) (int64, error) {
	ctx := writeCtx(r)
	id, err := d.Store.CreateCheck(ctx, store.NewCheck{
		ProjectID: projectID, Name: valid.name, Description: valid.description, PingUUID: ids.NewV4(),
		Kind: valid.kind, PeriodSecs: valid.periodSecs, GraceSecs: valid.graceSecs, CronExpr: valid.cronExpr,
		Timezone: valid.timezone, ScanIntervalSecs: valid.scanInterval, MaxRuntimeSecs: valid.maxRuntime,
		NagIntervalSecs: valid.nag,
	}, d.Now())
	if err != nil {
		return 0, err
	}
	return id, d.Store.BindAllProjectChannels(ctx, id, projectID)
}

func (d *Deps) checkEdit(admin bool) http.HandlerFunc {
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
		target := formTarget{"Edit check", fmt.Sprintf("%s/checks/%d", who.base(), id)}
		writeHTML(w, http.StatusOK, d.checkForm(r, who, target, checkInputOf(*check), ""))
	}
}

func (d *Deps) checkUpdate(admin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		who, ok := d.enter(w, r, admin)
		if !ok {
			return
		}
		id, ok := pathID(w, r, "id")
		if !ok {
			return
		}
		if _, ok := d.checkFor(w, r, who, id); !ok {
			return
		}
		in := readCheckInput(r)
		valid, refusal := validateCheck(in)
		target := formTarget{"Edit check", fmt.Sprintf("%s/checks/%d", who.base(), id)}
		if refusal != "" {
			writeHTML(w, http.StatusOK, d.checkForm(r, who, target, in, refusal))
			return
		}
		err := d.Store.UpdateCheckSchedule(writeCtx(r), id, store.UpdateCheck{
			Name: valid.name, Description: valid.description, Kind: valid.kind, PeriodSecs: valid.periodSecs,
			GraceSecs: valid.graceSecs, CronExpr: valid.cronExpr, Timezone: valid.timezone,
			ScanIntervalSecs: valid.scanInterval, MaxRuntimeSecs: valid.maxRuntime, NagIntervalSecs: valid.nag,
		})
		if err != nil {
			internalError(w)
			return
		}
		redirect(w, r, fmt.Sprintf("%s/checks/%d", who.base(), id))
	}
}
