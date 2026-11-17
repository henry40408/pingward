package server_test

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/ratelimit"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

// Ports of tests/auth_web.rs: setup, login, session and cookie behaviour.

const firstAdminPassword = "a first admin passphrase"

func (a *app) postSetup(t *testing.T, token *string) *testutil.Response {
	t.Helper()
	form := url.Values{
		"_csrf":    {testutil.AnonymousCSRF(t, a.client, "/setup")},
		"username": {"admin"}, "password": {firstAdminPassword},
	}
	if token != nil {
		form.Set("setup_token", *token)
	}
	return a.client.PostForm("/setup", form)
}

func tokenApp(t *testing.T) *app {
	t.Helper()
	a := newApp(t, nil)
	a.deps.SetupToken.Arm("ABCDE-12345")
	return a
}

func ptr(s string) *string { return &s }

func TestSetupCreatesAdminThenDashboardLoads(t *testing.T) {
	a := newApp(t, nil)
	a.deps.SetupToken.Arm(testutil.TestSetupToken)
	wantLocation(t, a.client.Get("/"), "/setup")
	csrf := testutil.AnonymousCSRF(t, a.client, "/setup")
	res := a.client.PostForm("/setup", url.Values{
		"_csrf": {csrf}, "setup_token": {testutil.TestSetupToken},
		"username": {"admin"}, "password": {firstAdminPassword},
	})
	wantStatus(t, res, http.StatusSeeOther)
	if countUsers(t, a) != 1 || !a.userByName(t, "admin").IsAdmin {
		t.Fatal("the first user must be an admin")
	}
	wantStatus(t, a.client.Get("/"), http.StatusOK)
}

// The idle window slides forward on use; expires_at is set near expiry in the
// database so the test needs no clock.
func TestAnActiveSessionSurvivesPastTheOriginalTTL(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	nearExpiry := time.Now().Add(time.Minute)
	_, err := a.store.DB.ExecContext(t.Context(), "UPDATE sessions SET expires_at = $1", nearExpiry.UTC().Format(time.RFC3339Nano))
	if err != nil {
		t.Fatal(err)
	}
	wantStatus(t, a.client.Get("/"), http.StatusOK)
	var stored string
	if err := a.store.DB.QueryRowContext(t.Context(), "SELECT expires_at FROM sessions").Scan(&stored); err != nil {
		t.Fatal(err)
	}
	expiresAt, err := time.Parse(time.RFC3339Nano, stored)
	if err != nil {
		t.Fatal(err)
	}
	if !expiresAt.After(nearExpiry.Add(time.Hour)) {
		t.Fatalf("expires_at must have slid forward past its near-expiry value, got %v", expiresAt)
	}
}

func TestDisablingUserInvalidatesSession(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	wantStatus(t, a.client.Get("/"), http.StatusOK)
	admin := a.userByName(t, "admin")
	if err := a.store.SetUserDisabled(t.Context(), admin.ID, true); err != nil {
		t.Fatal(err)
	}
	if _, err := a.store.DeleteSessionsForUser(t.Context(), admin.ID); err != nil {
		t.Fatal(err)
	}
	wantBouncedToLogin(t, a.client.Get("/projects/new"))
	if sessionRows(t, a) != 0 {
		t.Fatal("no session may remain")
	}
}

func TestDisabledUserCannotLogIn(t *testing.T) {
	a := newApp(t, nil)
	id := a.addUser(t, "bob", false)
	if err := a.store.SetUserDisabled(t.Context(), id, true); err != nil {
		t.Fatal(err)
	}
	res := a.attemptLogin(t, "bob", testPassword)
	wantStatus(t, res, http.StatusOK)
	if sessionRows(t, a) != 0 {
		t.Fatal("a disabled account must not get a session row")
	}
}

func TestCreateAndDeleteProject(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	adminID := a.adminID(t)
	token := testutil.NewestSessionCSRF(t, a.store)
	wantStatus(t, a.client.PostForm("/projects", withToken(newProjectForm(), token)), http.StatusSeeOther)
	projects, err := a.store.ListProjectsForUser(t.Context(), adminID)
	if err != nil || len(projects) != 1 {
		t.Fatalf("projects = %v (%v)", projects, err)
	}
	path := "/projects/" + itoa(projects[0].ID)
	wantStatus(t, a.client.Get(path), http.StatusOK)
	wantStatus(t, a.postToken(path+"/delete?confirmed=1"), http.StatusSeeOther)
	if projects, _ := a.store.ListProjectsForUser(t.Context(), adminID); len(projects) != 0 {
		t.Fatal("the project must be deleted")
	}
}

