package web

import (
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/view"
	"github.com/henry40408/pingward/internal/views"
)

const auditPageSize = 20

// settingsFields is the persisted values, or an invalid save's raw input so
// it can be fixed.
type settingsFields struct {
	scanInterval, nagInterval                      string
	pingsRetentionDays, notificationsRetentionDays string
	auditRetentionDays, displayTimezone            string
}

// adminNotices are the per-request parts of renderAdmin; it loads the rest.
type adminNotices struct {
	settings      settingsFields
	settingsError string
	settingsFlash string
	userFlash     string
	// elevationFlash is separate from userFlash, which is the error slot.
	elevationFlash     string
	passwordResetFlash string
	userError          string
}

// auditQuery is a-prefixed to share /admin's query string. Bad filter text is
// ignored, but a non-numeric cursor is a 400.
type auditQuery struct {
	before, after           *int64
	actor, action, from, to string
}

func readAuditQuery(w http.ResponseWriter, r *http.Request) (auditQuery, bool) {
	values := r.URL.Query()
	q := auditQuery{
		actor: values.Get("aactor"), action: values.Get("aaction"),
		from: values.Get("afrom"), to: values.Get("ato"),
	}
	for _, cursor := range []struct {
		key  string
		into **int64
	}{{"ab", &q.before}, {"aa", &q.after}} {
		if !values.Has(cursor.key) {
			continue
		}
		id, err := strconv.ParseInt(values.Get(cursor.key), 10, 64)
		if err != nil {
			http.Error(w, "invalid "+cursor.key, http.StatusBadRequest)
			return auditQuery{}, false
		}
		*cursor.into = &id
	}
	return q, true
}

// filterText is free text (audit actor/action); an unknown value just pages empty.
func filterText(raw string) *string {
	text := trimRustSpace(raw)
	if text == "" {
		return nil
	}
	return &text
}

func (q auditQuery) filter() store.AuditFilter {
	return store.AuditFilter{
		Actor: filterText(q.actor), Action: filterText(q.action),
		From: parseDateBound(q.from), To: parseDateBound(q.to),
	}
}

func auditTarget(entry models.AuditLog) string {
	switch {
	case entry.TargetType != nil && entry.TargetID != nil:
		return *entry.TargetType + " #" + strconv.FormatInt(*entry.TargetID, 10)
	case entry.TargetType != nil:
		return *entry.TargetType
	}
	return "—"
}

func auditMethodPath(entry models.AuditLog) string {
	switch {
	case entry.Method != nil && entry.Path != nil:
		return *entry.Method + " " + *entry.Path
	case entry.Method != nil:
		return *entry.Method
	case entry.Path != nil:
		return *entry.Path
	}
	return "—"
}

func auditRow(entry models.AuditLog) views.AuditRow {
	owner := "—"
	if entry.TargetOwnerID != nil {
		owner = "user #" + strconv.FormatInt(*entry.TargetOwnerID, 10)
	}
	detail := ""
	if entry.Detail != nil {
		detail = *entry.Detail
	}
	return views.AuditRow{
		Time: view.FmtUTC(entry.CreatedAt), ISO: view.RFC3339(entry.CreatedAt),
		Actor: entry.ActorUsername, Action: entry.Action,
		Target: auditTarget(entry), MethodPath: auditMethodPath(entry),
		Detail: detail, TargetOwner: owner,
		Expandable: entry.Method != nil || entry.Path != nil || entry.Detail != nil || entry.TargetOwnerID != nil,
	}
}

func (d *Deps) auditPartial(r *http.Request, q auditQuery) (views.AdminAuditView, error) {
	filter := q.filter()
	page, err := d.Store.ListAuditPage(r.Context(), pageCursor(q.before, q.after), auditPageSize, filter)
	if err != nil {
		return views.AdminAuditView{}, err
	}
	actors, actions, err := d.Store.AuditFilterOptions(r.Context())
	if err != nil {
		return views.AdminAuditView{}, err
	}
	rows := make([]views.AuditRow, len(page.Items))
	for i, entry := range page.Items {
		rows[i] = auditRow(entry)
	}
	fromToken, toToken := dateBoundToken(filter.From), dateBoundToken(filter.To)
	actorText, actionText := deref(filter.Actor), deref(filter.Action)
	carry := [][2]string{{"aactor", actorText}, {"aaction", actionText}, {"afrom", fromToken}, {"ato", toToken}}
	older, newer := neighbourIDs(page, func(entry models.AuditLog) int64 { return entry.ID })
	return views.AdminAuditView{
		Rows: rows, Actors: actors, Actions: actions,
		FilterActor: actorText, FilterAction: actionText, FilterFrom: fromToken, FilterTo: toToken,
		Filtered: !filter.IsEmpty(),
		Older:    hrefOrEmpty(older, func(id int64) string { return historyHref("/admin/audit", "ab", id, carry) }),
		Newer:    hrefOrEmpty(newer, func(id int64) string { return historyHref("/admin/audit", "aa", id, carry) }),
	}, nil
}

