package web

import (
	"fmt"
	"net/http"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/view"
	"github.com/henry40408/pingward/internal/views"
)

const historyPageSize = 20

// The query keys each history section's visible controls own; the other
// section's keys ride along as hidden fields.
var (
	pingFilterKeys  = []string{"pk", "pfrom", "pto"}
	notifFilterKeys = []string{"ne", "ns", "nfrom", "nto"}
)

// historyQuery is shared by the check page and both fragments: p* are pings,
// n* notifications; pb/pa and nb/na are keyset cursors (older/newer).
type historyQuery struct {
	pb, pa, nb, na *int64
	pk, ne, ns     string
	pfrom, pto     string
	nfrom, nto     string
}

// readHistoryQuery: bad filter text is ignored, but a non-numeric cursor is a 400.
func readHistoryQuery(w http.ResponseWriter, r *http.Request) (historyQuery, bool) {
	values := r.URL.Query()
	q := historyQuery{
		pk: values.Get("pk"), ne: values.Get("ne"), ns: values.Get("ns"),
		pfrom: values.Get("pfrom"), pto: values.Get("pto"), nfrom: values.Get("nfrom"), nto: values.Get("nto"),
	}
	cursors := []struct {
		key  string
		into **int64
	}{{"pb", &q.pb}, {"pa", &q.pa}, {"nb", &q.nb}, {"na", &q.na}}
	for _, cursor := range cursors {
		if !values.Has(cursor.key) {
			continue
		}
		id, err := strconv.ParseInt(values.Get(cursor.key), 10, 64)
		if err != nil {
			http.Error(w, "Invalid cursor.", http.StatusBadRequest)
			return historyQuery{}, false
		}
		*cursor.into = &id
	}
	return q, true
}

// parseDateBound reads RFC 3339 (from JS) or a bare YYYY-MM-DDTHH:MM[:SS] (JS
// off) as UTC; anything else is no bound.
func parseDateBound(raw string) *time.Time {
	text := trimRustSpace(raw)
	if text == "" {
		return nil
	}
	if at, err := time.Parse(time.RFC3339, text); err == nil {
		at = at.UTC()
		return &at
	}
	for _, layout := range []string{"2006-01-02T15:04:05", "2006-01-02T15:04"} {
		if at, err := time.Parse(layout, text); err == nil {
			return &at
		}
	}
	return nil
}

// dateBoundToken is Z-form because +00:00 would need percent-encoding in a pager href.
func dateBoundToken(at *time.Time) string {
	if at == nil {
		return ""
	}
	return at.UTC().Format("2006-01-02T15:04:05Z")
}

// parseFilterToken: blank or unknown is no filter; the UI offers one choice.
func parseFilterToken[T ~string](raw string, known ...T) []T {
	token := T(trimRustSpace(raw))
	if !slices.Contains(known, token) {
		return nil
	}
	return []T{token}
}

func (q historyQuery) pingFilter() store.PingFilter {
	return store.PingFilter{
		Kinds: parseFilterToken(q.pk, models.PingSuccess, models.PingFail, models.PingStart, models.PingLog, models.PingExitcode),
		From:  parseDateBound(q.pfrom), To: parseDateBound(q.pto),
	}
}

func (q historyQuery) notifFilter() store.NotifFilter {
	return store.NotifFilter{
		Events:   parseFilterToken(q.ne, models.EventDown, models.EventUp, models.EventReminder, models.EventTest),
		Statuses: parseFilterToken(q.ns, models.NotifyOK, models.NotifyError),
		From:     parseDateBound(q.nfrom), To: parseDateBound(q.nto),
	}
}

// pageCursor: before wins when both are set.
func pageCursor(before, after *int64) store.PageCursor {
	switch {
	case before != nil:
		return store.Before(*before)
	case after != nil:
		return store.After(*after)
	}
	return store.PageCursor{}
}

func firstToken[T ~string](values []T) string {
	if len(values) == 0 {
		return ""
	}
	return string(values[0])
}

// filterTokens is every check-page filter token, so each section can carry the
// other's half.
type filterTokens struct {
	kind, pingFrom, pingTo, event, status, notifFrom, notifTo string
}

func (t filterTokens) pairs() [][2]string {
	return [][2]string{
		{"pk", t.kind}, {"pfrom", t.pingFrom}, {"pto", t.pingTo},
		{"ne", t.event}, {"ns", t.status}, {"nfrom", t.notifFrom}, {"nto", t.notifTo},
	}
}

// carryFields skips empties and mine, the keys this form's visible controls already send.
func carryFields(all [][2]string, mine []string) []views.HiddenField {
	var out []views.HiddenField
	for _, pair := range all {
		if pair[1] != "" && !slices.Contains(mine, pair[0]) {
			out = append(out, views.HiddenField{Name: pair[0], Value: pair[1]})
		}
	}
	return out
}

func clearHref(path string, carry []views.HiddenField) string {
	var href strings.Builder
	href.WriteString(path)
	for i, field := range carry {
		separator := "&"
		if i == 0 {
			separator = "?"
		}
		href.WriteString(separator + field.Name + "=" + field.Value)
	}
	return href.String()
}

