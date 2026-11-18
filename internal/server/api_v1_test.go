package server_test

import (
	"fmt"
	"net/http"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/apikey"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
)

func TestMissingBearerIs401JSON(t *testing.T) {
	a := newApp(t, nil)
	res := a.api("GET", "/api/v1/projects", "", nil)
	wantStatus(t, res, 401)
	if errorCode(t, res) != "unauthorized" {
		t.Fatal(res.Body)
	}
}

func TestBogusBearerIs401(t *testing.T) {
	a := newApp(t, nil)
	wantStatus(t, a.api("GET", "/api/v1/projects", "pw_not_a_real_key", nil), 401)
}

func TestListsOnlyCallersOwnProjects(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	a.newProject(t, uid, "mine")
	other, _ := a.apiUser(t, "bob", false)
	a.newProject(t, other, "theirs")
	res := a.api("GET", "/api/v1/projects", token, nil)
	wantStatus(t, res, 200)
	list := decode[[]obj](t, res)
	if len(list) != 1 || list[0]["name"] != "mine" || list[0]["owner_id"] != float64(uid) {
		t.Fatalf("list = %v", list)
	}
}

func TestMemberCannotReadAnotherUsersProject(t *testing.T) {
	a := newApp(t, nil)
	_, token := a.apiUser(t, "alice", false)
	other, _ := a.apiUser(t, "bob", false)
	pid := a.newProject(t, other, "theirs")
	wantStatus(t, a.api("GET", fmt.Sprintf("/api/v1/projects/%d", pid), token, nil), 404)
}

func TestAdminKeyReadsCrossUserAndWritesAudit(t *testing.T) {
	a := newApp(t, nil)
	_, token := a.apiUser(t, "root", true)
	other, _ := a.apiUser(t, "bob", false)
	pid := a.newProject(t, other, "theirs")
	res := a.api("GET", fmt.Sprintf("/api/v1/projects/%d", pid), token, nil)
	wantStatus(t, res, 200)
	if decode[obj](t, res)["name"] != "theirs" {
		t.Fatal(res.Body)
	}
	audits, err := a.store.ListAudit(t.Context(), 10)
	if err != nil || len(audits) != 1 {
		t.Fatalf("audits = %v, %v", audits, err)
	}
	got := audits[0]
	if got.Action != "admin.api.access" || *got.TargetType != "project" || *got.TargetID != pid || *got.TargetOwnerID != other {
		t.Fatalf("audit = %+v", got)
	}
}

func TestAdminReadingOwnProjectDoesNotAudit(t *testing.T) {
	a := newApp(t, nil)
	admin, token := a.apiUser(t, "root", true)
	pid := a.newProject(t, admin, "mine")
	wantStatus(t, a.api("GET", fmt.Sprintf("/api/v1/projects/%d", pid), token, nil), 200)
	if audits, _ := a.store.ListAudit(t.Context(), 10); len(audits) != 0 {
		t.Fatalf("audits = %v", audits)
	}
}

func (a *app) insertPings(t *testing.T, checkID int64, count int) {
	t.Helper()
	for range count {
		err := a.store.InsertPing(t.Context(), store.NewPing{CheckID: checkID, Kind: models.PingSuccess}, time.Now())
		if err != nil {
			t.Fatal(err)
		}
	}
}

func TestCheckPingsArePaginatedNewestFirst(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	cid := a.newCheck(t, a.newProject(t, uid, "p"), "job", "uuid-pg")
	a.insertPings(t, cid, 25)
	path := fmt.Sprintf("/api/v1/checks/%d/pings", cid)

	res := a.api("GET", path, token, nil)
	wantStatus(t, res, 200)
	page := decode[obj](t, res)
	items := page["items"].([]any)
	lastID := items[19].(obj)["id"]
	if len(items) != 20 || page["has_older"] != true || page["has_newer"] != false || page["next_before"] != lastID || page["next_after"] != nil {
		t.Fatalf("first page = %s", res.Body)
	}

	page2 := decode[obj](t, a.api("GET", fmt.Sprintf("%s?before=%v", path, lastID), token, nil))
	items2 := page2["items"].([]any)
	if len(items2) != 5 || page2["has_newer"] != true || page2["has_older"] != false ||
		page2["next_after"] != items2[0].(obj)["id"] || page2["next_before"] != nil {
		t.Fatalf("second page = %v", page2)
	}
}

func TestLimitIsClamped(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	cid := a.newCheck(t, a.newProject(t, uid, "p"), "job", "uuid-lim")
	a.insertPings(t, cid, 3)
	res := a.api("GET", fmt.Sprintf("/api/v1/checks/%d/pings?limit=0", cid), token, nil)
	if items := decode[obj](t, res)["items"].([]any); len(items) != 1 {
		t.Fatalf("limit=0 gave %d items", len(items))
	}
}

