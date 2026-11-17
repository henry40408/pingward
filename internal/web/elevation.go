package web

import (
	"net/http"

	"github.com/henry40408/pingward/internal/duration"
	"github.com/henry40408/pingward/internal/elevate"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/ratelimit"
	"github.com/henry40408/pingward/internal/views"
)

// elevation is the admin's standing for access-granting actions.
type elevation struct {
	// notApplicable: no local password, so there is nothing to re-assert.
	notApplicable bool
	remainingSecs int64
	unlocked      bool
}

// allows is whether an access-granting admin action may proceed.
func (e elevation) allows() bool { return e.notApplicable || e.unlocked }

func (d *Deps) elevationOf(r *http.Request, user *models.User) elevation {
	if user.PasswordHash == nil {
		return elevation{notApplicable: true}
	}
	handle, ok := d.currentSessionHandle(r)
	if !ok {
		return elevation{}
	}
	secs, unlocked := d.Elevations.RemainingSecs(handle, d.Now())
	return elevation{remainingSecs: secs, unlocked: unlocked}
}

// adminLocked bounces to the /admin/unlock interstitial instead of answering
// 403. The refused action is not replayed after unlocking.
func (d *Deps) adminLocked(w http.ResponseWriter, r *http.Request) {
	http.SetCookie(w, d.flashCookie("admin_locked"))
	redirect(w, r, "/admin/unlock")
}

func readableSecs(secs int64) string { return duration.Format(secs) }

type unlockNotice struct {
	bouncedFlash string
	err          string
}

func (d *Deps) renderAdminUnlock(w http.ResponseWriter, r *http.Request, admin *models.User, notice unlockNotice) {
	standing := d.elevationOf(r, admin)
	view := views.AdminUnlockView{
		IsAdmin: true, CSRF: d.csrfToken(r), Applies: !standing.notApplicable,
		TTL: readableSecs(int64(elevate.TTL.Seconds())), BouncedFlash: notice.bouncedFlash, Error: notice.err,
	}
	if standing.unlocked {
		view.Remaining = readableSecs(standing.remainingSecs)
	}
	writeHTML(w, http.StatusOK, views.AdminUnlock(view))
}

func (d *Deps) adminUnlockPage(w http.ResponseWriter, r *http.Request) {
	admin, ok := d.requireAdmin(w, r)
	if !ok {
		return
	}
	notice := unlockNotice{}
	if message, taken := d.takeFlash(w, r, "admin_locked"); taken {
		notice.bouncedFlash = message
	}
	d.renderAdminUnlock(w, r, admin, notice)
}

// grantElevation is a no-op without a session handle; passwordless accounts
// already pass through elevation.notApplicable.
func (d *Deps) grantElevation(r *http.Request, userID int64) {
	if handle, ok := d.currentSessionHandle(r); ok {
		d.Elevations.Grant(handle, userID, d.Now())
	}
}

// adminUnlock opens the access-granting controls for elevate.TTL. It goes
// through reauthenticate so wrong passwords are metered here too.
func (d *Deps) adminUnlock(w http.ResponseWriter, r *http.Request) {
	admin, ok := d.requireAdmin(w, r)
	if !ok {
		return
	}
	outcome := d.reauthenticate(admin, r.PostFormValue("password"), "admin_unlock")
	// The JS dialog wants a status, not HTML. Presentation only; already decided.
	if wantsFragment(r) {
		d.answerUnlockFetch(w, r, admin, outcome)
		return
	}
	if refusal := reauthRefusal(outcome, "That password is not correct."); refusal != "" {
		d.renderAdminUnlock(w, r, admin, unlockNotice{err: refusal})
		return
	}
	d.grantElevation(r, admin.ID)
	http.SetCookie(w, d.flashCookie("admin_unlocked"))
	redirect(w, r, "/admin")
}

func (d *Deps) answerUnlockFetch(w http.ResponseWriter, r *http.Request, admin *models.User, outcome reauthOutcome) {
	switch outcome {
	case reauthPassed:
		d.grantElevation(r, admin.ID)
		w.WriteHeader(http.StatusNoContent)
	case reauthFailed:
		w.WriteHeader(http.StatusForbidden)
	default:
		setRetryAfter(w, int64(ratelimit.AccountWindow.Seconds()))
		w.WriteHeader(http.StatusTooManyRequests)
	}
}