// historyHref values are ids, enum tokens or Z-form datetimes: query-safe, so no encoding.
func historyHref(path, cursorKey string, cursorID int64, carry [][2]string) string {
	href := fmt.Sprintf("%s?%s=%d", path, cursorKey, cursorID)
	for _, pair := range carry {
		if pair[1] != "" {
			href += "&" + pair[0] + "=" + pair[1]
		}
	}
	return href
}

// neighbourIDs are the ids of the rows at the page's far edges, when there are more pages.
func neighbourIDs[T any](page store.Page[T], idOf func(T) int64) (older, newer *int64) {
	if page.HasOlder && len(page.Items) > 0 {
		id := idOf(page.Items[len(page.Items)-1])
		older = &id
	}
	if page.HasNewer && len(page.Items) > 0 {
		id := idOf(page.Items[0])
		newer = &id
	}
	return older, newer
}

func hrefOrEmpty(id *int64, build func(int64) string) string {
	if id == nil {
		return ""
	}
	return build(*id)
}

func pingPillClass(kind models.PingKind) string {
	switch kind {
	case models.PingFail:
		return "fail"
	case models.PingStart:
		return "start"
	case models.PingLog:
		return "log"
	}
	return "ok"
}

// notifEventPillClass reuses the ping-kind palette.
func notifEventPillClass(event models.EventKind) string {
	switch event {
	case models.EventUp:
		return "ok"
	case models.EventDown:
		return "fail"
	case models.EventReminder:
		return "start"
	}
	return "log"
}

func pingRows(pings []models.Ping, durations map[int64]int64) []views.PingRow {
	rows := make([]views.PingRow, len(pings))
	for i, ping := range pings {
		row := views.PingRow{
			Time: view.FmtUTC(ping.CreatedAt), ISO: view.RFC3339(ping.CreatedAt),
			PillClass: pingPillClass(ping.Kind), KindLabel: string(ping.Kind),
			Exit: "—", Duration: "—", Source: "—", Body: ping.Body,
		}
		if ping.ExitCode != nil {
			row.Exit = fmt.Sprintf("exit %d", *ping.ExitCode)
		}
		if secs, ok := durations[ping.ID]; ok {
			row.Duration = view.FmtSecs(secs)
		}
		if ping.SourceIP != nil {
			row.Source = *ping.SourceIP
		}
		rows[i] = row
	}
	return rows
}

func notificationRows(notifications []models.Notification, channelNames map[int64]string) []views.NotificationRow {
	rows := make([]views.NotificationRow, len(notifications))
	for i, n := range notifications {
		channel, known := channelNames[n.ChannelID]
		if !known {
			channel = "(deleted)"
		}
		row := views.NotificationRow{
			CreatedAt: view.FmtUTC(n.CreatedAt), ISO: view.RFC3339(n.CreatedAt),
			Event: string(n.Event), EventPillClass: notifEventPillClass(n.Event),
			Status: string(n.Status), Channel: channel,
		}
		if n.Error != nil {
			row.Error = *n.Error
		}
		rows[i] = row
	}
	return rows
}

// pingsPartial recent is the page's already-fetched heartbeat window; nil
// re-fetches when the default view needs it.
func (d *Deps) pingsPartial(r *http.Request, base string, checkID int64, q historyQuery, recent []models.PingSummary) (views.CheckPingsView, error) {
	ctx := r.Context()
	filter := q.pingFilter()
	cursor := pageCursor(q.pb, q.pa)
	page, err := d.Store.ListPingsPage(ctx, checkID, cursor, historyPageSize, filter)
	if err != nil {
		return views.CheckPingsView{}, err
	}
	// The default view pairs over the heartbeat window, so an off-page start
	// still yields a duration; elsewhere pairing is best-effort.
	var durations map[int64]int64
	if cursor.Kind == store.CursorLatest && filter.IsEmpty() {
		if recent == nil {
			recent, err = d.Store.ListRecentPingSummaries(ctx, checkID, heartbeatWindow)
			if err != nil {
				return views.CheckPingsView{}, err
			}
		}
		durations = view.RunDurations(recent)
	} else {
		summaries := make([]models.PingSummary, len(page.Items))
		for i, ping := range page.Items {
			summaries[i] = ping.Summary()
		}
		durations = view.RunDurations(summaries)
	}
	kind, from, to := firstToken(filter.Kinds), dateBoundToken(filter.From), dateBoundToken(filter.To)
	carry := [][2]string{{"pk", kind}, {"pfrom", from}, {"pto", to}}
	endpoint := fmt.Sprintf("%s/checks/%d/pings", base, checkID)
	olderID, newerID := neighbourIDs(page, func(p models.Ping) int64 { return p.ID })

	notifFilter := q.notifFilter()
	hidden := carryFields(filterTokens{
		kind: kind, pingFrom: from, pingTo: to,
		event: firstToken(notifFilter.Events), status: firstToken(notifFilter.Statuses),
		notifFrom: dateBoundToken(notifFilter.From), notifTo: dateBoundToken(notifFilter.To),
	}.pairs(), pingFilterKeys)
	return views.CheckPingsView{
		HistoryFilter: views.HistoryFilter{
			Base: base, CheckID: checkID, FromUTC: from, ToUTC: to, Filtered: !filter.IsEmpty(),
			Carry: hidden, Clear: clearHref(endpoint, hidden),
			Newer: hrefOrEmpty(newerID, func(id int64) string { return historyHref(endpoint, "pa", id, carry) }),
			Older: hrefOrEmpty(olderID, func(id int64) string { return historyHref(endpoint, "pb", id, carry) }),
		},
		Rows: pingRows(page.Items, durations), Kind: kind,
	}, nil
}

