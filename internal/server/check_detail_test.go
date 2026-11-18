package server_test

import (
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
)

func (a *app) checkBody(t *testing.T, checkID int64) string {
	t.Helper()
	res := a.client.Get("/checks/" + itoa(checkID))
	wantStatus(t, res, http.StatusOK)
	return res.Body
}

func (a *app) insertPing(t *testing.T, checkID int64, kind models.PingKind, body string) {
	t.Helper()
	ip := "203.0.113.7"
	if err := a.store.InsertPing(t.Context(), store.NewPing{CheckID: checkID, Kind: kind, Body: body, SourceIP: &ip}, time.Now()); err != nil {
		t.Fatal(err)
	}
}

func TestCheckDetailShowsHeartbeatBodyAndSource(t *testing.T) {
	a, projectID := projectWith(t)
	checkID := a.addCheck(t, projectID, "backup", "cu")
	a.insertPing(t, checkID, models.PingFail, "boom trace")
	body := a.checkBody(t, checkID)
	for _, want := range []string{`class="beat"`, "boom trace", "Source", "203.0.113.7"} {
		if !strings.Contains(body, want) {
			t.Errorf("%q missing from the check page", want)
		}
	}
}

func TestPingTimestampsAreLocalizableWithUTCFallback(t *testing.T) {
	a, projectID := projectWith(t)
	checkID := a.addCheck(t, projectID, "backup", "cu2")
	a.insertPing(t, checkID, models.PingSuccess, "ok")
	body := a.checkBody(t, checkID)
	for message, want := range map[string]string{
		"a localizable timestamp":   `class="localtime" data-ts="`,
		"RFC3339 UTC in data-ts":    "+00:00",
		"a no-JS date labelled UTC": " UTC</span>",
	} {
		if !strings.Contains(body, want) {
			t.Errorf("missing %s (%q)", message, want)
		}
	}
}

func TestCheckDescriptionNeutralizesXSSPayloads(t *testing.T) {
	a, projectID := projectWith(t)
	period := int64(3600)
	checkID, err := a.store.CreateCheck(t.Context(), store.NewCheck{
		ProjectID: projectID, Name: "backup", PingUUID: "cu-xss", Kind: models.SchedulePeriod, PeriodSecs: &period,
		GraceSecs: 300, Timezone: "UTC", Description: "<img src=x onerror=alert(1)> and [x](javascript:alert(1))",
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	body := a.checkBody(t, checkID)
	if strings.Contains(body, "<img ") || strings.Contains(body, `href="javascript:`) {
		t.Fatalf("a raw tag or live javascript: href leaked: %s", body)
	}
	if !strings.Contains(body, "&lt;img") || !strings.Contains(body, `data-testid="check-description"`) {
		t.Fatalf("the escaped text and the description block must render: %s", body)
	}
}

// DueTime is the scheduler's; the tests inject a fixed deadline an hour out.
func (a *app) dueInAnHour() {
	a.deps.DueTime = func(models.Check) *time.Time {
		due := time.Now().Add(time.Hour + 30*time.Second)
		return &due
	}
}

func TestCheckDetailShowsWhenTheNextPingIsDue(t *testing.T) {
	a, projectID := projectWith(t)
	checkID := a.addCheck(t, projectID, "backup", "cu-due")
	next := time.Now().Add(time.Hour)
	a.markPing(t, checkID, store.PingMark{Status: models.StatusUp, NextDueAt: &next})
	a.dueInAnHour()
	body := a.checkBody(t, checkID)
	for _, want := range []string{`data-testid="check-next-due"`, "due in 1h", `check-next-due" title="`} {
		if !strings.Contains(body, want) {
			t.Errorf("%q missing: %s", want, body)
		}
	}
}

// A never-pinged check has a deadline, labelled as the first ping's.
func TestCheckDetailNextDueNamesTheFirstPingWhenNoneHasArrived(t *testing.T) {
	a, projectID := projectWith(t)
	checkID := a.addCheck(t, projectID, "backup", "cu-due-new")
	a.dueInAnHour()
	if body := a.checkBody(t, checkID); !strings.Contains(body, "first ping due in 1h") {
		t.Fatalf("a never-pinged check should name the first ping: %s", body)
	}
}

func TestCheckDetailPausedShowsNoDeadline(t *testing.T) {
	a, projectID := projectWith(t)
	checkID := a.addCheck(t, projectID, "backup", "cu-due-paused")
	if err := a.store.SetStatus(t.Context(), checkID, models.StatusPaused); err != nil {
		t.Fatal(err)
	}
	a.dueInAnHour()
	body := a.checkBody(t, checkID)
	if !strings.Contains(body, "not scheduled while paused") {
		t.Errorf("a paused check should name the state: %s", body)
	}
	if strings.Contains(body, "due in") {
		t.Error("a paused check must not count down to a deadline nothing enforces")
	}
}

// The strip is deliberately wider than any viewport (CSS clips from the left);
// pinned so it is not "optimised" down.
func TestTheHeartbeatRendersMoreBarsThanAViewportFits(t *testing.T) {
	a, projectID := projectWith(t)
	checkID := a.addCheck(t, projectID, "busy", "cu-busy")
	for range 150 {
		a.insertPing(t, checkID, models.PingStart, "")
		a.insertPing(t, checkID, models.PingSuccess, "")
	}
	body := a.checkBody(t, checkID)
	_, afterBeat, found := strings.Cut(body, `class="beat"`)
	if !found {
		t.Fatal("heartbeat strip missing")
	}
	strip, _, _ := strings.Cut(afterBeat, "</div>")
	if bars := strings.Count(strip, "<i "); bars != 120 {
		t.Fatalf("bars = %d, want the full 120", bars)
	}
	if strings.Contains(body, "30 runs ago") {
		t.Fatal("stale fixed-count caption")
	}
}
