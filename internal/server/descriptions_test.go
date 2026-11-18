package server_test

import (
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/store"
)

func TestCheckDescriptionMarkdownRendersOnCheckPage(t *testing.T) {
	a, projectID := projectWith(t)
	checkID := a.addDescribedCheck(t, projectID, "backup", "Runs **nightly** at 2am.", "cu-md")
	wantContains(t, a.checkBody(t, checkID), "<strong>nightly</strong>", "check description markdown")
}

func TestProjectAndCheckDescriptionsRenderOnProjectPage(t *testing.T) {
	a, projectID := projectWith(t)
	err := a.store.UpdateProject(t.Context(), projectID, store.ProjectSettings{
		Name: "web", Description: "A **bold** project description.\n\n- alpha item\n- beta item",
	})
	if err != nil {
		t.Fatal(err)
	}
	a.addDescribedCheck(t, projectID, "backup", "Runs *nightly* backups.", "cu-desc")
	body := a.projectBody(t, projectID)
	for _, want := range []string{
		"<strong>bold</strong>", "<ul>", "<li>alpha item</li>", "<li>beta item</li>",
		`class="cdesc" data-testid="check-description-summary">Runs nightly backups.</div>`,
	} {
		wantContains(t, body, want, "project page")
	}
	wantAbsent(t, body, "Runs *nightly*", "the check row strips markdown markers")
}

func TestDashboardShowsTruncatedDescriptionsWithMarkdownStripped(t *testing.T) {
	a := dashboardApp(t)
	projectID, err := a.store.CreateProject(t.Context(), a.userByName(t, "admin").ID,
		store.ProjectSettings{Name: "web", Description: "**Web** services project"}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	a.addDescribedCheck(t, projectID, "backup", "**Nightly** backups of the primary database run every day and verify "+
		"checksum integrity end to end, catching silent corruption early before it can spread "+
		"further into downstream systems and backups.", "cu-long-desc")
	a.addCheck(t, projectID, "idle", "cu-empty-desc")
	body := a.dashboardBody(t, "")

	// A literal, so a broken truncation cannot agree with itself: 120 characters and an ellipsis.
	const expected = "Nightly backups of the primary database run every day and verify checksum integrity end to end, catching silent corrupti…"
	if got := len([]rune(expected)); got != 121 {
		t.Fatalf("fixture is %d characters", got)
	}
	wantContains(t, body, `class="gdesc">Web services project</span>`, "project description stripped")
	wantAbsent(t, body, "**Web**", "markers leaked into gdesc")
	wantContains(t, body, `class="cdesc" data-testid="check-description-summary">`+expected+`</div>`, "truncated and stripped")
	wantAbsent(t, body, "downstream systems and backups", "the untruncated tail")
	wantAbsent(t, body, "**Nightly**", "markers leaked into cdesc")
	if n := strings.Count(body, `data-testid="check-description-summary"`); n != 1 {
		t.Fatalf("the empty-description check must not render a cdesc element, found %d", n)
	}
}
