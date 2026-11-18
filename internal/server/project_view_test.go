package server_test

import (
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
)

// projectWith is a signed-in admin who owns one empty project.
func projectWith(t *testing.T) (a *app, projectID int64) {
	t.Helper()
	a = newApp(t, nil)
	a.loggedInAdmin(t)
	projectID, err := a.store.CreateProject(t.Context(), a.userByName(t, "admin").ID, store.ProjectSettings{Name: "web"}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return a, projectID
}

func (a *app) addCheck(t *testing.T, projectID int64, name, uuid string) int64 {
	t.Helper()
	period := int64(3600)
	id, err := a.store.CreateCheck(t.Context(), store.NewCheck{
		ProjectID: projectID, Name: name, PingUUID: uuid, Kind: models.SchedulePeriod,
		PeriodSecs: &period, GraceSecs: 300, Timezone: "UTC",
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return id
}

func (a *app) markPing(t *testing.T, checkID int64, mark store.PingMark) {
	t.Helper()
	if err := a.store.MarkPing(t.Context(), checkID, mark); err != nil {
		t.Fatal(err)
	}
}

func (a *app) projectBody(t *testing.T, projectID int64) string {
	t.Helper()
	res := a.client.Get("/projects/" + itoa(projectID))
	wantStatus(t, res, http.StatusOK)
	return res.Body
}

func TestProjectPageShowsRunningBadgeForInFlightCheck(t *testing.T) {
	a, projectID := projectWith(t)
	checkID := a.addCheck(t, projectID, "backup", "cu")
	started := time.Now()
	a.markPing(t, checkID, store.PingMark{Status: models.StatusNew, LastStartAt: &started})
	body := a.projectBody(t, projectID)
	for _, want := range []string{`class="badge running"`, `class="status-dot running"`} {
		if !strings.Contains(body, want) {
			t.Errorf("%s missing on the project page", want)
		}
	}
}

// Both directions, so a view that always (or never) emits the chip fails.
func TestProjectPageNoChannelChipReflectsBindingState(t *testing.T) {
	a, projectID := projectWith(t)
	unbound := a.addCheck(t, projectID, "no-chan", "cu-unbound")
	bound := a.addCheck(t, projectID, "has-chan", "cu-bound")
	channelID, err := a.store.CreateChannel(t.Context(), store.NewChannel{
		ProjectID: projectID, Kind: models.ChannelWebhook, Name: "hook", ConfigJSON: `{"url":"http://x"}`,
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if err := a.store.BindChannel(t.Context(), bound, channelID); err != nil {
		t.Fatal(err)
	}
	body := a.projectBody(t, projectID)
	const chip = `data-testid="check-no-channel"`
	if n := strings.Count(body, chip); n != 1 {
		t.Fatalf("%d rows carry the chip, want exactly the unbound one", n)
	}
	for _, row := range strings.Split(body, `class="check"`) {
		switch {
		case strings.Contains(row, "/checks/"+itoa(unbound)+`"`) && !strings.Contains(row, chip):
			t.Error("the unbound check's row must carry the chip")
		case strings.Contains(row, "/checks/"+itoa(bound)+`"`) && strings.Contains(row, chip):
			t.Error("the bound check's row must not carry the chip")
		}
	}
}

// The page must use the display status, not the raw stored one.
func TestProjectPageShowsLateForAStoredUpCheckInTheGraceWindow(t *testing.T) {
	a, projectID := projectWith(t)
	checkID := a.addCheck(t, projectID, "backup", "cu")
	now := time.Now()
	last, due := now.Add(-4000*time.Second), now.Add(120*time.Second)
	a.markPing(t, checkID, store.PingMark{Status: models.StatusUp, LastPingAt: &last, NextDueAt: &due})
	body := a.projectBody(t, projectID)
	for _, want := range []string{`class="badge late"`, `class="status-dot late"`} {
		if !strings.Contains(body, want) {
			t.Errorf("%s missing: the raw stored status would show up", want)
		}
	}
}

func TestProjectDescriptionNeutralizesXSSPayloads(t *testing.T) {
	a, projectID := projectWith(t)
	err := a.store.UpdateProject(t.Context(), projectID, store.ProjectSettings{
		Name: "web", Description: "<img src=x onerror=alert(1)> and [x](javascript:alert(1))",
	})
	if err != nil {
		t.Fatal(err)
	}
	body := a.projectBody(t, projectID)
	if strings.Contains(body, "<img ") || strings.Contains(body, `href="javascript:`) {
		t.Fatalf("a raw tag or live javascript: href leaked: %s", body)
	}
	// The escaped payload still appears as text, so asserting on "onerror"
	// would be the wrong check.
	if !strings.Contains(body, "&lt;img") || !strings.Contains(body, `data-testid="project-description"`) {
		t.Fatalf("the escaped text and the description block must render: %s", body)
	}
}

func TestProjectPageListsChannelsWithoutTheirSecrets(t *testing.T) {
	a, projectID := projectWith(t)
	const secret = "https://hooks.example.com/SECRET-TOKEN"
	channelID, err := a.store.CreateChannel(t.Context(), store.NewChannel{
		ProjectID: projectID, Kind: models.ChannelWebhook, Name: "on-call hook", ConfigJSON: `{"url":"` + secret + `"}`,
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	body := a.projectBody(t, projectID)
	for _, want := range []string{"on-call hook", `class="kind">webhook</span>`, `data-testid="channel-edit-` + itoa(channelID) + `"`} {
		if !strings.Contains(body, want) {
			t.Errorf("%q missing from the project page", want)
		}
	}
	for _, leaked := range []string{secret, "config_json"} {
		if strings.Contains(body, leaked) {
			t.Errorf("%q must never reach the project page", leaked)
		}
	}
}
