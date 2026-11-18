// Package ping receives check-ins: /ping/{uuid} and its /start, /fail, /log
// and /{exit code} variants, over GET and POST.
package ping

import (
	"context"
	"errors"
	"io"
	"net/http"
	"strconv"
	"time"

	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/config"
	"github.com/henry40408/pingward/internal/events"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/notify"
	"github.com/henry40408/pingward/internal/store"
)

const (
	// maxRequestBytes is axum's default body limit, which the Rust build relied on.
	maxRequestBytes = 2 << 20
	// maxStoredBodyBytes is how much of a ping body is kept.
	maxStoredBodyBytes = 10 * 1024
)

// Handler records pings and moves check state. DueTime is the scheduler's
// due-time rule, injected so this package does not depend on cron.
type Handler struct {
	Store   *store.Store
	Config  *config.Config
	Events  *events.Broker
	Now     func() time.Time
	DueTime func(check models.Check) *time.Time
	// Deliver sends an event to the check's channels. It runs in its own
	// goroutine, off the response path; nil disables delivery.
	Deliver func(ctx context.Context, event notify.Event, now time.Time)
}

// Mount registers the ping routes. Static segments beat {code} in the mux.
func (h *Handler) Mount(mux *http.ServeMux) {
	for _, method := range []string{"GET", "POST"} {
		mux.HandleFunc(method+" /ping/{uuid}", h.kind(models.PingSuccess))
		mux.HandleFunc(method+" /ping/{uuid}/fail", h.kind(models.PingFail))
		mux.HandleFunc(method+" /ping/{uuid}/start", h.kind(models.PingStart))
		mux.HandleFunc(method+" /ping/{uuid}/log", h.kind(models.PingLog))
		mux.HandleFunc(method+" /ping/{uuid}/{code}", h.exitCode)
	}
}

type request struct {
	uuid     string
	kind     models.PingKind
	exitCode *int64
}

func (h *Handler) kind(kind models.PingKind) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		h.apply(w, r, request{uuid: r.PathValue("uuid"), kind: kind})
	}
}

// exitCode answers 400 for a non-integer code before touching the database.
func (h *Handler) exitCode(w http.ResponseWriter, r *http.Request) {
	code, err := strconv.ParseInt(r.PathValue("code"), 10, 64)
	if err != nil {
		http.Error(w, "bad request", http.StatusBadRequest)
		return
	}
	kind := models.PingFail
	if code == 0 {
		kind = models.PingSuccess
	}
	h.apply(w, r, request{uuid: r.PathValue("uuid"), kind: kind, exitCode: &code})
}

func (h *Handler) apply(w http.ResponseWriter, r *http.Request, req request) {
	body, err := io.ReadAll(http.MaxBytesReader(w, r.Body, maxRequestBytes))
	if err != nil {
		var tooLarge *http.MaxBytesError
		if errors.As(err, &tooLarge) {
			http.Error(w, "payload too large", http.StatusRequestEntityTooLarge)
			return
		}
		http.Error(w, "bad request", http.StatusBadRequest)
		return
	}
	// A client that hangs up (curl -m) must not cancel the writes halfway.
	ctx := context.WithoutCancel(r.Context())
	check, err := h.Store.FindCheckByUUID(ctx, req.uuid)
	if err != nil {
		http.Error(w, "internal error", http.StatusInternalServerError)
		return
	}
	if check == nil {
		http.Error(w, "not found", http.StatusNotFound)
		return
	}
	if err := h.record(ctx, r, *check, req, body); err != nil {
		http.Error(w, "internal error", http.StatusInternalServerError)
		return
	}
	w.WriteHeader(http.StatusOK)
}

func (h *Handler) record(ctx context.Context, r *http.Request, check models.Check, req request, body []byte) error {
	now := h.Now()
	if len(body) > maxStoredBodyBytes {
		body = body[:maxStoredBodyBytes]
	}
	var sourceIP *string
	if ip, ok := auth.ClientIP(r.Header, auth.PeerAddr(r.RemoteAddr), h.Config.TrustedProxies); ok {
		sourceIP = &ip
	}
	err := h.Store.InsertPing(ctx, store.NewPing{
		CheckID: check.ID, Kind: req.kind, ExitCode: req.exitCode, Body: lossyUTF8(body), SourceIP: sourceIP,
	}, now)
	if err != nil {
		return err
	}
	// Live-tail signal; free when nobody is watching.
	if h.Events.Subscribers() > 0 {
		h.Events.Publish(check.ID)
	}
	// A paused check's ping is recorded but must not move it to up/down.
	if check.Status == models.StatusPaused {
		return nil
	}
	return h.transition(ctx, check, req, now)
}

// transition applies the state change. check is the pre-ping snapshot: an up
// notification reports the ping before the recovery.
func (h *Handler) transition(ctx context.Context, check models.Check, req request, now time.Time) error {
	switch req.kind {
	case models.PingSuccess:
		return h.markUp(ctx, check, now)
	case models.PingFail:
		return h.markDown(ctx, check, req.exitCode, now)
	case models.PingStart:
		return h.Store.MarkPing(ctx, check.ID, store.PingMark{
			Status: check.Status, LastStartAt: &now, NextDueAt: check.NextDueAt,
		})
	}
	return nil
}

func (h *Handler) markUp(ctx context.Context, check models.Check, now time.Time) error {
	updated := check
	updated.LastPingAt = &now
	err := h.Store.MarkPing(ctx, check.ID, store.PingMark{
		Status: models.StatusUp, LastPingAt: &now, NextDueAt: h.DueTime(updated),
	})
	if err != nil || check.Status != models.StatusDown {
		return err
	}
	if err := h.Store.ClearNag(ctx, check.ID); err != nil {
		return err
	}
	h.deliverAsync(check, models.EventUp, nil, now)
	return nil
}

func (h *Handler) markDown(ctx context.Context, check models.Check, exitCode *int64, now time.Time) error {
	err := h.Store.MarkPing(ctx, check.ID, store.PingMark{Status: models.StatusDown, LastPingAt: &now})
	if err != nil || (check.Status != models.StatusUp && check.Status != models.StatusNew) {
		return err
	}
	if err := h.Store.BeginDownAlert(ctx, check.ID, now); err != nil {
		return err
	}
	cause := notify.DownCause{Kind: notify.CauseFailed, ExitCode: exitCode}
	h.deliverAsync(check, models.EventDown, &cause, now)
	return nil
}

// deliverAsync keeps notification I/O (and the project lookup) off the ping
// response path. The goroutine is not tracked, and outlives the request.
func (h *Handler) deliverAsync(check models.Check, kind models.EventKind, cause *notify.DownCause, now time.Time) {
	if h.Deliver == nil {
		return
	}
	go func() {
		ctx := context.Background()
		var projectName *string
		if project, err := h.Store.FindProject(ctx, check.ProjectID); err == nil && project != nil {
			projectName = &project.Name
		}
		detail := notify.DetailFromCheck(check, projectName, h.Config.BaseURL)
		if timezone := h.Store.DisplayTimezone(ctx); timezone != "" {
			detail = detail.WithDisplayTimezone(&timezone)
		}
		if cause != nil {
			detail = detail.WithCause(*cause)
		}
		h.Deliver(ctx, notify.Event{
			CheckID: check.ID, CheckName: check.Name, Kind: kind, At: now,
			ProjectID: check.ProjectID, Detail: detail,
		}, now)
	}()
}