func deref(text *string) string {
	if text == nil {
		return ""
	}
	return *text
}

// adminAuditFragment serves the audit card on its own for the JS filter.
func (d *Deps) adminAuditFragment(w http.ResponseWriter, r *http.Request) {
	if _, ok := d.requireAdmin(w, r); !ok {
		return
	}
	// Parsed first, as axum's Query extractor is: a bad cursor is 400 even on a redirect.
	q, ok := readAuditQuery(w, r)
	if !ok {
		return
	}
	if !wantsFragment(r) {
		fragmentPageRedirect(w, r, "/admin", "audit-section")
		return
	}
	partial, err := d.auditPartial(r, q)
	if err != nil {
		internalError(w)
		return
	}
	writeHTML(w, http.StatusOK, string(views.AdminAudit(partial)))
}

func (d *Deps) adminPage(w http.ResponseWriter, r *http.Request) {
	admin, ok := d.requireAdmin(w, r)
	if !ok {
		return
	}
	q, ok := readAuditQuery(w, r)
	if !ok {
		return
	}
	settings, err := d.loadSettingsFields(r)
	if err != nil {
		internalError(w)
		return
	}
	notices := adminNotices{settings: settings}
	// Each takeFlash consumes only its own surface's cookie. admin_locked is
	// left for /admin/unlock.
	notices.settingsFlash, _ = d.takeFlash(w, r, "settings")
	notices.userFlash, _ = d.takeFlash(w, r, "users_blocked")
	notices.elevationFlash, _ = d.takeFlash(w, r, "admin_unlocked")
	notices.passwordResetFlash = d.takePasswordResetKeysFlash(w, r)
	d.renderAdmin(w, r, admin, notices, q)
}

// renderAdmin: the audit card is rendered here so /admin and /admin/audit emit
// identical markup.
func (d *Deps) renderAdmin(w http.ResponseWriter, r *http.Request, admin *models.User, notices adminNotices, q auditQuery) {
	page, err := d.adminView(r, admin, notices, q)
	if err != nil {
		internalError(w)
		return
	}
	writeHTML(w, http.StatusOK, views.Admin(page))
}

func (d *Deps) adminView(r *http.Request, admin *models.User, notices adminNotices, q auditQuery) (views.AdminView, error) {
	ctx := r.Context()
	now := d.Now()
	dayAgo := now.Add(-24 * time.Hour)
	partial, err := d.auditPartial(r, q)
	if err != nil {
		return views.AdminView{}, err
	}
	overview, err := d.adminOverview(r, dayAgo)
	if err != nil {
		return views.AdminView{}, err
	}
	users, err := d.Store.ListUsers(ctx)
	if err != nil {
		return views.AdminView{}, err
	}
	projects, err := d.Store.ListAllProjectsWithOwner(ctx)
	if err != nil {
		return views.AdminView{}, err
	}
	standing := d.elevationOf(r, admin)
	overview.CSRF = d.csrfToken(r)
	overview.AuditPartial = views.AdminAudit(partial)
	overview.LastScan, err = d.loopStamp(r, "last_scan_at", now)
	if err != nil {
		return views.AdminView{}, err
	}
	overview.LastPrune, err = d.loopStamp(r, "last_prune_at", now)
	if err != nil {
		return views.AdminView{}, err
	}
	s := notices.settings
	overview.ScanInterval, overview.NagInterval = s.scanInterval, s.nagInterval
	overview.PingsRetentionDays, overview.NotificationsRetentionDays = s.pingsRetentionDays, s.notificationsRetentionDays
	overview.AuditRetentionDays, overview.DisplayTimezone = s.auditRetentionDays, s.displayTimezone
	overview.SettingsError, overview.SettingsFlash = notices.settingsError, notices.settingsFlash
	overview.Users = userRows(users, admin.ID)
	overview.UserFlash, overview.ElevationFlash = notices.userFlash, notices.elevationFlash
	overview.ElevationApplies = !standing.notApplicable
	overview.ElevationLocked = !standing.notApplicable && !standing.unlocked
	if standing.unlocked {
		overview.ElevationRemaining = readableSecs(standing.remainingSecs)
	}
	overview.PasswordResetFlash, overview.UserError = notices.passwordResetFlash, notices.userError
	overview.Projects = projectRows(projects)
	overview.Env = envGroups(d.Config)
	return overview, nil
}

