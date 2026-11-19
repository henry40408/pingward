package server_test

import (
	"bufio"
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/ping"
	"github.com/henry40408/pingward/internal/server"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
	"github.com/henry40408/pingward/internal/web"
)

type fixture struct {
	projectID, checkID, channelID int64
}

// seed gives ownerID a project with one check and one channel.
func (a *app) seed(t *testing.T, ownerID int64, name string) fixture {
	t.Helper()
	ctx := t.Context()
	projectID, err := a.store.CreateProject(ctx, ownerID, store.ProjectSettings{Name: name}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	period := int64(60)
	checkID, err := a.store.CreateCheck(ctx, store.NewCheck{
		ProjectID: projectID, Name: name + "-check", PingUUID: "uuid-" + name, Kind: models.SchedulePeriod,
		PeriodSecs: &period, GraceSecs: 30, Timezone: "UTC",
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	channelID, err := a.store.CreateChannel(ctx, store.NewChannel{
		ProjectID: projectID, Kind: models.ChannelWebhook, Name: name + "-hook", ConfigJSON: `{"url":"http://example.invalid/x"}`,
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return fixture{projectID, checkID, channelID}
}

func (a *app) csrfHeader() http.Header {
	return http.Header{"X-Csrf-Token": {testutil.NewestSessionCSRF(a.client.T, a.store)}}
}

// Another user's resource is a 404 on every owner route, whatever the verb:
// a 403 would confirm the id exists.
func TestOwnerRoutesHideOtherUsersResources(t *testing.T) {
	a := newApp(t, nil)
	ownerID := a.addUser(t, "owner", false)
	a.addUser(t, "intruder", false)
	f := a.seed(t, ownerID, "private")
	a.login(t, "intruder")
	header := a.csrfHeader()
	for _, route := range []struct{ method, pattern string }{
		{"GET", "/projects/{id}"}, {"GET", "/projects/{id}/edit"}, {"POST", "/projects/{id}"},
		{"POST", "/projects/{id}/delete"}, {"GET", "/checks/{id}"}, {"GET", "/checks/{id}/pings"},
		{"GET", "/checks/{id}/events"}, {"GET", "/checks/{id}/notifications"}, {"POST", "/checks/{id}/pause"},
		{"POST", "/checks/{id}/resume"}, {"POST", "/checks/{id}/ack"}, {"POST", "/checks/{id}/regenerate"},
		{"POST", "/checks/{id}/delete"}, {"POST", "/checks/{id}/channels"},
		{"GET", "/projects/{pid}/channels/new"}, {"POST", "/projects/{pid}/channels"},
	} {
		id := f.checkID
		if strings.HasPrefix(route.pattern, "/projects/") {
			id = f.projectID
		}
		path := strings.NewReplacer("{id}", fmt.Sprint(id), "{pid}", fmt.Sprint(id)).Replace(route.pattern)
		res := a.client.Do(route.method, path, strings.NewReader(""), header)
		if res.StatusCode != http.StatusNotFound {
			t.Errorf("%s %s = %d, want 404", route.method, path, res.StatusCode)
		}
	}
	wantStatus(t, a.client.Get(fmt.Sprintf("/channels/%d/edit", f.channelID)), http.StatusNotFound)
	for _, path := range []string{"/channels/%d/delete", "/channels/%d/test", "/channels/%d"} {
		wantStatus(t, a.client.Do("POST", fmt.Sprintf(path, f.channelID), strings.NewReader(""), header), http.StatusNotFound)
	}
}

func TestOwnerSeesOwnProjectAndCheck(t *testing.T) {
	a := newApp(t, nil)
	ownerID := a.addUser(t, "owner", false)
	f := a.seed(t, ownerID, "mine")
	a.login(t, "owner")
	if res := a.client.Get("/"); res.StatusCode != 200 || !strings.Contains(res.Body, "mine") {
		t.Fatalf("dashboard: %d", res.StatusCode)
	}
	wantStatus(t, a.client.Get(fmt.Sprintf("/projects/%d", f.projectID)), 200)
	res := a.client.Get(fmt.Sprintf("/checks/%d", f.checkID))
	wantStatus(t, res, 200)
	if !strings.Contains(res.Body, "/ping/uuid-mine") {
		t.Fatal("the owner sees the ping URL")
	}
}

func TestAdminMustRevealOtherUsersPingURL(t *testing.T) {
	a := newApp(t, nil)
	ownerID := a.addUser(t, "owner", false)
	f := a.seed(t, ownerID, "theirs")
	a.loggedInAdmin(t)
	// Elevate through the unlock flow is covered elsewhere; here the admin door is gated.
	res := a.client.Get(fmt.Sprintf("/admin/checks/%d", f.checkID))
	if strings.Contains(res.Body, "uuid-theirs") {
		t.Fatal("an unrevealed admin view must not carry the ping UUID")
	}
}

func TestPauseResumeAckAndSetChannelsRedirectToCheckPage(t *testing.T) {
	a := newApp(t, nil)
	ownerID := a.addUser(t, "owner", false)
	f := a.seed(t, ownerID, "p")
	a.login(t, "owner")
	header := a.csrfHeader()
	page := fmt.Sprintf("/checks/%d", f.checkID)
	res := a.client.Do("POST", page+"/pause", strings.NewReader(""), header)
	wantStatus(t, res, http.StatusSeeOther)
	check, _ := a.store.FindCheck(t.Context(), f.checkID)
	if check.Status != models.StatusPaused {
		t.Fatalf("status = %s", check.Status)
	}
	wantStatus(t, a.client.Do("POST", page+"/resume", strings.NewReader(""), header), http.StatusSeeOther)
	check, _ = a.store.FindCheck(t.Context(), f.checkID)
	if check.Status != models.StatusNew {
		t.Fatalf("status = %s", check.Status)
	}
	form := url.Values{"channel_ids": {fmt.Sprint(f.channelID), "9999"}}
	res = a.client.Do("POST", page+"/channels", strings.NewReader(form.Encode()),
		http.Header{"X-Csrf-Token": header["X-Csrf-Token"], "Content-Type": {"application/x-www-form-urlencoded"}})
	wantStatus(t, res, http.StatusSeeOther)
	bound, _ := a.store.BoundChannelIDs(t.Context(), f.checkID)
	if len(bound) != 1 || bound[0] != f.channelID {
		t.Fatalf("only the project's own channel binds, got %v", bound)
	}
}

func TestDeleteAndRegenerateNeedConfirmation(t *testing.T) {
	a := newApp(t, nil)
	ownerID := a.addUser(t, "owner", false)
	f := a.seed(t, ownerID, "d")
	a.login(t, "owner")
	header := a.csrfHeader()
	page := fmt.Sprintf("/checks/%d", f.checkID)
	res := a.client.Do("POST", page+"/delete", strings.NewReader(""), header)
	wantStatus(t, res, 200)
	if check, _ := a.store.FindCheck(t.Context(), f.checkID); check == nil {
		t.Fatal("unconfirmed delete must not delete")
	}
	res = a.client.Do("POST", page+"/delete?confirmed=1", strings.NewReader(""), header)
	wantStatus(t, res, http.StatusSeeOther)
	if got := res.Header.Get("Location"); got != fmt.Sprintf("/projects/%d", f.projectID) {
		t.Fatalf("location = %s", got)
	}
}

// A hung-up client must free the stream at once, and a stopping server must
// not wait on it either.
func TestEventsStreamEndsWhenClientDisconnects(t *testing.T) {
	a := newApp(t, nil)
	ownerID := a.addUser(t, "owner", false)
	f := a.seed(t, ownerID, "s")
	a.login(t, "owner")
	res, err := a.client.HTTP.Get(a.client.Server.URL + fmt.Sprintf("/checks/%d/events", f.checkID))
	if err != nil {
		t.Fatal(err)
	}
	if ct := res.Header.Get("Content-Type"); ct != "text/event-stream" {
		t.Fatalf("content type = %s", ct)
	}
	waitFor(t, func() bool { return a.deps.Events.Subscribers() == 1 })
	a.deps.Events.Publish(f.checkID + 1000)
	a.deps.Events.Publish(f.checkID)
	line, err := bufio.NewReader(res.Body).ReadString('\n')
	if err != nil || line != "data: changed\n" {
		t.Fatalf("frame = %q, %v", line, err)
	}
	_ = res.Body.Close()
	waitFor(t, func() bool { return a.deps.Events.Subscribers() == 0 })
}

func TestEventsStreamEndsWhenServerStops(t *testing.T) {
	a := newApp(t, nil)
	stop := make(chan struct{})
	a.deps.Stopping = stop
	ownerID := a.addUser(t, "owner", false)
	f := a.seed(t, ownerID, "s")
	a.login(t, "owner")
	res, err := a.client.HTTP.Get(a.client.Server.URL + fmt.Sprintf("/checks/%d/events", f.checkID))
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = res.Body.Close() }()
	close(stop)
	done := make(chan struct{})
	go func() {
		_, _ = bufio.NewReader(res.Body).ReadString('\n')
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("stream must end when the server stops")
	}
}

func waitFor(t *testing.T, condition func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for !condition() {
		if time.Now().After(deadline) {
			t.Fatal("condition never held")
		}
		time.Sleep(10 * time.Millisecond)
	}
}

// Drives the real /ping/{uuid} handler, not Events.Publish, so a ping that
// stops publishing would fail here.
func TestOwnerReceivesChangedEventWhenTheCheckIsPinged(t *testing.T) {
	a := newApp(t, nil)
	ownerID := a.addUser(t, "owner", false)
	f := a.seed(t, ownerID, "s")
	pings := &ping.Handler{
		Store: a.store, Config: a.deps.Config, Events: a.deps.Events, Now: time.Now,
		DueTime: func(models.Check) *time.Time { return nil },
	}
	client := testutil.NewClient(t, server.New(a.deps, web.Routes(a.deps), pings.Mount))
	csrf := testutil.AnonymousCSRF(t, client, "/login")
	wantStatus(t, client.PostForm("/login", url.Values{"_csrf": {csrf}, "username": {"owner"}, "password": {testPassword}}), http.StatusSeeOther)

	// Subscribe first: publishing is gated on someone watching.
	res, err := client.HTTP.Get(client.Server.URL + fmt.Sprintf("/checks/%d/events", f.checkID))
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = res.Body.Close() }()
	if ct := res.Header.Get("Content-Type"); !strings.HasPrefix(ct, "text/event-stream") {
		t.Fatalf("content type = %q", ct)
	}
	waitFor(t, func() bool { return a.deps.Events.Subscribers() == 1 })
	wantStatus(t, client.Do("POST", "/ping/uuid-s", strings.NewReader("done"), nil), http.StatusOK)

	got := make(chan string, 1)
	go func() {
		line, _ := bufio.NewReader(res.Body).ReadString('\n')
		got <- line
	}()
	select {
	case line := <-got:
		if !strings.Contains(line, "changed") {
			t.Fatalf("frame = %q", line)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("the ping never reached the subscriber")
	}
}
