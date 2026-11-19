package server_test

import (
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/apikey"
	"github.com/henry40408/pingward/internal/auth"
	"github.com/henry40408/pingward/internal/ratelimit"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

// memberApp is an app with a signed-in non-admin "member".
func memberApp(t *testing.T) (*app, int64) {
	t.Helper()
	a := newApp(t, nil)
	id := a.addUser(t, "member", false)
	a.login(t, "member")
	return a, id
}

// accountToken is the CSRF token of the account page.
func accountToken(t *testing.T, a *app) string {
	t.Helper()
	res := a.client.Get("/account")
	wantStatus(t, res, http.StatusOK)
	return extractCSRF(t, res.Body)
}

func (a *app) post(t *testing.T, path string, form url.Values) *testutil.Response {
	t.Helper()
	return a.client.PostForm(path, withToken(form, accountToken(t, a)))
}

func passwordForm(current, next, confirm string) url.Values {
	return url.Values{"current_password": {current}, "new_password": {next}, "confirm_password": {confirm}}
}

func keyForm(password, name, expires string) url.Values {
	return url.Values{"current_password": {password}, "name": {name}, "expires_in": {expires}}
}

func countRows(t *testing.T, a *app, query string, args ...any) int {
	t.Helper()
	var n int
	if err := a.store.DB.QueryRowContext(t.Context(), query, args...).Scan(&n); err != nil {
		t.Fatal(err)
	}
	return n
}

func storedHash(t *testing.T, a *app, id int64) string {
	t.Helper()
	user, err := a.store.FindUserByID(t.Context(), id)
	if err != nil || user == nil || user.PasswordHash == nil {
		t.Fatalf("user %d: %+v %v", id, user, err)
	}
	return *user.PasswordHash
}

func TestAccountPageListsAndMarksTheCurrentSession(t *testing.T) {
	a, _ := memberApp(t)
	res := a.client.Get("/account")
	wantStatus(t, res, http.StatusOK)
	// The session id is the cookie's bearer secret; rows use its SHA-256 handle.
	sessionID := a.sessionID(t)
	if strings.Contains(res.Body, sessionID) {
		t.Error("the raw session id (the cookie secret) must never be rendered")
	}
	if !strings.Contains(res.Body, "session-row-"+apikey.Hash(sessionID)) {
		t.Error("the session row must carry the handle")
	}
	if strings.Contains(res.Body, "session-revoke-others") {
		t.Error("a lone session has nothing to revoke")
	}
	if !strings.Contains(res.Body, `data-testid="session-current"`) {
		t.Error("the current session must be marked")
	}
	if strings.Contains(res.Body, `data-testid="session-sso"`) {
		t.Error("a password login has no SSO pill")
	}
	if !strings.Contains(a.client.Get("/projects/new").Body, `href="/account"`) {
		t.Error("the nav must link to the account page")
	}
}

func TestRevokeOthersLeavesOnlyTheCurrentSession(t *testing.T) {
	a, _ := memberApp(t)
	other := testutil.NewClient(t, newHandler(a.deps))
	csrf := testutil.AnonymousCSRF(t, other, "/login")
	wantStatus(t, other.PostForm("/login", url.Values{"_csrf": {csrf}, "username": {"member"}, "password": {testPassword}}), http.StatusSeeOther)
	if rows := sessionRows(t, a); rows != 2 {
		t.Fatalf("sessions = %d, want 2", rows)
	}
	wantStatus(t, a.post(t, "/account/sessions/revoke-others?confirmed=1", url.Values{}), http.StatusSeeOther)
	if rows := sessionRows(t, a); rows != 1 {
		t.Fatalf("sessions = %d, want 1", rows)
	}
	wantStatus(t, a.client.Get("/account"), http.StatusOK)
	wantBouncedToLogin(t, other.Get("/account"))
}

func TestRevokingTheCurrentSessionLogsOut(t *testing.T) {
	a, _ := memberApp(t)
	handle := apikey.Hash(a.sessionID(t))
	res := a.post(t, "/account/sessions/"+handle+"/revoke?confirmed=1", url.Values{})
	wantStatus(t, res, http.StatusSeeOther)
	if got := res.Header.Get("Location"); got != "/login" {
		t.Fatalf("Location = %q, want /login", got)
	}
	if rows := sessionRows(t, a); rows != 0 {
		t.Fatalf("sessions = %d, want 0", rows)
	}
}

// An unknown or foreign handle must neither reveal that it exists nor revoke it.
func TestUnknownOrForeignHandleRevokesNothing(t *testing.T) {
	a, _ := memberApp(t)
	a.addUser(t, "victim", false)
	other := testutil.NewClient(t, newHandler(a.deps))
	csrf := testutil.AnonymousCSRF(t, other, "/login")
	wantStatus(t, other.PostForm("/login", url.Values{"_csrf": {csrf}, "username": {"victim"}, "password": {testPassword}}), http.StatusSeeOther)
	var victimID string
	if err := a.store.DB.QueryRowContext(t.Context(), "SELECT id FROM sessions WHERE user_id = (SELECT id FROM users WHERE username = 'victim')").Scan(&victimID); err != nil {
		t.Fatal(err)
	}
	for _, handle := range []string{apikey.Hash(victimID), "no-such-handle"} {
		res := a.post(t, "/account/sessions/"+handle+"/revoke?confirmed=1", url.Values{})
		wantStatus(t, res, http.StatusSeeOther)
		if got := res.Header.Get("Location"); got != "/account" {
			t.Fatalf("Location = %q, want /account", got)
		}
	}
	if rows := sessionRows(t, a); rows != 2 {
		t.Fatalf("sessions = %d, want 2: nothing may be revoked", rows)
	}
}

func TestChangingThePasswordRotatesItAndSignsOutOtherSessions(t *testing.T) {
	a, id := memberApp(t)
	other := testutil.NewClient(t, newHandler(a.deps))
	csrf := testutil.AnonymousCSRF(t, other, "/login")
	other.PostForm("/login", url.Values{"_csrf": {csrf}, "username": {"member"}, "password": {testPassword}})
	before := storedHash(t, a, id)

	res := a.post(t, "/account/password", passwordForm(testPassword, "a-brand-new-passphrase", "a-brand-new-passphrase"))
	wantStatus(t, res, http.StatusSeeOther)
	if storedHash(t, a, id) == before {
		t.Fatal("the hash must change")
	}
	if rows := sessionRows(t, a); rows != 1 {
		t.Fatalf("sessions = %d, want only the caller's", rows)
	}
	wantBouncedToLogin(t, other.Get("/account"))
	wantStatus(t, a.client.Get("/account"), http.StatusOK)
	var detail string
	err := a.store.DB.QueryRowContext(t.Context(), "SELECT detail FROM audit_log WHERE action = 'user.password_change'").Scan(&detail)
	if err != nil || detail != "sessions_revoked=1" {
		t.Fatalf("audit detail = %q (%v)", detail, err)
	}
	if got := a.logs.EventsNamed(t, "session.destroyed"); len(got) != 1 || got[0]["reason"] != "password_change" {
		t.Fatalf("session.destroyed events = %v", got)
	}
}

func TestRejectedPasswordChangesTouchNeitherThePasswordNorTheSessions(t *testing.T) {
	a, id := memberApp(t)
	a.secondSession(t, "member")
	before := storedHash(t, a, id)
	forms := map[string]url.Values{
		"wrong current": passwordForm("nope", "a-brand-new-passphrase", "a-brand-new-passphrase"),
		"mismatch":      passwordForm(testPassword, "a-brand-new-passphrase", "something-else-entirely"),
		"blank":         passwordForm(testPassword, "", ""),
		"too short":     passwordForm(testPassword, "short pass", "short pass"),
	}
	for name, form := range forms {
		res := a.post(t, "/account/password", form)
		wantStatus(t, res, http.StatusOK)
		if !strings.Contains(res.Body, "password-error") {
			t.Errorf("%s: the error must be shown", name)
		}
		if storedHash(t, a, id) != before {
			t.Fatalf("%s: the password must not change", name)
		}
		if rows := countRows(t, a, "SELECT COUNT(*) FROM sessions WHERE user_id = ?", id); rows != 2 {
			t.Fatalf("%s: sessions = %d, want 2", name, rows)
		}
	}
	if !strings.Contains(a.post(t, "/account/password", passwordForm("nope", "x", "y")).Body, "Current password is incorrect.") {
		t.Error("a wrong current password is reported")
	}
}

// Policy is checked before the match check, so the more useful error wins.
func TestPasswordPolicyIsCheckedBeforeTheMatch(t *testing.T) {
	a, _ := memberApp(t)
	res := a.post(t, "/account/password", passwordForm(testPassword, "short", "different"))
	if refusal := auth.ValidatePassword("short"); !strings.Contains(res.Body, refusal) {
		t.Fatalf("want the policy message %q", refusal)
	}
}

func TestOverLongPasswordIsRefusedNotTruncated(t *testing.T) {
	a, id := memberApp(t)
	before := storedHash(t, a, id)
	long := strings.Repeat("a", 2000)
	wantStatus(t, a.post(t, "/account/password", passwordForm(testPassword, long, long)), http.StatusOK)
	if storedHash(t, a, id) != before {
		t.Fatal("an over-long password must be refused")
	}
}

func TestPasswordlessAccountHasNoPasswordFormAndCannotSetOne(t *testing.T) {
	a := newApp(t, forwardAuthEnv)
	cookie := signedInViaGateway(t, a)
	header := http.Header{"X-Forwarded-User": {"alice"}, "Cookie": {cookie.Name + "=" + cookie.Value}}
	page := a.client.Do("GET", "/account", nil, header)
	token := extractCSRF(t, page.Body)
	if strings.Contains(page.Body, "password-submit") {
		t.Error("a passwordless account gets no password form")
	}
	if strings.Contains(page.Body, "api-key-password-input") {
		t.Error("no stored password means no re-authentication field")
	}
	res := a.client.PostRaw("/account/password", withToken(passwordForm("", "a-brand-new-passphrase", "a-brand-new-passphrase"), token).Encode(), header)
	wantStatus(t, res, http.StatusForbidden)
	user, _ := a.store.FindUserByUsername(t.Context(), "alice")
	if user.PasswordHash != nil {
		t.Fatal("a passwordless account must stay passwordless")
	}
	// Minting a key has nothing to re-authenticate against.
	res = a.client.PostRaw("/account/api-keys", withToken(url.Values{"name": {"ci"}, "expires_in": {""}}, token).Encode(), header)
	wantStatus(t, res, http.StatusOK)
	if n := countRows(t, a, "SELECT COUNT(*) FROM api_keys"); n != 1 {
		t.Fatalf("keys = %d, want 1", n)
	}
}

func TestCreatingAKeyShowsTheTokenOnce(t *testing.T) {
	a, id := memberApp(t)
	res := a.post(t, "/account/api-keys", keyForm(testPassword, "ci", ""))
	wantStatus(t, res, http.StatusOK)
	keys, _ := a.store.ListAPIKeysForUser(t.Context(), id)
	if len(keys) != 1 || keys[0].ExpiresAt != nil {
		t.Fatalf("keys = %+v", keys)
	}
	token := extractDataCopy(t, res.Body)
	if !strings.HasPrefix(token, "pw_") || len(token) != 67 { // "pw_" + 64 hex
		t.Fatalf("token = %q", token)
	}
	if userID, found, err := a.store.ValidateAPIKey(t.Context(), apikey.Hash(token), time.Now()); err != nil || !found || userID != id {
		t.Fatalf("the shown token must authenticate its owner (%d %v %v)", userID, found, err)
	}
	again := a.client.Get("/account").Body
	if strings.Contains(again, token) {
		t.Error("the plaintext token must not reappear")
	}
	if !strings.Contains(again, keys[0].Prefix) {
		t.Error("the prefix identifies the key afterwards")
	}
}

func TestKeyExpiryAndRejections(t *testing.T) {
	a, id := memberApp(t)
	before := time.Now()
	wantStatus(t, a.post(t, "/account/api-keys", keyForm(testPassword, "ci", "30d")), http.StatusOK)
	keys, _ := a.store.ListAPIKeysForUser(t.Context(), id)
	if len(keys) != 1 || keys[0].ExpiresAt == nil || keys[0].ExpiresAt.Before(before.Add(29*24*time.Hour)) {
		t.Fatalf("keys = %+v", keys)
	}
	for _, form := range []url.Values{keyForm(testPassword, "ci", "soon"), keyForm(testPassword, "   ", "")} {
		wantStatus(t, a.post(t, "/account/api-keys", form), http.StatusOK)
	}
	if n := countRows(t, a, "SELECT COUNT(*) FROM api_keys"); n != 1 {
		t.Fatalf("keys = %d, want 1: rejected forms mint nothing", n)
	}
}

func TestKeysAreCallerScopedAndRevocable(t *testing.T) {
	a, id := memberApp(t)
	a.post(t, "/account/api-keys", keyForm(testPassword, "mine", ""))
	other := a.addUser(t, "other", false)
	if _, err := a.store.DB.ExecContext(t.Context(),
		"INSERT INTO api_keys (user_id, name, token_hash, prefix, created_at) VALUES (?, 'theirs', 'h', 'p', ?)", other, time.Now().UTC().Format(time.RFC3339)); err != nil {
		t.Fatal(err)
	}
	var theirs int64
	if err := a.store.DB.QueryRowContext(t.Context(), "SELECT id FROM api_keys WHERE name = 'theirs'").Scan(&theirs); err != nil {
		t.Fatal(err)
	}
	if page := a.client.Get("/account").Body; strings.Contains(page, "theirs") {
		t.Error("another user's key must not be listed")
	}
	wantStatus(t, a.post(t, "/account/api-keys/"+itoa(theirs)+"/delete?confirmed=1", url.Values{}), http.StatusSeeOther)
	if n := countRows(t, a, "SELECT COUNT(*) FROM api_keys WHERE name = 'theirs'"); n != 1 {
		t.Fatal("a key the caller does not own must survive")
	}
	mine, _ := a.store.ListAPIKeysForUser(t.Context(), id)
	wantStatus(t, a.post(t, "/account/api-keys/"+itoa(mine[0].ID)+"/delete?confirmed=1", url.Values{}), http.StatusSeeOther)
	if n := countRows(t, a, "SELECT COUNT(*) FROM api_keys WHERE user_id = ?", id); n != 0 {
		t.Fatal("the own key must be revoked")
	}
}

func TestRevokingAKeyAsksForConfirmationFirst(t *testing.T) {
	a, id := memberApp(t)
	a.post(t, "/account/api-keys", keyForm(testPassword, "mine", ""))
	mine, _ := a.store.ListAPIKeysForUser(t.Context(), id)
	res := a.post(t, "/account/api-keys/"+itoa(mine[0].ID)+"/delete", url.Values{})
	wantStatus(t, res, http.StatusOK)
	if n := countRows(t, a, "SELECT COUNT(*) FROM api_keys"); n != 1 {
		t.Fatal("the confirm page must not delete")
	}
}

func TestAccountPostWithoutCSRFIsForbidden(t *testing.T) {
	a, _ := memberApp(t)
	wantStatus(t, a.client.PostForm("/account/api-keys", keyForm(testPassword, "ci", "")), http.StatusForbidden)
}

func TestKeyCreationWithoutOrWithWrongPasswordIsRefused(t *testing.T) {
	a, _ := memberApp(t)
	for _, password := range []string{"", "not-my-password"} {
		res := a.post(t, "/account/api-keys", keyForm(password, "ci", ""))
		wantStatus(t, res, http.StatusOK)
		if !strings.Contains(res.Body, "Current password is incorrect.") || !strings.Contains(res.Body, "api-key-error") {
			t.Errorf("password %q: want the refusal", password)
		}
	}
	if n := countRows(t, a, "SELECT COUNT(*) FROM api_keys"); n != 0 {
		t.Fatal("no key without the password")
	}
	if got := a.logs.EventsNamed(t, "reauth.failed"); len(got) != 2 || got[0]["surface"] != "api_key_create" || got[0]["reason"] != "bad_current_password" {
		t.Fatalf("reauth.failed events = %v", got)
	}
}

func TestPasswordIsCheckedBeforeTheRestOfTheKeyForm(t *testing.T) {
	a, _ := memberApp(t)
	res := a.post(t, "/account/api-keys", keyForm("wrong", "", "garbage"))
	if !strings.Contains(res.Body, "Current password is incorrect.") || strings.Contains(res.Body, "A name is required.") || strings.Contains(res.Body, "Expiry must be a duration") {
		t.Fatal("the password gate must come first")
	}
}

func TestRepeatedWrongPasswordsExhaustTheAccountBudget(t *testing.T) {
	a, _ := memberApp(t)
	for range ratelimit.AccountMaxAttempts {
		wantStatus(t, a.post(t, "/account/api-keys", keyForm("not-my-password", "ci", "")), http.StatusOK)
	}
	res := a.post(t, "/account/api-keys", keyForm(testPassword, "ci", ""))
	if !strings.Contains(res.Body, "Too many attempts") {
		t.Fatal("even the correct password is refused once the budget is gone")
	}
	if n := countRows(t, a, "SELECT COUNT(*) FROM api_keys"); n != 0 {
		t.Fatal("no key while throttled")
	}
	events := a.logs.EventsNamed(t, "reauth.failed")
	if last := events[len(events)-1]; last["reason"] != "rate_limited" {
		t.Fatalf("last event = %v", last)
	}
}

// A success clears the bucket rather than refunding one attempt.
func TestCorrectPasswordClearsTheAccountBudget(t *testing.T) {
	a, _ := memberApp(t)
	for range ratelimit.AccountMaxAttempts - 1 {
		a.post(t, "/account/api-keys", keyForm("not-my-password", "ci", ""))
	}
	a.post(t, "/account/api-keys", keyForm(testPassword, "one", ""))
	for range ratelimit.AccountMaxAttempts {
		res := a.post(t, "/account/api-keys", keyForm("not-my-password", "ci2", ""))
		if strings.Contains(res.Body, "Too many attempts") {
			t.Fatal("the success must have reset the budget")
		}
	}
}

func TestOpeningTheAccountPageReapsOnlyTheCallersCappedSessions(t *testing.T) {
	a, id := memberApp(t)
	other := a.addUser(t, "other", false)
	old := time.Now().Add(-auth.SessionAbsoluteMax - time.Hour)
	for _, owner := range []int64{id, other} {
		stale := store.NewSession{ID: "stale-" + itoa(owner), UserID: owner, ExpiresAt: time.Now().Add(time.Hour)}
		if err := a.store.CreateSession(t.Context(), stale, old); err != nil {
			t.Fatal(err)
		}
	}
	res := a.client.Get("/account")
	wantStatus(t, res, http.StatusOK)
	if strings.Contains(res.Body, "session-row-"+apikey.Hash("stale-"+itoa(id))) {
		t.Error("a capped session must not be listed")
	}
	if n := countRows(t, a, "SELECT COUNT(*) FROM sessions WHERE id = ?", "stale-"+itoa(id)); n != 0 {
		t.Error("the caller's capped session must be reaped, not merely hidden")
	}
	if n := countRows(t, a, "SELECT COUNT(*) FROM sessions WHERE user_id = ?", id); n != 1 {
		t.Errorf("the live session must stay, sessions = %d", n)
	}
	if n := countRows(t, a, "SELECT COUNT(*) FROM sessions WHERE id = ?", "stale-"+itoa(other)); n != 1 {
		t.Error("another user's session must be left alone")
	}
}

func extractDataCopy(t *testing.T, html string) string {
	t.Helper()
	const marker = `data-copy="`
	start := strings.Index(html, marker)
	if start < 0 {
		t.Fatal("token banner missing")
	}
	rest := html[start+len(marker):]
	return rest[:strings.Index(rest, `"`)]
}

func itoa(n int64) string { return strconv.FormatInt(n, 10) }
