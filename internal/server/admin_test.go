package server_test

import (
	"fmt"
	"net/http"
	"net/url"
	"regexp"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
	"github.com/henry40408/pingward/internal/web"
)

var adminPathParams = strings.NewReplacer("{id}", "1", "{pid}", "1", "{handle}", "x")

func adminRoutes(a *app) []web.Route {
	var routes []web.Route
	for _, route := range web.Routes(a.deps) {
		if strings.HasPrefix(route.Pattern, "/admin") {
			routes = append(routes, route)
		}
	}
	return routes
}

// The route table is data, so a new admin route cannot dodge this test.
func TestNonAdminIsForbiddenOnEveryAdminRoute(t *testing.T) {
	a, _ := memberApp(t)
	routes := adminRoutes(a)
	if len(routes) < 25 {
		t.Fatalf("only %d admin routes: the walk would pass vacuously", len(routes))
	}
	header := a.csrfHeader()
	for _, route := range routes {
		path := adminPathParams.Replace(route.Pattern)
		res := a.client.Do(route.Method, path, strings.NewReader("_=1"), http.Header{
			"X-Csrf-Token": header["X-Csrf-Token"], "Content-Type": {"application/x-www-form-urlencoded"},
		})
		if res.StatusCode != http.StatusForbidden || !strings.Contains(res.Body, "admin only") {
			t.Errorf("%s %s = %d %q, want 403 admin only", route.Method, path, res.StatusCode, res.Body)
		}
	}
}

func TestAnonymousVisitorsAreSentToLoginFromEveryAdminGET(t *testing.T) {
	a := newApp(t, nil)
	for _, route := range adminRoutes(a) {
		if route.Method != "GET" {
			continue
		}
		res := a.client.Get(adminPathParams.Replace(route.Pattern))
		if res.StatusCode != http.StatusSeeOther || res.Header.Get("Location") != "/login" {
			t.Errorf("GET %s = %d %s", route.Pattern, res.StatusCode, res.Header.Get("Location"))
		}
	}
}

func TestAdminSeesTheAdminLinkAndAMemberDoesNot(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	if !strings.Contains(a.client.Get("/").Body, `href="/admin"`) {
		t.Fatal("an admin must see the Admin link")
	}
	member, _ := memberApp(t)
	if strings.Contains(member.client.Get("/").Body, `href="/admin"`) {
		t.Fatal("a member must not")
	}
}

func TestAdminPageRendersFigures(t *testing.T) {
	a, memberID := adminApp(t)
	a.seed(t, memberID, "shop")
	res := a.client.Get("/admin")
	wantStatus(t, res, http.StatusOK)
	for _, want := range []string{"Check health", "Notification health", "Audit trail", "shop", "member"} {
		if !strings.Contains(res.Body, want) {
			t.Errorf("/admin is missing %q", want)
		}
	}
}

func TestOnlyAdminsReachTheDashboardOverview(t *testing.T) {
	a, _ := memberApp(t)
	wantStatus(t, a.client.Get("/admin"), http.StatusForbidden)
}

func (a *app) seedAudit(t *testing.T, adminID int64, count int) {
	t.Helper()
	at := time.Now().Add(-time.Hour)
	for i := range count {
		action := "admin.access"
		if i%2 == 1 {
			action = "user.create"
		}
		if _, err := a.store.RecordAudit(t.Context(), store.NewAudit{ActorUserID: adminID, ActorUsername: "admin", Action: action}, at.Add(time.Duration(i)*time.Second)); err != nil {
			t.Fatal(err)
		}
	}
}

func (a *app) auditFragment(t *testing.T, path string) *testutil.Response {
	t.Helper()
	res := a.client.Do("GET", path, nil, fetchHeader)
	wantStatus(t, res, http.StatusOK)
	return res
}

func rowCount(body string) int { return strings.Count(body, `data-testid="audit-row"`) }

func TestAdminAuditEmptyState(t *testing.T) {
	a, _ := adminApp(t)
	body := a.auditFragment(t, "/admin/audit").Body
	if !strings.Contains(body, "No audit entries yet.") && rowCount(body) == 0 {
		t.Fatal("an empty trail needs its empty state")
	}
}