func TestCannotViewAnotherUsersProject(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	other := a.addUser(t, "other", false)
	f := a.seed(t, other, "secret")
	wantStatus(t, a.client.Get("/projects/"+itoa(f.projectID)), http.StatusNotFound)
}

func TestSetupPageUsesAuthCard(t *testing.T) {
	a := newApp(t, nil)
	res := a.client.Get("/setup")
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, `class="auth"`) {
		t.Fatal("the setup page must use the auth card")
	}
}

func TestLoginPageUsesAuthCardAndErrorIsRestyled(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "bob", false)
	res := a.client.Get("/login")
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, `class="auth"`) {
		t.Fatal("the login page must use the auth card")
	}
	res = a.attemptLogin(t, "bob", "nope")
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "invalid username or password") || !strings.Contains(res.Body, `class="flash err"`) {
		t.Fatalf("missing the restyled error banner: %s", res.Body)
	}
}

func TestLoginPageBouncesAnAlreadySignedInVisitor(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "bob", false)
	a.login(t, "bob")
	wantLocation(t, a.client.Get("/login"), "/")
}

func TestLoginLogoutCycle(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "bob", false)
	wantStatus(t, a.attemptLogin(t, "bob", "nope"), http.StatusOK)
	login := a.attemptLogin(t, "bob", testPassword)
	wantStatus(t, login, http.StatusSeeOther)
	loginCookie := sessionCookieSet(login)
	wantStatus(t, a.client.Get("/"), http.StatusOK)

	logout := a.postToken("/logout")
	wantStatus(t, logout, http.StatusSeeOther)
	removal := sessionCookieSet(logout)
	if removal == nil || removal.Path != "/" || removal.Secure != loginCookie.Secure {
		t.Fatalf("the removal cookie must match the original's Path and Secure: %v", removal)
	}
	// Only "cache" reaches a gateway on a sibling subdomain; "storage" would
	// wipe the theme preference.
	if got := logout.Header.Get("Clear-Site-Data"); got != `"cache"` {
		t.Fatalf("Clear-Site-Data = %q", got)
	}
	wantLocation(t, a.client.Get("/"), "/login")
}

func rawSessionSetCookie(t *testing.T, res *testutil.Response, name string) string {
	t.Helper()
	for _, value := range res.Header.Values("Set-Cookie") {
		if strings.HasPrefix(value, name+"=") {
			return value
		}
	}
	t.Fatalf("no Set-Cookie for %s in %v", name, res.Header.Values("Set-Cookie"))
	return ""
}

// Secure and the __Host- prefix follow PINGWARD_BASE_URL's scheme; there is
// never Max-Age or Expires, since expiry is server-side.
func TestSessionCookieCarriesSecureOnlyWhenConfigured(t *testing.T) {
	cases := []struct {
		env    map[string]string
		name   string
		secure bool
	}{
		{map[string]string{"PINGWARD_BASE_URL": "https://pingward.example"}, "__Host-pingward_session", true},
		{nil, "pingward_session", false},
	}
	for _, tc := range cases {
		a := newApp(t, tc.env)
		a.addUser(t, "alice", true)
		res := a.attemptLogin(t, "alice", testPassword)
		wantStatus(t, res, http.StatusSeeOther)
		header := rawSessionSetCookie(t, res, tc.name)
		lower := strings.ToLower(header)
		if strings.Contains(header, "; Secure") != tc.secure || (tc.secure && !strings.Contains(header, "; Path=/")) {
			t.Errorf("%s: %s", tc.name, header)
		}
		for _, banned := range []string{"domain=", "max-age", "expires"} {
			if strings.Contains(lower, banned) {
				t.Errorf("%s must not carry %s: %s", tc.name, banned, header)
			}
		}
	}
}