// adminOverview fills the site-wide counters and health tables.
func (d *Deps) adminOverview(r *http.Request, dayAgo time.Time) (views.AdminView, error) {
	ctx := r.Context()
	var v views.AdminView
	var err error
	if v.UserCount, err = d.Store.CountUsers(ctx); err != nil {
		return v, err
	}
	if v.ProjectCount, err = d.Store.CountProjects(ctx); err != nil {
		return v, err
	}
	if v.Checks, err = d.Store.CountChecks(ctx); err != nil {
		return v, err
	}
	if v.Pings24h, err = d.Store.CountPingsSince(ctx, dayAgo); err != nil {
		return v, err
	}
	counts, err := d.Store.CountChecksByStatus(ctx)
	if err != nil {
		return v, err
	}
	v.Status = views.StatusCounts{New: counts.New, Up: counts.Up, Running: counts.Running, Down: counts.Down, Paused: counts.Paused}
	down, err := d.Store.ListDownChecksWithOwner(ctx)
	if err != nil {
		return v, err
	}
	for _, entry := range down {
		v.Down = append(v.Down, views.DownCheck{ID: entry.Check.ID, Name: entry.Check.Name, Project: entry.ProjectName, Owner: entry.OwnerUsername})
	}
	if v.NotifOK, v.NotifErr, err = d.Store.NotificationCountsSince(ctx, dayAgo); err != nil {
		return v, err
	}
	failures, err := d.Store.ChannelFailureCountsSince(ctx, dayAgo)
	if err != nil {
		return v, err
	}
	for _, failure := range failures {
		v.ChannelFailures = append(v.ChannelFailures, views.ChannelFailure{Name: failure.ChannelName, OK: failure.OK, Failed: failure.Errors})
	}
	recent, err := d.Store.RecentFailedNotifications(ctx, 10)
	if err != nil {
		return v, err
	}
	for _, notification := range recent {
		v.RecentFailures = append(v.RecentFailures, views.FailedNotification{
			CreatedAt: notification.CreatedAt, Event: string(notification.Event),
			CheckID: notification.CheckID, Error: deref(notification.Error),
		})
	}
	return v, nil
}

// loopStamp: an unparseable stamp renders no age rather than a wrong one.
func (d *Deps) loopStamp(r *http.Request, key string, now time.Time) (views.LoopStamp, error) {
	raw, err := d.Store.GetSetting(r.Context(), key)
	if err != nil || raw == nil {
		return views.LoopStamp{}, err
	}
	stamp := views.LoopStamp{At: *raw}
	at, err := time.Parse(time.RFC3339, *raw)
	if err != nil {
		return stamp, nil
	}
	stamp.Ago, stamp.UTC = view.FmtRelative(at, now), view.FmtUTC(at)
	return stamp, nil
}

func userRows(users []models.User, adminID int64) []views.AdminUserRow {
	rows := make([]views.AdminUserRow, len(users))
	for i, user := range users {
		rows[i] = views.AdminUserRow{ID: user.ID, Username: user.Username, IsAdmin: user.IsAdmin, Disabled: user.Disabled, IsSelf: user.ID == adminID}
	}
	return rows
}

func projectRows(projects []store.ProjectWithOwner) []views.AdminProjectRow {
	rows := make([]views.AdminProjectRow, len(projects))
	for i, project := range projects {
		rows[i] = views.AdminProjectRow{ID: project.Project.ID, Name: project.Project.Name, Owner: project.OwnerUsername}
	}
	return rows
}

// readableSettingDuration turns stored seconds into the readable form;
// anything else passes through untouched.
func readableSettingDuration(raw string) string {
	seconds, err := strconv.ParseInt(strings.TrimFunc(raw, isRustSpace), 10, 64)
	if err != nil || seconds <= 0 {
		return raw
	}
	return readableSecs(seconds)
}

// loadSettingsFields is the stored settings, for /admin and user-error re-renders.
func (d *Deps) loadSettingsFields(r *http.Request) (settingsFields, error) {
	var fields settingsFields
	for _, field := range []struct {
		key  string
		into *string
	}{
		{"scan_interval", &fields.scanInterval}, {"nag_interval", &fields.nagInterval},
		{"pings_retention_days", &fields.pingsRetentionDays},
		{"notifications_retention_days", &fields.notificationsRetentionDays},
		{"audit_retention_days", &fields.auditRetentionDays}, {"display_timezone", &fields.displayTimezone},
	} {
		value, err := d.Store.GetSetting(r.Context(), field.key)
		if err != nil {
			return fields, err
		}
		*field.into = deref(value)
	}
	fields.scanInterval = readableSettingDuration(fields.scanInterval)
	fields.nagInterval = readableSettingDuration(fields.nagInterval)
	return fields, nil
}