func TestAdminAuditFragmentFiltersByActionAndFilteredEmptyDiffers(t *testing.T) {
	a, _ := adminApp(t)
	a.seedAudit(t, 1, 24)
	if n := rowCount(a.auditFragment(t, "/admin/audit?aaction=admin.access").Body); n != 12 {
		t.Fatalf("filtered rows = %d, want 12", n)
	}
	empty := a.auditFragment(t, "/admin/audit?aaction=nothing.like.this").Body
	if !strings.Contains(empty, "No audit entries match the filter.") {
		t.Fatalf("filtered empty state missing: %s", empty)
	}
}

var olderHref = regexp.MustCompile(`href="(/admin/audit\?ab=[^"]+)"`)

func TestAdminAuditPagesAndTheCursorCarriesTheFilter(t *testing.T) {
	a, _ := adminApp(t)
	a.seedAudit(t, 1, 48)
	first := a.auditFragment(t, "/admin/audit?aaction=admin.access").Body
	if n := rowCount(first); n != 20 {
		t.Fatalf("first page = %d rows", n)
	}
	match := olderHref.FindStringSubmatch(first)
	if match == nil || !strings.Contains(match[1], "aaction=admin.access") {
		t.Fatalf("the Older link must carry the filter: %v", match)
	}
	if n := rowCount(a.auditFragment(t, strings.ReplaceAll(match[1], "&amp;", "&")).Body); n != 4 {
		t.Fatalf("second page = %d rows, want 4", n)
	}
}

func TestAnAuditFragmentWithoutFetchRedirectsToTheAnchor(t *testing.T) {
	a, _ := adminApp(t)
	res := a.client.Get("/admin/audit?aaction=x")
	wantLocation(t, res, "/admin?aaction=x#audit-section")
	wantStatus(t, a.client.Get("/admin/audit?ab=nope"), http.StatusBadRequest)
}

func settingsForm(overrides map[string]string) url.Values {
	form := url.Values{
		"scan_interval": {""}, "nag_interval": {""}, "pings_retention_days": {""},
		"notifications_retention_days": {""}, "audit_retention_days": {""}, "display_timezone": {""},
	}
	for key, value := range overrides {
		form.Set(key, value)
	}
	return form
}

func TestSettingsSaveIsAuditedWithTheChangedKeys(t *testing.T) {
	a, _ := adminApp(t)
	wantLocation(t, a.post(t, "/admin/settings", settingsForm(map[string]string{"scan_interval": "5m", "audit_retention_days": "90"})), "/admin")
	entry := a.lastAudit(t, "settings.update")
	if entry.Detail == nil || *entry.Detail != "scan_interval=300 audit_retention_days=90" {
		t.Fatalf("detail = %v", entry.Detail)
	}
	if entry.Method == nil || *entry.Method != "POST" || entry.Path == nil || *entry.Path != "/admin/settings" {
		t.Fatalf("audit = %+v", entry)
	}
	page := a.client.Get("/admin").Body
	if !strings.Contains(page, "Settings saved.") || !strings.Contains(page, `value="5m"`) {
		t.Fatal("the flash and the readable duration must show")
	}
}

func TestSettingsSaveWithNoChangesWritesNoAudit(t *testing.T) {
	a, _ := adminApp(t)
	wantLocation(t, a.post(t, "/admin/settings", settingsForm(nil)), "/admin")
	if got := a.auditActions(t); len(got) != 0 {
		t.Fatalf("audit = %v", got)
	}
}

func TestSettingsSaveRecordsAClearedValueAsUnset(t *testing.T) {
	a, _ := adminApp(t)
	wantLocation(t, a.post(t, "/admin/settings", settingsForm(map[string]string{"pings_retention_days": "30"})), "/admin")
	wantLocation(t, a.post(t, "/admin/settings", settingsForm(nil)), "/admin")
	if detail := a.lastAudit(t, "settings.update").Detail; detail == nil || *detail != "pings_retention_days=unset" {
		t.Fatalf("detail = %v", detail)
	}
}

func TestInvalidSettingsReRenderTheInputAndWriteNothing(t *testing.T) {
	a, _ := adminApp(t)
	for message, form := range map[string]map[string]string{
		"Global scan interval must be a positive duration": {"scan_interval": "soon"},
		"Pings retention must be a positive integer":       {"scan_interval": "5m", "pings_retention_days": "0"},
	} {
		res := a.post(t, "/admin/settings", settingsForm(form))
		wantStatus(t, res, http.StatusOK)
		if !strings.Contains(res.Body, message) {
			t.Errorf("missing %q", message)
		}
	}
	if got, _ := a.store.GetSetting(t.Context(), "scan_interval"); got != nil {
		t.Fatalf("an invalid save must be atomic, scan_interval = %q", *got)
	}
}

