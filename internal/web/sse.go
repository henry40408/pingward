package web

import (
	"fmt"
	"net/http"
	"time"
)

// sseKeepAlive is axum's KeepAlive default: a bare comment frame.
const sseKeepAlive = 15 * time.Second

// changedFrame is data-less: the browser re-fetches the fragments.
const changedFrame = "data: changed\n\n"

// checkEvents streams a "changed" frame per ping or scan transition of one
// check. It ends when the client hangs up or the server starts stopping, so a
// graceful Shutdown never waits on it.
func (d *Deps) checkEvents(admin bool) http.HandlerFunc {
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
		d.streamCheckEvents(w, r, check.ID)
	}
}

func (d *Deps) streamCheckEvents(w http.ResponseWriter, r *http.Request, checkID int64) {
	flusher := http.NewResponseController(w)
	changes, cancel := d.Events.Subscribe()
	defer cancel()
	w.Header().Set("Content-Type", "text/event-stream")
	w.WriteHeader(http.StatusOK)
	if flusher.Flush() != nil {
		return
	}
	keepAlive := time.NewTicker(sseKeepAlive)
	defer keepAlive.Stop()
	for {
		var frame string
		select {
		case <-r.Context().Done():
			return
		case <-d.Stopping:
			return
		case <-keepAlive.C:
			frame = ":\n\n"
		case id := <-changes:
			if !sawCheck(id, checkID, changes) {
				continue
			}
			frame = changedFrame
		}
		if _, err := fmt.Fprint(w, frame); err != nil || flusher.Flush() != nil {
			return
		}
	}
}

// sawCheck coalesces a backlog: one frame covers every pending id, since each
// frame only says "refresh". It is true when any id, first or queued, is checkID.
func sawCheck(first, checkID int64, pending <-chan int64) bool {
	matched := first == checkID
	for {
		select {
		case id := <-pending:
			matched = matched || id == checkID
		default:
			return matched
		}
	}
}