func (d *Deps) notifsPartial(r *http.Request, base string, checkID int64, q historyQuery, channelNames map[int64]string) (views.CheckNotifsView, error) {
	filter := q.notifFilter()
	page, err := d.Store.ListNotificationsPage(r.Context(), checkID, pageCursor(q.nb, q.na), historyPageSize, filter)
	if err != nil {
		return views.CheckNotifsView{}, err
	}
	event, status := firstToken(filter.Events), firstToken(filter.Statuses)
	from, to := dateBoundToken(filter.From), dateBoundToken(filter.To)
	carry := [][2]string{{"ne", event}, {"ns", status}, {"nfrom", from}, {"nto", to}}
	endpoint := fmt.Sprintf("%s/checks/%d/notifications", base, checkID)
	olderID, newerID := neighbourIDs(page, func(n models.Notification) int64 { return n.ID })

	pingFilter := q.pingFilter()
	hidden := carryFields(filterTokens{
		kind: firstToken(pingFilter.Kinds), pingFrom: dateBoundToken(pingFilter.From), pingTo: dateBoundToken(pingFilter.To),
		event: event, status: status, notifFrom: from, notifTo: to,
	}.pairs(), notifFilterKeys)
	return views.CheckNotifsView{
		HistoryFilter: views.HistoryFilter{
			Base: base, CheckID: checkID, FromUTC: from, ToUTC: to, Filtered: !filter.IsEmpty(),
			Carry: hidden, Clear: clearHref(endpoint, hidden),
			Newer: hrefOrEmpty(newerID, func(id int64) string { return historyHref(endpoint, "na", id, carry) }),
			Older: hrefOrEmpty(olderID, func(id int64) string { return historyHref(endpoint, "nb", id, carry) }),
		},
		Rows: notificationRows(page.Items, channelNames), Event: event, Status: status,
	}, nil
}

func (d *Deps) channelNames(r *http.Request, projectID int64) (map[int64]string, error) {
	channels, err := d.Store.ListChannelsForProject(r.Context(), projectID)
	if err != nil {
		return nil, err
	}
	names := make(map[int64]string, len(channels))
	for _, channel := range channels {
		names[channel.ID] = channel.Name
	}
	return names, nil
}

// fragmentPageRedirect sends a JS-off pager or Clear link, which hits the
// fragment endpoint, to the embedding page instead of serving a bare partial.
func fragmentPageRedirect(w http.ResponseWriter, r *http.Request, path, anchor string) {
	target := path
	if query := r.URL.RawQuery; query != "" {
		target += "?" + query
	}
	redirect(w, r, target+"#"+anchor)
}

func (d *Deps) checkPings(admin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		who, ok := d.enter(w, r, admin)
		if !ok {
			return
		}
		id, ok := pathID(w, r, "id")
		if !ok {
			return
		}
		// Ownership first, or the redirect would disclose that the check exists.
		check, ok := d.checkFor(w, r, who, id)
		if !ok {
			return
		}
		if !wantsFragment(r) {
			fragmentPageRedirect(w, r, fmt.Sprintf("%s/checks/%d", who.base(), check.ID), "pings-section")
			return
		}
		q, ok := readHistoryQuery(w, r)
		if !ok {
			return
		}
		partial, err := d.pingsPartial(r, who.base(), check.ID, q, nil)
		if err != nil {
			internalError(w)
			return
		}
		writeHTML(w, http.StatusOK, string(views.CheckPings(partial)))
	}
}

func (d *Deps) checkNotifications(admin bool) http.HandlerFunc {
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
		if !wantsFragment(r) {
			fragmentPageRedirect(w, r, fmt.Sprintf("%s/checks/%d", who.base(), check.ID), "notifs-section")
			return
		}
		q, ok := readHistoryQuery(w, r)
		if !ok {
			return
		}
		names, err := d.channelNames(r, check.ProjectID)
		if err != nil {
			internalError(w)
			return
		}
		partial, err := d.notifsPartial(r, who.base(), check.ID, q, names)
		if err != nil {
			internalError(w)
			return
		}
		writeHTML(w, http.StatusOK, string(views.CheckNotifs(partial)))
	}
}
