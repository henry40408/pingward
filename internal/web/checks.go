package web

import (
	"context"
	"fmt"
	"github.com/henry40408/pingward/internal/markdown"
	"net/http"
	"slices"
	"strings"
	"time"

	"github.com/henry40408/pingward/internal/ids"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/view"
	"github.com/henry40408/pingward/internal/views"
)

// writeCtx keeps a mutation running to completion when the browser hangs up
// mid-request: several handlers write in more than one step, and a cancelled
// context would leave them half applied.
func writeCtx(r *http.Request) context.Context { return context.WithoutCancel(r.Context()) }

// statusSinceLabel reads like "down · 2h ago · not acknowledged", or "updated 3m ago".
func statusSinceLabel(check models.Check, now time.Time) string {
	if view.Status(check, now) != view.StatusDown {
		relative := "never"
		if check.LastPingAt != nil {
			relative = view.FmtRelative(*check.LastPingAt, now)
		}
		return "updated " + relative
	}
	acknowledgement := "not acknowledged"
	if check.Acknowledged {
		acknowledgement = "acknowledged"
	}
	// A check can go New -> Down without ever having received a ping.
	relative := "no pings yet"
	if check.LastPingAt != nil {
		relative = view.FmtRelative(*check.LastPingAt, now)
	}
	return "down · " + relative + " · " + acknowledgement
}

// checkPageOptions is what a check page shows beyond the session.
type checkPageOptions struct {
	flash string
	// pingURLRevealed is an admin's audited reveal of another user's ping URL.
	pingURLRevealed bool
}

// showsPingURL: the URL is a bearer credential. An admin must reveal another
// user's; their own is exempt.
func (who access) showsPingURL(ownerID int64, revealed bool) bool {
	return !who.admin || revealed || who.user.ID == ownerID
}

func (d *Deps) pingURL(check models.Check) string {
	return strings.TrimRight(d.Config.BaseURL, "/") + "/ping/" + check.PingUUID
}

// dueTime is nil until the scheduler is wired, which reads as "next due unknown".
func (d *Deps) dueTime(check models.Check) *time.Time {
	if d.DueTime == nil {
		return nil
	}
	return d.DueTime(check)
}

func (d *Deps) renderCheckPage(w http.ResponseWriter, r *http.Request, who access, check *models.Check, query historyQuery, opts checkPageOptions) {
	project, err := d.Store.FindProject(r.Context(), check.ProjectID)
	if err != nil {
		internalError(w)
		return
	}
	if project == nil {
		notFound(w)
		return
	}
	page, err := d.checkPageView(r, who, checkPageInput{check: *check, project: *project, query: query}, opts)
	if err != nil {
		internalError(w)
		return
	}
	writeHTML(w, http.StatusOK, views.Check(page))
}

type checkPageInput struct {
	check   models.Check
	project models.Project
	query   historyQuery
}

func (d *Deps) checkPageView(r *http.Request, who access, in checkPageInput, opts checkPageOptions) (views.CheckView, error) {
	ctx := r.Context()
	check, project, query := in.check, in.project, in.query
	now := d.Now()
	bound, err := d.Store.BoundChannelIDs(ctx, check.ID)
	if err != nil {
		return views.CheckView{}, err
	}
	channels, err := d.Store.ListChannelsForProject(ctx, check.ProjectID)
	if err != nil {
		return views.CheckView{}, err
	}
	names := make(map[int64]string, len(channels))
	boxes := make([]views.ChannelBox, len(channels))
	for i, channel := range channels {
		names[channel.ID] = channel.Name
		boxes[i] = views.ChannelBox{ID: channel.ID, Name: channel.Name, Kind: string(channel.Kind), Bound: slices.Contains(bound, channel.ID)}
	}
	// Always the latest window, never the table's page; summaries, so no bodies load.
	recent, err := d.Store.ListRecentPingSummaries(ctx, check.ID, heartbeatWindow)
	if err != nil {
		return views.CheckView{}, err
	}
	pings, err := d.pingsPartial(r, who.base(), check.ID, query, recent)
	if err != nil {
		return views.CheckView{}, err
	}
	notifs, err := d.notifsPartial(r, who.base(), check.ID, query, names)
	if err != nil {
		return views.CheckView{}, err
	}
	showURL := who.showsPingURL(project.UserID, opts.pingURLRevealed)
	pingURL := ""
	if showURL {
		pingURL = d.pingURL(check)
	}
	return views.CheckView{
		CSRF: d.csrfToken(r), IsAdmin: who.user.IsAdmin, Admin: who.admin, Check: check,
		DescriptionHTML: markdown.Render(check.Description), ProjectName: project.Name,
		Status: string(view.Status(check, now)), Since: statusSinceLabel(check, now),
		NextDue:  view.NextDueFor(check, d.dueTime(check), now),
		Schedule: scheduleLabel(check), PingURL: pingURL, PingURLHidden: !showURL,
		Bars: view.Heartbeat(recent, view.HeartbeatOptions{
			MaxRuntimeSecs: check.MaxRuntimeSecs, Paused: check.Status == models.StatusPaused, Count: heartbeatBars,
		}),
		ChannelBoxes: boxes, PingsPartial: views.CheckPings(pings), NotifsPartial: views.CheckNotifs(notifs),
		Flash: opts.flash,
	}, nil
}