// On a Secure deployment the __Host- name is read back by authenticated routes
// and cleared by logout.
func TestHostPrefixedCookieRoundTrips(t *testing.T) {
	a := newApp(t, map[string]string{"PINGWARD_BASE_URL": "https://pingward.example"})
	a.addUser(t, "alice", true)
	wantStatus(t, a.attemptLogin(t, "alice", testPassword), http.StatusSeeOther)
	wantStatus(t, a.client.Get("/account"), http.StatusOK)

	res := a.postToken("/logout")
	wantStatus(t, res, http.StatusSeeOther)
	// A 303 alone would pass if logout read the unprefixed name.
	if sessionRows(t, a) != 0 {
		t.Fatal("logout must delete the session row")
	}
	removal := rawSessionSetCookie(t, res, "__Host-pingward_session")
	if !strings.Contains(removal, "; Secure") || !strings.Contains(removal, "; Path=/") {
		t.Fatalf("the removal cookie lacks its required attributes: %s", removal)
	}
}

func TestBrowserResponsesAreNotCacheable(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	wantNoStore := func(res *testutil.Response, what string) {
		t.Helper()
		if got := res.Header.Get("Cache-Control"); got != "no-store" {
			t.Errorf("%s Cache-Control = %q", what, got)
		}
	}
	res := a.client.Get("/")
	wantStatus(t, res, http.StatusOK)
	wantNoStore(res, "/")
	f := a.seed(t, a.adminID(t), "web")
	res = a.client.Get("/checks/" + itoa(f.checkID))
	wantStatus(t, res, http.StatusOK)
	wantNoStore(res, "check page")

	anonymous := newApp(t, nil)
	wantNoStore(anonymous.client.Get("/login"), "/login")
	// no-store sits outside the CSRF guard, so its early 403 is covered.
	res = anonymous.client.PostForm("/login", url.Values{"username": {"admin"}, "password": {"pw"}})
	wantStatus(t, res, http.StatusForbidden)
	wantNoStore(res, "CSRF rejection")
}

// Five first visitors (the per-IP limit) submit /setup at once: the
// check-then-insert gap must not admit more than one admin. File-backed so the
// pool really holds several connections (testutil.NewStore is file-backed).
func TestConcurrentSetupCreatesExactlyOneAdmin(t *testing.T) {
	a := newApp(t, nil)
	a.deps.SetupToken.Arm(testutil.TestSetupToken)
	csrf := testutil.AnonymousCSRF(t, a.client, "/setup")
	const visitors = 5
	locations := make([]string, visitors)
	var wg sync.WaitGroup
	for i := range visitors {
		wg.Add(1)
		go func() {
			defer wg.Done()
			res := a.client.PostForm("/setup", url.Values{
				"_csrf": {csrf}, "setup_token": {testutil.TestSetupToken},
				"username": {"admin" + itoa(int64(i))}, "password": {firstAdminPassword},
			})
			locations[i] = res.Header.Get("Location")
		}()
	}
	wg.Wait()
	winners, losers := 0, 0
	for _, location := range locations {
		switch location {
		case "/":
			winners++
		case "/login":
			losers++
		}
	}
	if countUsers(t, a) != 1 || winners != 1 || losers != visitors-1 {
		t.Fatalf("users = %d, winners = %d, losers = %d (%v)", countUsers(t, a), winners, losers, locations)
	}
}

// Once any user exists /setup is closed: a late POST must not add an account.
func TestSetupAfterAUserExistsIsRefused(t *testing.T) {
	a := newApp(t, nil)
	a.deps.SetupToken.Arm(testutil.TestSetupToken)
	a.addUser(t, "existing", false)
	res := a.postSetup(t, ptr(testutil.TestSetupToken))
	wantLocation(t, res, "/login")
	if countUsers(t, a) != 1 {
		t.Fatal("the late POST must not add an account")
	}
}

func TestSetupRefusesAMissingOrWrongTokenAndAuditsIt(t *testing.T) {
	a := tokenApp(t)
	for _, token := range []*string{nil, ptr(""), ptr("ZZZZZ-ZZZZZ")} {
		res := a.postSetup(t, token)
		wantStatus(t, res, http.StatusForbidden)
		if !strings.Contains(res.Body, "setup-error") {
			t.Fatal("the refusal must render the setup error")
		}
	}
	if countUsers(t, a) != 0 || a.deps.SetupToken.Get() == "" {
		t.Fatal("a wrong guess must not create a user or burn the token")
	}
	if n := countRows(t, a, "SELECT COUNT(*) FROM audit_log WHERE action = 'setup.token_rejected'"); n != 3 {
		t.Fatalf("rejected = %d, want 3", n)
	}
}

