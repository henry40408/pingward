package server_test

import (
	"math"
	"net/http"
	"net/netip"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/ratelimit"
	"github.com/henry40408/pingward/internal/testutil"

	"net/url"
)

const wrongPassword = "zzz-never-log-this-zzz"

func (a *app) attemptLogin(t *testing.T, username, password string) *testutil.Response {
	t.Helper()
	csrf := testutil.AnonymousCSRF(t, a.client, "/login")
	return a.client.PostForm("/login", url.Values{"_csrf": {csrf}, "username": {username}, "password": {password}})
}

// withoutAddressLimit lets the per-account budget run out before the
// per-address one does.
func (a *app) withoutAddressLimit() {
	a.deps.LoginLimiter = ratelimit.New[netip.Addr](math.MaxUint32, time.Minute, time.Now())
}

func wantThrottled(t *testing.T, res *testutil.Response, window time.Duration) {
	t.Helper()
	wantStatus(t, res, http.StatusTooManyRequests)
	if got := res.Header.Get("Retry-After"); got != strconv.Itoa(int(window.Seconds())) {
		t.Fatalf("Retry-After = %q, want %d", got, int(window.Seconds()))
	}
}

func TestLoginIsRateLimitedAfterTheAddressBudget(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "bob", false)
	for range ratelimit.LoginMaxAttempts {
		wantStatus(t, a.attemptLogin(t, "bob", wrongPassword), http.StatusOK)
	}
	wantThrottled(t, a.attemptLogin(t, "bob", wrongPassword), ratelimit.LoginWindow)
}

func TestSuccessfulLoginsDoNotConsumeTheBudget(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "bob", false)
	for range ratelimit.LoginMaxAttempts + 1 {
		wantStatus(t, a.attemptLogin(t, "bob", testPassword), http.StatusSeeOther)
	}
}

func TestThrottledLoginNeverReachesThePasswordCheck(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "bob", false)
	for range ratelimit.LoginMaxAttempts {
		a.attemptLogin(t, "bob", wrongPassword)
	}
	wantThrottled(t, a.attemptLogin(t, "bob", testPassword), ratelimit.LoginWindow)
	if rows := sessionRows(t, a); rows != 0 {
		t.Fatalf("sessions = %d: a throttled request must not sign in", rows)
	}
}

func TestAnAccountLocksAfterItsOwnBudgetWhateverTheSource(t *testing.T) {
	a := newApp(t, nil)
	a.withoutAddressLimit()
	a.addUser(t, "bob", false)
	for range ratelimit.AccountMaxAttempts {
		wantStatus(t, a.attemptLogin(t, "bob", wrongPassword), http.StatusOK)
	}
	wantThrottled(t, a.attemptLogin(t, "bob", wrongPassword), ratelimit.AccountWindow)
	// Even the correct password is refused, and no session appears.
	wantThrottled(t, a.attemptLogin(t, "bob", testPassword), ratelimit.AccountWindow)
	if rows := sessionRows(t, a); rows != 0 {
		t.Fatalf("sessions = %d", rows)
	}
}

func TestLockingOneAccountLeavesAnotherSignable(t *testing.T) {
	a := newApp(t, nil)
	a.withoutAddressLimit()
	a.addUser(t, "bob", false)
	a.addUser(t, "carol", false)
	for range ratelimit.AccountMaxAttempts + 1 {
		a.attemptLogin(t, "bob", wrongPassword)
	}
	wantStatus(t, a.attemptLogin(t, "carol", testPassword), http.StatusSeeOther)
}

// An invented name must throttle exactly like a real one, or the lockout
// would reveal which usernames exist.
func TestUnknownUsernameLocksLikeARealOne(t *testing.T) {
	a := newApp(t, nil)
	a.withoutAddressLimit()
	a.addUser(t, "bob", false)
	for range ratelimit.AccountMaxAttempts {
		wantStatus(t, a.attemptLogin(t, "no-such-user", wrongPassword), http.StatusOK)
	}
	wantThrottled(t, a.attemptLogin(t, "no-such-user", wrongPassword), ratelimit.AccountWindow)
}

func TestSuccessfulLoginClearsTheAccountBudget(t *testing.T) {
	a := newApp(t, nil)
	a.withoutAddressLimit()
	a.addUser(t, "bob", false)
	for range ratelimit.AccountMaxAttempts - 1 {
		wantStatus(t, a.attemptLogin(t, "bob", wrongPassword), http.StatusOK)
	}
	wantStatus(t, a.attemptLogin(t, "bob", testPassword), http.StatusSeeOther)
	a.client.ClearCookies()
	for range ratelimit.AccountMaxAttempts {
		wantStatus(t, a.attemptLogin(t, "bob", wrongPassword), http.StatusOK)
	}
	wantThrottled(t, a.attemptLogin(t, "bob", wrongPassword), ratelimit.AccountWindow)
}