func TestChannelDTONeverLeaksConfigSecrets(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	secret := "https://hooks.example.com/SECRET-TOKEN"
	chid := a.newChannel(t, a.newProject(t, uid, "p"), models.ChannelWebhook, "hook", `{"url":"`+secret+`"}`)
	res := a.api("GET", fmt.Sprintf("/api/v1/channels/%d", chid), token, nil)
	wantStatus(t, res, 200)
	if strings.Contains(res.Body, secret) || strings.Contains(res.Body, "config_json") {
		t.Fatalf("config leaked: %s", res.Body)
	}
	if decode[obj](t, res)["kind"] != "webhook" {
		t.Fatal(res.Body)
	}
}

func TestNotificationsEndpointReturnsEvents(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	pid := a.newProject(t, uid, "p")
	cid := a.newCheck(t, pid, "job", "uuid-nt")
	chid := a.newChannel(t, pid, models.ChannelWebhook, "hook", "{}")
	err := a.store.RecordNotification(t.Context(), store.NewNotification{
		CheckID: cid, ChannelID: chid, Event: models.EventDown, Status: models.NotifyOK,
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	res := a.api("GET", fmt.Sprintf("/api/v1/checks/%d/notifications", cid), token, nil)
	wantStatus(t, res, 200)
	items := decode[obj](t, res)["items"].([]any)
	if len(items) != 1 || items[0].(obj)["event"] != "down" || items[0].(obj)["status"] != "ok" {
		t.Fatal(res.Body)
	}
}

func TestDisabledUsersKeyIsRejected(t *testing.T) {
	a := newApp(t, nil)
	uid, token := a.apiUser(t, "alice", false)
	if err := a.store.SetUserDisabled(t.Context(), uid, true); err != nil {
		t.Fatal(err)
	}
	wantStatus(t, a.api("GET", "/api/v1/projects", token, nil), 401)
}

func TestExpiredKeyIsRejected(t *testing.T) {
	a := newApp(t, nil)
	uid := a.addUser(t, "alice", false)
	key := apikey.Generate()
	expired := time.Now().Add(-time.Hour)
	_, err := a.store.InsertAPIKey(t.Context(), store.NewAPIKey{UserID: uid, Name: "old", TokenHash: key.Hash, Prefix: key.Prefix, ExpiresAt: &expired}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	wantStatus(t, a.api("GET", "/api/v1/projects", key.Full, nil), 401)
}

func TestKeysEndpointIsSelfScoped(t *testing.T) {
	a := newApp(t, nil)
	_, token := a.apiUser(t, "alice", false)
	a.apiUser(t, "bob", false)
	res := a.api("GET", "/api/v1/keys", token, nil)
	wantStatus(t, res, 200)
	list := decode[[]obj](t, res)
	if len(list) != 1 || list[0]["token_hash"] != nil || !strings.HasPrefix(list[0]["prefix"].(string), "pw_") {
		t.Fatal(res.Body)
	}
}

func TestDocsRequireALoggedInSession(t *testing.T) {
	a := newApp(t, nil)
	for _, path := range []string{"/api/openapi.json", "/api/docs"} {
		res := a.client.Get(path)
		wantStatus(t, res, 303)
		if res.Header.Get("Location") != "/login" {
			t.Errorf("%s redirects to %q", path, res.Header.Get("Location"))
		}
	}
}

func TestDocsAreServedToALoggedInUser(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "member", false)
	a.login(t, "member")
	spec := a.client.Get("/api/openapi.json")
	wantStatus(t, spec, 200)
	if info := decode[obj](t, spec)["info"].(obj); info["title"] != "pingward API" {
		t.Fatal(info)
	}
	docs := a.client.Get("/api/docs")
	wantStatus(t, docs, 200)
	if !strings.Contains(strings.ToLower(docs.Body), "scalar") {
		t.Fatal("docs page does not mention scalar")
	}
}

func TestEveryAPIV1RouteRequiresABearerKey(t *testing.T) {
	a := newApp(t, nil)
	routes := apiRoutes(a)
	if len(routes) < 20 {
		t.Fatalf("only %d routes", len(routes))
	}
	for _, route := range routes {
		// No body: the key check must run before body parsing, or this would 400.
		res := a.api(route.Method, strings.Replace(route.Pattern, "{id}", "1", 1), "", nil)
		if res.StatusCode != http.StatusUnauthorized {
			t.Errorf("%s %s: status %d, want 401", route.Method, route.Pattern, res.StatusCode)
		}
	}
}

var minimalBodies = map[string]obj{
	"PATCH /api/v1/projects/{id}":         {"name": "x"},
	"POST /api/v1/projects/{id}/checks":   {"name": "x"},
	"POST /api/v1/projects/{id}/channels": {"name": "x", "kind": "webhook"},
	"PATCH /api/v1/checks/{id}":           {"name": "x"},
	"PUT /api/v1/checks/{id}/channels":    {"channel_ids": []int{}},
	"PATCH /api/v1/channels/{id}":         {},
}

func TestMemberCannotReachAnotherUsersResourceOnAnyAPIRoute(t *testing.T) {
	a := newApp(t, nil)
	owner, ownerToken := a.apiUser(t, "alice", false)
	_, token := a.apiUser(t, "mallory", false)
	var routes []struct{ Method, Pattern string }
	for _, route := range apiRoutes(a) {
		if strings.Contains(route.Pattern, "{id}") {
			routes = append(routes, struct{ Method, Pattern string }{route.Method, route.Pattern})
		}
	}
	if len(routes) < 15 {
		t.Fatalf("only %d parameterised routes", len(routes))
	}
	for i, route := range routes {
		pid := a.newProject(t, owner, fmt.Sprintf("alice-project-%d", i))
		cid := a.newCheck(t, pid, fmt.Sprintf("alice-check-%d", i), fmt.Sprintf("alice-uuid-%d", i))
		chid := a.newChannel(t, pid, models.ChannelWebhook, fmt.Sprintf("alice-channel-%d", i), "{}")
		target := pid
		if strings.Contains(route.Pattern, "/checks/") {
			target = cid
		} else if strings.Contains(route.Pattern, "/channels/") {
			target = chid
		}
		path := idPath(route.Pattern, target)
		var body any
		if minimal, ok := minimalBodies[route.Method+" "+route.Pattern]; ok {
			body = minimal
		}
		if got := a.api(route.Method, path, token, body).StatusCode; got != 404 {
			t.Errorf("%s %s as non-owner: %d, want 404", route.Method, route.Pattern, got)
		}
		if got := a.api(route.Method, path, ownerToken, body).StatusCode; got == 404 {
			t.Errorf("%s %s: the owner got 404 too, so the control proves nothing", route.Method, route.Pattern)
		}
	}
}

func TestAPIWritesNeedNoCSRFTokenAndCarryNoCacheControl(t *testing.T) {
	a := newApp(t, nil)
	_, token := a.apiUser(t, "alice", false)
	res := a.api("POST", "/api/v1/projects", token, obj{"name": "x"})
	wantStatus(t, res, 201)
	if res.Header.Get("Cache-Control") != "" {
		t.Errorf("API response carries Cache-Control %q", res.Header.Get("Cache-Control"))
	}
}

func TestAPIDocsAreNotCacheable(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "member", false)
	a.login(t, "member")
	for _, path := range []string{"/api/docs", "/api/openapi.json"} {
		res := a.client.Get(path)
		wantStatus(t, res, 200)
		if got := res.Header.Get("Cache-Control"); got != "no-store" {
			t.Errorf("%s Cache-Control = %q, want no-store", path, got)
		}
		if res.Header.Get("Content-Security-Policy") != "" {
			t.Errorf("%s carries the browser UI's CSP", path)
		}
	}
	// A no-store hoisted from the docs routes to every API route would hit a 401 too.
	res := a.api("GET", "/api/v1/projects", "", nil)
	wantStatus(t, res, 401)
	if _, present := res.Header["Cache-Control"]; present {
		t.Errorf("/api/v1 must not set Cache-Control, got %q", res.Header.Get("Cache-Control"))
	}
}

// The stateless captures of the Rust API must match byte for byte (status,
// content type, body); the others depend on a database this test does not seed.
func TestStatelessErrorsMatchTheRustCaptures(t *testing.T) {
	a := newApp(t, nil)
	_, token := a.apiUser(t, "alice", false)
	for _, tc := range []struct{ file, token string }{
		{"no-auth", ""}, {"bad-key", "pw_bad"}, {"project-create-invalid", token}, {"project-bad-id", token},
	} {
		capture := readCapture(t, tc.file)
		res := a.api(capture.method, capture.path, tc.token, capture.body)
		if res.StatusCode != capture.status || res.Header.Get("Content-Type") != capture.contentType || res.Body != capture.body2 {
			t.Errorf("%s: got %d %q %q\nwant %d %q %q", tc.file, res.StatusCode, res.Header.Get("Content-Type"), res.Body,
				capture.status, capture.contentType, capture.body2)
		}
	}
}

type capture struct {
	method, path, contentType, body2 string
	body                             any
	status                           int
}

func readCapture(t *testing.T, name string) capture {
	t.Helper()
	raw, err := os.ReadFile("../api/testdata/responses/" + name + ".http")
	if err != nil {
		t.Fatal(err)
	}
	head, responseBody, _ := strings.Cut(string(raw), "\n\n")
	lines := strings.Split(head, "\n")
	var c capture
	c.method, c.path, _ = strings.Cut(lines[0], " ")
	c.path = strings.TrimSpace(c.path)
	for _, line := range lines[1:] {
		switch {
		case strings.HasPrefix(line, "body: "):
			c.body = strings.TrimPrefix(line, "body: ")
		case strings.HasPrefix(line, "HTTP/1.1 "):
			fmt.Sscanf(line, "HTTP/1.1 %d", &c.status)
		case strings.HasPrefix(line, "content-type: "):
			c.contentType = strings.TrimPrefix(line, "content-type: ")
		}
	}
	c.body2 = strings.TrimRight(responseBody, "\n")
	return c
}
