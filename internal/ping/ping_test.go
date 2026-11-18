package ping

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/events"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/notify"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

// periodDue is the scheduler's period rule (anchor + period + grace). The cron
// rule needs the cron evaluator, which is not ported yet; the handler only
// sees it through Handler.DueTime.
func periodDue(check models.Check) *time.Time {
	anchor := check.CreatedAt
	if check.LastPingAt != nil {
		anchor = *check.LastPingAt
	}
	due := anchor.Add(time.Duration(*check.PeriodSecs+check.GraceSecs) * time.Second)
	return &due
}

type fixture struct {
	handler *Handler
	store   *store.Store
	mux     *http.ServeMux
	events  *events.Broker
	sent    chan notify.Event
}

func newFixture(t *testing.T, env map[string]string) *fixture {
	t.Helper()
	st := testutil.NewStore(t)
	ctx := context.Background()
	userID, err := st.CreateUser(ctx, "u", nil, false, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	projectID, err := st.CreateProject(ctx, userID, store.ProjectSettings{Name: "p"}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	period := int64(60)
	if _, err := st.CreateCheck(ctx, store.NewCheck{
		ProjectID: projectID, Name: "job", PingUUID: "abc", Kind: models.SchedulePeriod,
		PeriodSecs: &period, GraceSecs: 30, Timezone: "UTC",
	}, time.Now()); err != nil {
		t.Fatal(err)
	}
	f := &fixture{store: st, mux: http.NewServeMux(), events: events.NewBroker(), sent: make(chan notify.Event, 4)}
	f.handler = &Handler{
		Store: st, Config: testutil.TestConfig(env), Events: f.events, Now: time.Now, DueTime: periodDue,
		Deliver: func(_ context.Context, event notify.Event, _ time.Time) { f.sent <- event },
	}
	f.handler.Mount(f.mux)
	return f
}

func (f *fixture) send(method, path, body string) *httptest.ResponseRecorder {
	return f.sendFrom(method, path, body, "192.0.2.1:5000", nil)
}

func (f *fixture) sendFrom(method, path, body, remote string, headers http.Header) *httptest.ResponseRecorder {
	request := httptest.NewRequest(method, path, strings.NewReader(body))
	request.RemoteAddr = remote
	for name, values := range headers {
		request.Header[name] = values
	}
	recorder := httptest.NewRecorder()
	f.mux.ServeHTTP(recorder, request)
	return recorder
}

func (f *fixture) check(t *testing.T) *models.Check {
	t.Helper()
	check, err := f.store.FindCheckByUUID(context.Background(), "abc")
	if err != nil || check == nil {
		t.Fatalf("check abc: %v %v", check, err)
	}
	return check
}

func (f *fixture) wantStatus(t *testing.T, recorder *httptest.ResponseRecorder, want int) {
	t.Helper()
	if recorder.Code != want {
		t.Fatalf("status = %d, want %d (%s)", recorder.Code, want, recorder.Body.String())
	}
}

func TestSuccessPingMarksUpAndRecords(t *testing.T) {
	f := newFixture(t, nil)
	f.wantStatus(t, f.send("POST", "/ping/abc", "done"), 200)
	check := f.check(t)
	if check.Status != models.StatusUp || check.LastPingAt == nil || check.NextDueAt == nil {
		t.Fatalf("got %+v", check)
	}
}

func TestFailPingMarksDown(t *testing.T) {
	f := newFixture(t, nil)
	f.wantStatus(t, f.send("POST", "/ping/abc/fail", ""), 200)
	if got := f.check(t).Status; got != models.StatusDown {
		t.Fatalf("status = %s", got)
	}
}

func TestUnknownUUIDIs404(t *testing.T) {
	f := newFixture(t, nil)
	recorder := f.send("GET", "/ping/does-not-exist", "")
	f.wantStatus(t, recorder, 404)
	if recorder.Body.String() != "not found\n" {
		t.Errorf("body = %q", recorder.Body.String())
	}
}

func TestNonZeroExitCodeMarksDownAndZeroMarksUp(t *testing.T) {
	f := newFixture(t, nil)
	f.wantStatus(t, f.send("POST", "/ping/abc/1", ""), 200)
	if got := f.check(t).Status; got != models.StatusDown {
		t.Fatalf("exit 1: status = %s", got)
	}
	f.wantStatus(t, f.send("POST", "/ping/abc/0", ""), 200)
	if got := f.check(t); got.Status != models.StatusUp || got.LastPingAt == nil {
		t.Fatalf("exit 0: %+v", got)
	}
}

// A non-integer code is refused before any lookup, so it cannot probe for uuids.
func TestNonIntegerCodeIs400EvenForAnUnknownUUID(t *testing.T) {
	f := newFixture(t, nil)
	f.wantStatus(t, f.send("POST", "/ping/abc/oops", ""), 400)
	f.wantStatus(t, f.send("POST", "/ping/nope/oops", ""), 400)
}

func TestStartDoesNotResetTheDueClock(t *testing.T) {
	f := newFixture(t, nil)
	f.send("POST", "/ping/abc", "")
	before := f.check(t)
	f.wantStatus(t, f.send("POST", "/ping/abc/start", ""), 200)
	after := f.check(t)
	if after.Status != models.StatusUp || !after.LastPingAt.Equal(*before.LastPingAt) ||
		!after.NextDueAt.Equal(*before.NextDueAt) || after.LastStartAt == nil {
		t.Fatalf("before %+v after %+v", before, after)
	}
}

func TestLogRecordsOnly(t *testing.T) {
	f := newFixture(t, nil)
	f.wantStatus(t, f.send("POST", "/ping/abc/log", "hello"), 200)
	check := f.check(t)
	if check.Status != models.StatusNew || check.LastPingAt != nil {
		t.Fatalf("log must not change state: %+v", check)
	}
}

func TestGetVerbWorksForSuccess(t *testing.T) {
	f := newFixture(t, nil)
	f.wantStatus(t, f.send("GET", "/ping/abc", ""), 200)
	if got := f.check(t).Status; got != models.StatusUp {
		t.Fatalf("status = %s", got)
	}
}

func TestAPausedCheckIsRecordedButNotResurrected(t *testing.T) {
	f := newFixture(t, nil)
	if err := f.store.SetStatus(context.Background(), f.check(t).ID, models.StatusPaused); err != nil {
		t.Fatal(err)
	}
	f.wantStatus(t, f.send("POST", "/ping/abc", "done"), 200)
	f.wantStatus(t, f.send("POST", "/ping/abc/fail", ""), 200)
	if got := f.check(t).Status; got != models.StatusPaused {
		t.Fatalf("status = %s, want paused", got)
	}
	pings, err := f.store.ListRecentPings(context.Background(), f.check(t).ID, 10)
	if err != nil || len(pings) != 2 {
		t.Fatalf("both pings must be recorded: %v %v", pings, err)
	}
	select {
	case event := <-f.sent:
		t.Fatalf("a paused check must not notify: %+v", event)
	case <-time.After(100 * time.Millisecond):
	}
}

func TestOnlyThePingBodyPrefixIsStoredAndOversizedRequestsAre413(t *testing.T) {
	f := newFixture(t, nil)
	f.wantStatus(t, f.send("POST", "/ping/abc/log", strings.Repeat("a", 20000)), 200)
	pings, _ := f.store.ListRecentPings(context.Background(), f.check(t).ID, 1)
	if len(pings) != 1 || len(pings[0].Body) != maxStoredBodyBytes {
		t.Fatalf("stored body length = %d, want %d", len(pings[0].Body), maxStoredBodyBytes)
	}
	f.wantStatus(t, f.send("POST", "/ping/abc/log", strings.Repeat("a", maxRequestBytes+1)), 413)
}

func TestTheFirstDownPingNotifiesWithTheExitCodeCause(t *testing.T) {
	f := newFixture(t, nil)
	f.send("POST", "/ping/abc/7", "")
	event := f.wantEvent(t)
	if event.Kind != models.EventDown || event.Detail.Cause == nil ||
		event.Detail.Cause.Kind != notify.CauseFailed || *event.Detail.Cause.ExitCode != 7 {
		t.Fatalf("got %+v", event)
	}
	// Already down: a second failure is not a new alert.
	f.send("POST", "/ping/abc/fail", "")
	select {
	case again := <-f.sent:
		t.Fatalf("a repeat failure must not notify: %+v", again)
	case <-time.After(100 * time.Millisecond):
	}
}

// The up notification carries the snapshot taken before the ping updated the check.
func TestRecoveryNotifiesUpWithThePreRecoverySnapshot(t *testing.T) {
	f := newFixture(t, nil)
	f.send("POST", "/ping/abc/fail", "")
	f.wantEvent(t)
	f.send("POST", "/ping/abc", "")
	event := f.wantEvent(t)
	if event.Kind != models.EventUp {
		t.Fatalf("got %+v", event)
	}
	if event.Detail.LastPingAt == nil || time.Since(*event.Detail.LastPingAt) < 0 {
		t.Fatalf("snapshot LastPingAt = %v", event.Detail.LastPingAt)
	}
	if f.check(t).Acknowledged {
		t.Error("recovery clears the nag state")
	}
}

func (f *fixture) wantEvent(t *testing.T) notify.Event {
	t.Helper()
	select {
	case event := <-f.sent:
		return event
	case <-time.After(5 * time.Second):
		t.Fatal("no notification delivered")
		return notify.Event{}
	}
}

func TestAPingPublishesTheCheckIDOnlyWhenSomeoneIsWatching(t *testing.T) {
	f := newFixture(t, nil)
	f.send("POST", "/ping/abc/log", "")
	updates, cancel := f.events.Subscribe()
	defer cancel()
	f.send("POST", "/ping/abc/log", "")
	select {
	case id := <-updates:
		if id != f.check(t).ID {
			t.Fatalf("published %d", id)
		}
	case <-time.After(time.Second):
		t.Fatal("subscriber saw no event")
	}
	if len(updates) != 0 {
		t.Fatal("the unwatched ping must not have been queued")
	}
}

func sourceIP(t *testing.T, env map[string]string, remote string, headers http.Header) string {
	t.Helper()
	f := newFixture(t, env)
	f.wantStatus(t, f.sendFrom("GET", "/ping/abc", "", remote, headers), 200)
	pings, err := f.store.ListRecentPings(context.Background(), f.check(t).ID, 1)
	if err != nil || len(pings) != 1 || pings[0].SourceIP == nil {
		t.Fatalf("the ping should be recorded with an address: %v %v", pings, err)
	}
	return *pings[0].SourceIP
}

func TestPingBehindATrustedProxyRecordsTheForwardedClient(t *testing.T) {
	env := map[string]string{"PINGWARD_TRUSTED_PROXIES": "172.16.0.0/12"}
	forwarded := http.Header{"X-Forwarded-For": {"203.0.113.7, 172.18.0.5"}}
	if got := sourceIP(t, env, "172.18.0.5:44321", forwarded); got != "203.0.113.7" {
		t.Fatalf("got %s", got)
	}
}

func TestPingFromAnUntrustedPeerRecordsThePeer(t *testing.T) {
	env := map[string]string{"PINGWARD_TRUSTED_PROXIES": "172.16.0.0/12"}
	forwarded := http.Header{"X-Forwarded-For": {"203.0.113.7"}}
	if got := sourceIP(t, env, "8.8.8.8:44321", forwarded); got != "8.8.8.8" {
		t.Fatalf("got %s", got)
	}
}

func TestPingWithoutTrustedProxiesRecordsThePeer(t *testing.T) {
	forwarded := http.Header{"X-Forwarded-For": {"203.0.113.7"}}
	if got := sourceIP(t, nil, "172.18.0.5:44321", forwarded); got != "172.18.0.5" {
		t.Fatalf("got %s", got)
	}
}

// A client that times out and hangs up must not abort the state change.
func TestPingStillRecordsAfterTheClientDisconnects(t *testing.T) {
	f := newFixture(t, nil)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	request := httptest.NewRequestWithContext(ctx, "POST", "/ping/abc/fail", nil)
	request.RemoteAddr = "192.0.2.1:5000"
	recorder := httptest.NewRecorder()
	f.mux.ServeHTTP(recorder, request)
	f.wantStatus(t, recorder, 200)
	if got := f.check(t).Status; got != models.StatusDown {
		t.Fatalf("status = %s, want down", got)
	}
}

func TestPanickingDeliveryIsLoggedAndTheResponseStillSucceeds(t *testing.T) {
	f := newFixture(t, nil)
	logger, logs := testutil.JSONLogger()
	f.handler.Logger = logger
	f.handler.Deliver = func(context.Context, notify.Event, time.Time) { panic("channel exploded") }
	f.wantStatus(t, f.send("POST", "/ping/abc/fail", ""), 200)
	deadline := time.Now().Add(5 * time.Second)
	for !strings.Contains(logs.String(), "channel exploded") {
		if time.Now().After(deadline) {
			t.Fatal("panic was not logged")
		}
		time.Sleep(10 * time.Millisecond)
	}
}

func TestTheDownNotificationNamesTheProjectAndUsesTheDisplayTimezone(t *testing.T) {
	f := newFixture(t, nil)
	if err := f.store.SetSetting(context.Background(), "display_timezone", "Asia/Taipei"); err != nil {
		t.Fatal(err)
	}
	f.send("POST", "/ping/abc/fail", "")
	event := f.wantEvent(t)
	if event.Detail.ProjectName == nil || *event.Detail.ProjectName != "p" {
		t.Fatalf("project name = %v, want p", event.Detail.ProjectName)
	}
	if event.Detail.Timezone == nil || *event.Detail.Timezone != "Asia/Taipei" {
		t.Fatalf("timezone = %v, want the display timezone", event.Detail.Timezone)
	}
}