func TestSetupAcceptsTheTokenInAnyDashCaseAndSpacing(t *testing.T) {
	a := tokenApp(t)
	wantLocation(t, a.postSetup(t, ptr(" abcde 12345 ")), "/")
	if countUsers(t, a) != 1 || a.deps.SetupToken.Get() != "" {
		t.Fatal("one admin must exist and the token must die with setup")
	}
}

func TestSetupWithoutAnArmedTokenIsRefused(t *testing.T) {
	a := newApp(t, nil)
	wantStatus(t, a.postSetup(t, ptr("")), http.StatusForbidden)
	if countUsers(t, a) != 0 {
		t.Fatal("no user may be created")
	}
}

func TestSetupAttemptsAreRateLimitedBeforeTheTokenIsChecked(t *testing.T) {
	a := tokenApp(t)
	for range ratelimit.SetupMaxAttempts {
		wantStatus(t, a.postSetup(t, ptr("WRONG-WRONG")), http.StatusForbidden)
	}
	wantThrottled(t, a.postSetup(t, ptr("ABCDE-12345")), ratelimit.SetupWindow)
	if countUsers(t, a) != 0 {
		t.Fatal("even the right token is throttled once the budget is spent")
	}
}

func TestASetupTokenIsIssuedOnlyWhileNoUserExists(t *testing.T) {
	a := newApp(t, nil)
	issued, err := a.deps.ArmSetupTokenIfUnclaimed(t.Context())
	if err != nil || issued == "" || issued != a.deps.SetupToken.Get() {
		t.Fatalf("issued %q, armed %q (%v)", issued, a.deps.SetupToken.Get(), err)
	}
	a.deps.SetupToken.Clear()
	a.addUser(t, "existing", true)
	issued, err = a.deps.ArmSetupTokenIfUnclaimed(t.Context())
	if err != nil || issued != "" || a.deps.SetupToken.Get() != "" {
		t.Fatalf("no token may be issued once a user exists: %q (%v)", issued, err)
	}
}

func TestSetupStoresTheTrimmedUsernameAndRefusesBadOnes(t *testing.T) {
	a := tokenApp(t)
	csrf := testutil.AnonymousCSRF(t, a.client, "/setup")
	post := func(username string) *testutil.Response {
		return a.client.PostForm("/setup", url.Values{
			"_csrf": {csrf}, "username": {username}, "password": {firstAdminPassword}, "setup_token": {"ABCDE-12345"},
		})
	}
	for _, bad := range []string{strings.Repeat("a", 65), "bad\x07name"} {
		res := post(bad)
		wantStatus(t, res, http.StatusOK)
		if !strings.Contains(res.Body, "setup-error") {
			t.Fatalf("%q must be refused with the setup error", bad)
		}
	}
	if countUsers(t, a) != 0 {
		t.Fatal("a bad username must not create a user")
	}
	wantStatus(t, post("  admin  "), http.StatusSeeOther)
	a.userByName(t, "admin")
}

// adminProjectApp is a signed-in admin owning one project ("web"), like the
// Rust logged_in_server plus a project.
func adminProjectApp(t *testing.T, env map[string]string) (*app, int64) {
	t.Helper()
	a := newApp(t, env)
	a.loggedInAdmin(t)
	projectID, err := a.store.CreateProject(t.Context(), a.adminID(t), store.ProjectSettings{Name: "web"}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return a, projectID
}

func (a *app) channelsOf(t *testing.T, projectID int64) []models.Channel {
	t.Helper()
	channels, err := a.store.ListChannelsForProject(t.Context(), projectID)
	if err != nil {
		t.Fatal(err)
	}
	return channels
}

func (a *app) postChannel(projectID int64, form url.Values) *testutil.Response {
	return a.client.PostForm("/projects/"+itoa(projectID)+"/channels", withToken(form, testutil.NewestSessionCSRF(a.client.T, a.store)))
}

func TestChannelFormHidesEmailWithoutSMTP(t *testing.T) {
	a, projectID := adminProjectApp(t, nil)
	res := a.client.Get("/projects/" + itoa(projectID) + "/channels/new")
	wantStatus(t, res, http.StatusOK)
	if strings.Contains(res.Body, `value="email"`) {
		t.Fatal("the email option must be hidden when SMTP is unconfigured")
	}
}

func TestChannelFormShowsEmailWithSMTP(t *testing.T) {
	a, projectID := adminProjectApp(t, smtpEnv)
	res := a.client.Get("/projects/" + itoa(projectID) + "/channels/new")
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, `value="email"`) {
		t.Fatal("the email option must appear when SMTP is configured")
	}
}

