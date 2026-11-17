package web

import (
	"net/http"
	"strconv"

	"github.com/henry40408/pingward/internal/store"
)

// adminCheckRevealPingURL is the one audited admin read: it discloses a
// credential. A POST, not ?reveal=1, so the URL cannot be seen without
// passing through here. Resolving the check does not audit it (POST would),
// so the reveal audits itself as admin.ping_url_reveal.
func (d *Deps) adminCheckRevealPingURL(w http.ResponseWriter, r *http.Request) {
	admin, ok := d.requireAdmin(w, r)
	if !ok {
		return
	}
	id, ok := pathID(w, r, "id")
	if !ok {
		return
	}
	query, ok := readHistoryQuery(w, r)
	if !ok {
		return
	}
	check, err := d.Store.FindCheck(r.Context(), id)
	if err != nil {
		internalError(w)
		return
	}
	if check == nil {
		notFound(w)
		return
	}
	project, err := d.Store.FindProject(r.Context(), check.ProjectID)
	if err != nil {
		internalError(w)
		return
	}
	var ownerID *int64
	if project != nil {
		ownerID = &project.UserID
	}
	// One's own check discloses nothing; the control is not rendered for it.
	if ownerID == nil || *ownerID != admin.ID {
		if err := d.auditPingURLReveal(r, admin.ID, admin.Username, check.ID, ownerID); err != nil {
			internalError(w)
			return
		}
	}
	d.renderCheckPage(w, r, access{user: admin, admin: true}, check, query, checkPageOptions{pingURLRevealed: true})
}

func (d *Deps) auditPingURLReveal(r *http.Request, adminID int64, adminName string, checkID int64, ownerID *int64) error {
	targetType, method := "check", http.MethodPost
	path := "/admin/checks/" + strconv.FormatInt(checkID, 10) + "/ping-url"
	_, err := d.Store.RecordAudit(writeCtx(r), store.NewAudit{
		ActorUserID: adminID, ActorUsername: adminName, Action: "admin.ping_url_reveal",
		TargetType: &targetType, TargetID: &checkID, TargetOwnerID: ownerID, Method: &method, Path: &path,
	}, d.Now())
	return err
}
