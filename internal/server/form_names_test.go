package server_test

import (
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"testing"
)

func projectForm(name string) url.Values {
	return url.Values{"name": {name}, "description": {""}, "scan_interval_secs": {""}, "nag_interval_secs": {""}}
}

func (a *app) storedProjectName(t *testing.T, projectID int64) string {
	t.Helper()
	project, err := a.store.FindProject(t.Context(), projectID)
	if err != nil || project == nil {
		t.Fatalf("project %d: %v", projectID, err)
	}
	return project.Name
}

func TestProjectCreateStoresATrimmedName(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	res := a.post(t, "/projects", projectForm("  Nightly jobs  "))
	wantStatus(t, res, http.StatusSeeOther)
	id, err := strconv.ParseInt(res.Header.Get("Location")[strings.LastIndex(res.Header.Get("Location"), "/")+1:], 10, 64)
	if err != nil {
		t.Fatalf("the redirect should point at /projects/{id}: %v", err)
	}
	if got := a.storedProjectName(t, id); got != "Nightly jobs" {
		t.Fatalf("stored name = %q", got)
	}
}

func TestProjectUpdateStoresATrimmedName(t *testing.T) {
	a, projectID := projectWith(t)
	wantStatus(t, a.post(t, "/projects/"+itoa(projectID), projectForm("  Renamed jobs  ")), http.StatusSeeOther)
	if got := a.storedProjectName(t, projectID); got != "Renamed jobs" {
		t.Fatalf("stored name = %q", got)
	}
}

// The admin route shares validation but is a separate handler.
func TestAdminProjectUpdateStoresATrimmedName(t *testing.T) {
	a, projectID := projectWith(t)
	wantStatus(t, a.post(t, "/admin/projects/"+itoa(projectID), projectForm("  Admin renamed  ")), http.StatusSeeOther)
	if got := a.storedProjectName(t, projectID); got != "Admin renamed" {
		t.Fatalf("stored name = %q", got)
	}
}