func (a *app) revealTarget(t *testing.T) (checkID int64, path string) {
	t.Helper()
	memberID := a.userByName(t, "member").ID
	f := a.seed(t, memberID, "private")
	return f.checkID, fmt.Sprintf("/admin/checks/%d", f.checkID)
}

func TestAdminCheckPageWithholdsAnotherUsersPingURL(t *testing.T) {
	a, _ := adminApp(t)
	_, path := a.revealTarget(t)
	body := a.client.Get(path).Body
	if strings.Contains(body, "uuid-private") || !strings.Contains(body, "reveal-ping-url") {
		t.Fatal("the URL must be withheld behind the reveal control")
	}
	if slicesContain(a.auditActions(t), "admin.ping_url_reveal") {
		t.Fatal("opening the page is not a disclosure")
	}
}

func slicesContain(items []string, want string) bool {
	for _, item := range items {
		if item == want {
			return true
		}
	}
	return false
}

func TestAdminRevealingAnotherUsersPingURLIsAudited(t *testing.T) {
	a, _ := adminApp(t)
	checkID, path := a.revealTarget(t)
	res := a.post(t, path+"/ping-url", url.Values{})
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "uuid-private") {
		t.Fatal("the reveal must show the URL")
	}
	entry := a.lastAudit(t, "admin.ping_url_reveal")
	if entry.TargetID == nil || *entry.TargetID != checkID || entry.TargetOwnerID == nil ||
		*entry.TargetOwnerID != a.userByName(t, "member").ID || entry.Path == nil || *entry.Path != path+"/ping-url" {
		t.Fatalf("audit = %+v", entry)
	}
	// A reveal is a POST on the admin door, but it audits itself only once.
	if n := strings.Count(strings.Join(a.auditActions(t), ","), "admin.access"); n != 0 {
		t.Fatalf("the reveal must not also record admin.access (%d)", n)
	}
}

func TestAdminSeesTheirOwnPingURLWithoutRevealing(t *testing.T) {
	a, _ := adminApp(t)
	f := a.seed(t, a.userByName(t, "admin").ID, "mine")
	path := "/admin/checks/" + strconv.FormatInt(f.checkID, 10)
	if body := a.client.Get(path).Body; !strings.Contains(body, "uuid-mine") {
		t.Fatal("one's own URL needs no reveal")
	}
	a.post(t, path+"/ping-url", url.Values{})
	if slicesContain(a.auditActions(t), "admin.ping_url_reveal") {
		t.Fatal("revealing one's own URL discloses nothing")
	}
}

func TestAdminMutatingAnotherUsersCheckIsStillAudited(t *testing.T) {
	a, _ := adminApp(t)
	checkID, path := a.revealTarget(t)
	wantStatus(t, a.post(t, path+"/pause", url.Values{}), http.StatusSeeOther)
	entry := a.lastAudit(t, "admin.access")
	if entry.TargetType == nil || *entry.TargetType != "check" || entry.TargetID == nil || *entry.TargetID != checkID {
		t.Fatalf("audit = %+v", entry)
	}
}

func TestAdminReadingAnotherUsersProjectIsNotAudited(t *testing.T) {
	a, memberID := adminApp(t)
	f := a.seed(t, memberID, "private")
	wantStatus(t, a.client.Get(fmt.Sprintf("/admin/projects/%d", f.projectID)), http.StatusOK)
	if got := a.auditActions(t); len(got) != 0 {
		t.Fatalf("a GET must not be audited: %v", got)
	}
}

func TestAdminDeletesAnotherUsersProjectAndLandsOnAdmin(t *testing.T) {
	a, memberID := adminApp(t)
	f := a.seed(t, memberID, "private")
	path := fmt.Sprintf("/admin/projects/%d/delete?confirmed=1", f.projectID)
	wantLocation(t, a.post(t, path, url.Values{}), "/admin")
	if entry := a.lastAudit(t, "admin.access"); entry.TargetOwnerID == nil || *entry.TargetOwnerID != memberID {
		t.Fatalf("audit = %+v", entry)
	}
}