func (d *Deps) checkShow(admin bool) http.HandlerFunc {
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
		query, ok := readHistoryQuery(w, r)
		if !ok {
			return
		}
		flash, _ := d.takeFlash(w, r, "channels")
		d.renderCheckPage(w, r, who, check, query, checkPageOptions{flash: flash})
	}
}

// checkAction is a one-step mutation of a check followed by a redirect to its page.
func (d *Deps) checkAction(admin bool, apply func(context.Context, *Deps, *models.Check) error) http.HandlerFunc {
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
		if err := apply(writeCtx(r), d, check); err != nil {
			internalError(w)
			return
		}
		redirect(w, r, fmt.Sprintf("%s/checks/%d", who.base(), id))
	}
}

func (d *Deps) checkPause(admin bool) http.HandlerFunc {
	return d.checkAction(admin, func(ctx context.Context, d *Deps, c *models.Check) error {
		return d.Store.SetStatus(ctx, c.ID, models.StatusPaused)
	})
}

func (d *Deps) checkResume(admin bool) http.HandlerFunc {
	return d.checkAction(admin, func(ctx context.Context, d *Deps, c *models.Check) error {
		return d.Store.SetStatus(ctx, c.ID, models.StatusNew)
	})
}

func (d *Deps) checkAck(admin bool) http.HandlerFunc {
	return d.checkAction(admin, func(ctx context.Context, d *Deps, c *models.Check) error {
		return d.Store.Acknowledge(ctx, c.ID)
	})
}

// confirmedCheckAction is a checkAction behind a confirmation page.
func (d *Deps) confirmedCheckAction(admin bool, copy confirmCopy, slug string, apply func(w http.ResponseWriter, r *http.Request, who access, check *models.Check)) http.HandlerFunc {
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
		page := fmt.Sprintf("%s/checks/%d", who.base(), id)
		if !isConfirmed(r) {
			d.confirmationPage(w, r, who.user.IsAdmin, copy, page+"/"+slug, page)
			return
		}
		apply(w, r, who, check)
	}
}

func (d *Deps) checkRegenerate(admin bool) http.HandlerFunc {
	return d.confirmedCheckAction(admin, confirmRegenerateURL, "regenerate", func(w http.ResponseWriter, r *http.Request, who access, check *models.Check) {
		if err := d.Store.RegenerateUUID(writeCtx(r), check.ID, ids.NewV4()); err != nil {
			internalError(w)
			return
		}
		redirect(w, r, fmt.Sprintf("%s/checks/%d", who.base(), check.ID))
	})
}

func (d *Deps) checkDelete(admin bool) http.HandlerFunc {
	return d.confirmedCheckAction(admin, confirmDeleteCheck, "delete", func(w http.ResponseWriter, r *http.Request, who access, check *models.Check) {
		if err := d.Store.DeleteCheck(writeCtx(r), check.ID); err != nil {
			internalError(w)
			return
		}
		redirect(w, r, fmt.Sprintf("%s/projects/%d", who.base(), check.ProjectID))
	})
}