var smtpEnv = map[string]string{"PINGWARD_SMTP_HOST": "mail.example.com", "PINGWARD_SMTP_FROM": "alerts@example.com"}

func TestCreateEmailChannelStoresRecipient(t *testing.T) {
	a, projectID := adminProjectApp(t, smtpEnv)
	res := a.postChannel(projectID, url.Values{"name": {"ops"}, "kind": {"email"}, "email_to": {"ops@example.com"}})
	wantStatus(t, res, http.StatusSeeOther)
	channels := a.channelsOf(t, projectID)
	if len(channels) != 1 || !strings.Contains(channels[0].ConfigJSON, "ops@example.com") {
		t.Fatalf("channels = %+v", channels)
	}
}

func TestRegenerateUUIDChangesPingURL(t *testing.T) {
	a, projectID := adminProjectApp(t, nil)
	checkID := a.addCheck(t, projectID, "job", "old-uuid")
	wantStatus(t, a.postToken("/checks/"+itoa(checkID)+"/regenerate?confirmed=1"), http.StatusSeeOther)
	check, err := a.store.FindCheck(t.Context(), checkID)
	if err != nil || check.PingUUID == "old-uuid" {
		t.Fatalf("the ping uuid must change: %+v (%v)", check, err)
	}
}

func TestAdminSetsGlobalScanInterval(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	wantStatus(t, a.client.Get("/admin"), http.StatusOK)
	form := url.Values{"scan_interval": {"45"}, "nag_interval": {""}, "pings_retention_days": {""}, "notifications_retention_days": {""}, "audit_retention_days": {""}}
	wantStatus(t, a.client.PostForm("/admin/settings", withToken(form, testutil.NewestSessionCSRF(t, a.store))), http.StatusSeeOther)
	a.wantSetting(t, "scan_interval", "45")
}

func (a *app) wantSetting(t *testing.T, key, want string) {
	t.Helper()
	got, err := a.store.GetSetting(t.Context(), key)
	if err != nil || got == nil || *got != want {
		t.Fatalf("setting %s = %v (%v), want %q", key, got, err, want)
	}
}

func TestAdminSetsRetentionDays(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	wantStatus(t, a.client.Get("/admin"), http.StatusOK)
	form := url.Values{"scan_interval": {""}, "nag_interval": {""}, "pings_retention_days": {"30"}, "notifications_retention_days": {"90"}, "audit_retention_days": {"365"}}
	wantStatus(t, a.client.PostForm("/admin/settings", withToken(form, testutil.NewestSessionCSRF(t, a.store))), http.StatusSeeOther)
	a.wantSetting(t, "pings_retention_days", "30")
	a.wantSetting(t, "audit_retention_days", "365")
	a.wantSetting(t, "notifications_retention_days", "90")
}

func TestAdminCreatesAndDeletesUser(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	// Creating (not deleting) an account is behind the elevation gate.
	wantStatus(t, a.unlock(t, testPassword, nil), http.StatusSeeOther)
	form := url.Values{"username": {"carol"}, "password": {"carol's passphrase"}, "is_admin": {""}}
	wantStatus(t, a.client.PostForm("/admin/users", withToken(form, testutil.NewestSessionCSRF(t, a.store))), http.StatusSeeOther)
	carol := a.userByName(t, "carol")
	if carol.IsAdmin {
		t.Fatal("carol must not be an admin")
	}
	wantStatus(t, a.postToken("/admin/users/"+itoa(carol.ID)+"/delete?confirmed=1"), http.StatusSeeOther)
	if user, _ := a.store.FindUserByUsername(t.Context(), "carol"); user != nil {
		t.Fatal("carol must be deleted")
	}
}