// The unknown-user path costs one argon2 verification too. Loose on purpose:
// timing tests flake, so only a gross gap (a skipped hash) fails.
func TestUnknownUserCostsAboutAsMuchAsAKnownOne(t *testing.T) {
	a := newApp(t, nil)
	a.withoutAddressLimit()
	a.addUser(t, "bob", false)
	elapsed := func(username string) time.Duration {
		start := time.Now()
		a.attemptLogin(t, username, wrongPassword)
		return time.Since(start)
	}
	elapsed("bob")
	known, unknown := elapsed("bob"), elapsed("nobody")
	if unknown*2 < known {
		t.Fatalf("unknown user took %v against %v for a known one", unknown, known)
	}
}

func loginFailures(t *testing.T, a *app) []map[string]any {
	return a.logs.EventsNamed(t, "login.failed")
}

func TestWrongPasswordIsLoggedWithoutThePassword(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "alice", false)
	a.attemptLogin(t, "alice", wrongPassword)
	events := loginFailures(t, a)
	if len(events) != 1 || events[0]["reason"] != "bad_credentials" || events[0]["username"] != "alice" {
		t.Fatalf("events = %v", events)
	}
	if strings.Contains(a.logs.String(), wrongPassword) {
		t.Fatal("the submitted password must never be logged")
	}
}

func TestUnknownUsernameIsLoggedLikeAWrongPassword(t *testing.T) {
	a := newApp(t, nil)
	a.attemptLogin(t, "nobody", wrongPassword)
	events := loginFailures(t, a)
	if len(events) != 1 || events[0]["reason"] != "bad_credentials" || events[0]["username"] != "nobody" {
		t.Fatalf("events = %v", events)
	}
}

func TestDisabledAccountLogsItsOwnReason(t *testing.T) {
	a := newApp(t, nil)
	id := a.addUser(t, "banned", false)
	if err := a.store.SetUserDisabled(t.Context(), id, true); err != nil {
		t.Fatal(err)
	}
	a.attemptLogin(t, "banned", testPassword)
	if events := loginFailures(t, a); len(events) != 1 || events[0]["reason"] != "account_disabled" {
		t.Fatalf("events = %v", events)
	}
}

func TestHittingTheRateLimitIsLoggedWithItsBucket(t *testing.T) {
	a := newApp(t, nil)
	a.addUser(t, "alice", false)
	for range ratelimit.LoginMaxAttempts + 1 {
		a.attemptLogin(t, "alice", wrongPassword)
	}
	events := loginFailures(t, a)
	last := events[len(events)-1]
	if last["reason"] != "rate_limited" || last["bucket"] != "127.0.0.1" {
		t.Fatalf("last event = %v", last)
	}
}

func TestLockingAnAccountLogsItsOwnReason(t *testing.T) {
	a := newApp(t, nil)
	a.withoutAddressLimit()
	a.addUser(t, "alice", false)
	for range ratelimit.AccountMaxAttempts + 1 {
		a.attemptLogin(t, "alice", wrongPassword)
	}
	events := loginFailures(t, a)
	if last := events[len(events)-1]; last["reason"] != "account_locked" || last["username"] != "alice" {
		t.Fatalf("last event = %v", last)
	}
}

// Structured logging keeps a forged newline inside one field, so the log
// stays one event per line.
func TestForgedNewlineInTheUsernameCannotOpenASecondLogLine(t *testing.T) {
	a := newApp(t, nil)
	a.attemptLogin(t, "eve\nERROR login.succeeded user_id=1", wrongPassword)
	if lines := strings.Count(strings.TrimSpace(a.logs.String()), "\n"); lines != 0 {
		t.Fatalf("the log has %d extra lines:\n%s", lines, a.logs.String())
	}
	events := loginFailures(t, a)
	if len(events) != 1 {
		t.Fatalf("events = %v", events)
	}
	if username, _ := events[0]["username"].(string); !strings.HasPrefix(username, "eve") {
		t.Fatalf("the username must still be logged, got %q", events[0]["username"])
	}
	if !strings.Contains(a.logs.String(), `\n`) {
		t.Fatal("the newline must be escaped inside the field")
	}
}

func TestOversizedUsernameIsTruncatedInTheLog(t *testing.T) {
	a := newApp(t, nil)
	a.attemptLogin(t, strings.Repeat("x", 10_000), wrongPassword)
	log := a.logs.String()
	if len(log) > 1_000 || !strings.Contains(log, "…") {
		t.Fatalf("a 10k username produced a %d-byte log", len(log))
	}
}

func TestWrongCurrentPasswordOnAccountIsLoggedWithItsSurface(t *testing.T) {
	a, _ := memberApp(t)
	a.post(t, "/account/password", passwordForm(wrongPassword, "a brand new passphrase", "a brand new passphrase"))
	a.post(t, "/account/api-keys", keyForm(wrongPassword, "ci", ""))
	events := a.logs.EventsNamed(t, "reauth.failed")
	if len(events) != 2 || events[0]["surface"] != "password_change" || events[1]["surface"] != "api_key_create" {
		t.Fatalf("events = %v", events)
	}
	for _, event := range events {
		if event["reason"] != "bad_current_password" {
			t.Fatalf("event = %v", event)
		}
	}
	if strings.Contains(a.logs.String(), wrongPassword) {
		t.Fatal("the submitted password must never be logged")
	}
}