func TestCreateChannelAndBindToCheck(t *testing.T) {
	a, projectID := adminProjectApp(t, nil)
	checkID := a.addCheck(t, projectID, "job", "cu")
	wantStatus(t, a.postChannel(projectID, url.Values{"name": {"hook"}, "kind": {"webhook"}, "webhook_url": {"http://example.test/h"}}), http.StatusSeeOther)
	channels := a.channelsOf(t, projectID)
	if len(channels) != 1 || !strings.Contains(channels[0].ConfigJSON, "example.test") {
		t.Fatalf("channels = %+v", channels)
	}
	bindPath := "/checks/" + itoa(checkID) + "/channels"
	token := testutil.NewestSessionCSRF(t, a.store)
	wantStatus(t, a.client.PostForm(bindPath, url.Values{"_csrf": {token}, "channel_ids": {itoa(channels[0].ID)}}), http.StatusSeeOther)
	if bound, _ := a.store.BoundChannelIDs(t.Context(), checkID); !slices.Equal(bound, []int64{channels[0].ID}) {
		t.Fatalf("bound = %v", bound)
	}
	// No channel_ids unbinds all.
	wantStatus(t, a.client.PostForm(bindPath, url.Values{"_csrf": {token}, "_": {""}}), http.StatusSeeOther)
	if bound, _ := a.store.BoundChannelIDs(t.Context(), checkID); len(bound) != 0 {
		t.Fatalf("bound = %v, want none", bound)
	}
}

func TestChannelCreationPersistsEachKindsConfig(t *testing.T) {
	cases := []struct {
		name string
		form url.Values
		kind models.ChannelKind
		want []string
	}{
		{"telegram", url.Values{"name": {"tg"}, "kind": {"telegram"}, "telegram_token": {"123:ABC"}, "telegram_chat_id": {"999"}},
			models.ChannelTelegram, []string{`"token":"123:ABC"`, `"chat_id":"999"`}},
		{"slack", url.Values{"name": {"sl"}, "kind": {"slack"}, "slack_url": {"http://hooks.test/x"}},
			models.ChannelSlack, []string{"hooks.test"}},
		{"ntfy", url.Values{"name": {"nt"}, "kind": {"ntfy"}, "ntfy_base_url": {"https://ntfy.example"}, "ntfy_topic": {"alerts"}, "ntfy_token": {""}},
			models.ChannelNtfy, []string{`"topic":"alerts"`, "ntfy.example"}},
		{"pushover", url.Values{"name": {"po"}, "kind": {"pushover"}, "pushover_token": {"apptok"}, "pushover_user": {"userkey"}},
			models.ChannelPushover, []string{`"token":"apptok"`, `"user":"userkey"`}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			a, projectID := adminProjectApp(t, nil)
			wantStatus(t, a.postChannel(projectID, tc.form), http.StatusSeeOther)
			channels := a.channelsOf(t, projectID)
			if len(channels) != 1 || channels[0].Kind != tc.kind {
				t.Fatalf("channels = %+v", channels)
			}
			for _, want := range tc.want {
				if !strings.Contains(channels[0].ConfigJSON, want) {
					t.Errorf("config %s lacks %s", channels[0].ConfigJSON, want)
				}
			}
		})
	}
}

func TestChannelCreateRejectsBlankRequiredField(t *testing.T) {
	a, projectID := adminProjectApp(t, nil)
	res := a.postChannel(projectID, url.Values{"name": {"tg"}, "kind": {"telegram"}, "telegram_token": {"123:ABC"}, "telegram_chat_id": {""}})
	wantStatus(t, res, http.StatusOK)
	if len(a.channelsOf(t, projectID)) != 0 {
		t.Fatal("no channel may be stored")
	}
}

// otherUsersProject is a second user's project, check and channel.
func otherUsersProject(t *testing.T, a *app) fixture {
	t.Helper()
	return a.seed(t, a.addUser(t, "other", false), "secret")
}

func TestCannotOperateOnAnotherUsersCheck(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	f := otherUsersProject(t, a)
	path := "/checks/" + itoa(f.checkID)
	wantStatus(t, a.client.Get(path), http.StatusNotFound)
	wantStatus(t, a.postToken(path+"/pause"), http.StatusNotFound)
	wantStatus(t, a.postToken(path+"/delete?confirmed=1"), http.StatusNotFound)
	if check, _ := a.store.FindCheck(t.Context(), f.checkID); check == nil {
		t.Fatal("the check must survive")
	}
}

func TestNonOwnerCannotAcknowledge(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	f := otherUsersProject(t, a)
	if err := a.store.SetStatus(t.Context(), f.checkID, models.StatusDown); err != nil {
		t.Fatal(err)
	}
	wantStatus(t, a.postToken("/checks/"+itoa(f.checkID)+"/ack"), http.StatusNotFound)
	if check, _ := a.store.FindCheck(t.Context(), f.checkID); check.Acknowledged {
		t.Fatal("the check must not be acknowledged")
	}
}

func TestCannotDeleteAnotherUsersChannel(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	f := otherUsersProject(t, a)
	wantStatus(t, a.postToken("/channels/"+itoa(f.channelID)+"/delete"), http.StatusNotFound)
	if channel, _ := a.store.FindChannel(t.Context(), f.channelID); channel == nil {
		t.Fatal("the channel must survive")
	}
}

func TestCannotCreateChannelInAnotherUsersProject(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	f := otherUsersProject(t, a)
	res := a.postChannel(f.projectID, url.Values{"name": {"x"}, "kind": {"webhook"}, "webhook_url": {"http://evil.test/h"}})
	wantStatus(t, res, http.StatusNotFound)
	channels := a.channelsOf(t, f.projectID)
	if len(channels) != 1 || !strings.Contains(channels[0].ConfigJSON, "example.invalid") {
		t.Fatalf("the other project's channels must be untouched: %+v", channels)
	}
}

func TestAdminCannotDeleteSelf(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	adminID := a.adminID(t)
	wantStatus(t, a.postToken("/admin/users/"+itoa(adminID)+"/delete?confirmed=1"), http.StatusSeeOther)
	if user, _ := a.store.FindUserByID(t.Context(), adminID); user == nil {
		t.Fatal("the sole admin must survive")
	}
}

func sendTestNotification(t *testing.T, status int) *testutil.Response {
	t.Helper()
	hook := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) { w.WriteHeader(status) }))
	t.Cleanup(hook.Close)
	a, projectID := adminProjectApp(t, nil)
	channelID, err := a.store.CreateChannel(t.Context(), store.NewChannel{
		ProjectID: projectID, Kind: models.ChannelWebhook, Name: "hook", ConfigJSON: `{"url":"` + hook.URL + `"}`,
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	res := a.postToken("/channels/" + itoa(channelID) + "/test")
	wantStatus(t, res, http.StatusOK)
	return res
}

func TestSendTestNotificationReportsSuccess(t *testing.T) {
	res := sendTestNotification(t, http.StatusOK)
	if !strings.Contains(res.Body, "Test notification sent") || !strings.Contains(res.Body, `class="flash ok"`) {
		t.Fatalf("missing the restyled ok banner: %s", res.Body)
	}
}

func TestSendTestNotificationReportsFailure(t *testing.T) {
	res := sendTestNotification(t, http.StatusInternalServerError)
	if !strings.Contains(res.Body, "Test notification failed") || !strings.Contains(res.Body, `class="flash err"`) {
		t.Fatalf("missing the restyled err banner: %s", res.Body)
	}
}

func TestAdminPageUsesRestyledFieldClass(t *testing.T) {
	a := newApp(t, nil)
	a.loggedInAdmin(t)
	if !strings.Contains(a.client.Get("/admin").Body, `class="field"`) {
		t.Fatal("the admin page must use the restyled field class")
	}
}

func TestCheckPageShowsNotificationChannelAndError(t *testing.T) {
	a, projectID := adminProjectApp(t, nil)
	checkID := a.addCheck(t, projectID, "job", "cu")
	channelID, err := a.store.CreateChannel(t.Context(), store.NewChannel{
		ProjectID: projectID, Kind: models.ChannelWebhook, Name: "my-hook", ConfigJSON: `{"url":"http://x"}`,
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	failure := "status 500"
	err = a.store.RecordNotification(t.Context(), store.NewNotification{
		CheckID: checkID, ChannelID: channelID, Event: models.EventDown, Status: models.NotifyError, Error: &failure,
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	res := a.client.Get("/checks/" + itoa(checkID))
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "my-hook") || !strings.Contains(res.Body, "status 500") {
		t.Fatalf("the channel name and error text must show: %s", res.Body)
	}
}
