package server_test

import (
	"net/http"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/apikey"
	"github.com/henry40408/pingward/internal/store"
)

func TestNavShowsTheAccountLinkForAMember(t *testing.T) {
	a, _ := memberApp(t)
	if !strings.Contains(a.client.Get("/").Body, "nav-account") {
		t.Fatal("a member must see the Account link")
	}
}

func TestPasswordLoginSessionHasNoSSOPill(t *testing.T) {
	a, _ := memberApp(t)
	body := a.client.Get("/account").Body
	if !strings.Contains(body, "session-current") {
		t.Fatal("non-vacuity: a session row must render")
	}
	if strings.Contains(body, "session-sso") {
		t.Fatal("a plain password-login session must not be flagged SSO")
	}
}

func TestSecondLoginListsTwoSessionsAndRevokeOthersLeavesOne(t *testing.T) {
	a, _ := memberApp(t)
	a.secondSession(t, "member")
	if n := strings.Count(a.client.Get("/account").Body, "session-row-"); n != 2 {
		t.Fatalf("listed %d sessions, want 2", n)
	}
	wantStatus(t, a.post(t, "/account/sessions/revoke-others?confirmed=1", url.Values{}), http.StatusSeeOther)
	body := a.client.Get("/account").Body
	if n := strings.Count(body, "session-row-"); n != 1 || !strings.Contains(body, "session-current") {
		t.Fatalf("only the current session must remain (%d rows)", n)
	}
}

func TestChangingThePasswordLeavesAPIKeysAlone(t *testing.T) {
	a, id := memberApp(t)
	wantStatus(t, a.post(t, "/account/api-keys", keyForm(testPassword, "ci", "")), http.StatusOK)
	next := "another-long-passphrase"
	wantStatus(t, a.post(t, "/account/password", passwordForm(testPassword, next, next)), http.StatusSeeOther)
	if keys, _ := a.store.ListAPIKeysForUser(t.Context(), id); len(keys) != 1 {
		t.Fatalf("a password change revokes sessions, not keys: %d", len(keys))
	}
}

func TestAccountPageLinksToTheDocs(t *testing.T) {
	a, _ := memberApp(t)
	body := a.client.Get("/account").Body
	for _, want := range []string{`data-testid="api-docs-link"`, `href="/api/docs"`, `href="/api/openapi.json"`} {
		if !strings.Contains(body, want) {
			t.Errorf("the account page is missing %s", want)
		}
	}
}

func (a *app) insertKey(t *testing.T, userID int64, name string, expiresAt *time.Time) (int64, apikey.Key) {
	t.Helper()
	key := apikey.Generate()
	id, err := a.store.InsertAPIKey(t.Context(), store.NewAPIKey{
		UserID: userID, Name: name, TokenHash: key.Hash, Prefix: key.Prefix, ExpiresAt: expiresAt,
	}, time.Now())
	if err != nil {
		t.Fatal(err)
	}
	return id, key
}

func TestAnExpiredKeyIsFlaggedButALiveOneIsNot(t *testing.T) {
	a, userID := memberApp(t)
	past, future := time.Now().Add(-time.Hour), time.Now().Add(30*24*time.Hour)
	dead, _ := a.insertKey(t, userID, "dead", &past)
	live, _ := a.insertKey(t, userID, "live", &future)
	body := a.client.Get("/account").Body
	if !strings.Contains(body, "api-key-expired-"+itoa(dead)) {
		t.Error("an expired key must carry the expired badge")
	}
	if strings.Contains(body, "api-key-expired-"+itoa(live)) {
		t.Error("a live key must not be flagged expired")
	}
}

func TestValidateRejectsExpiredAndUnknownKeys(t *testing.T) {
	a, userID := memberApp(t)
	past := time.Now().Add(-time.Hour)
	_, key := a.insertKey(t, userID, "old", &past)
	for _, hash := range []string{key.Hash, "deadbeef"} {
		if _, found, err := a.store.ValidateAPIKey(t.Context(), hash, time.Now()); err != nil || found {
			t.Fatalf("%s must not validate (found=%v err=%v)", hash, found, err)
		}
	}
}

func TestCreatingAKeyWithABadExpiryIsRejectedWithAMessage(t *testing.T) {
	a, id := memberApp(t)
	res := a.post(t, "/account/api-keys", keyForm(testPassword, "temp", "banana"))
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "expiry must be") {
		t.Fatalf("missing message: %s", res.Body)
	}
	if keys, _ := a.store.ListAPIKeysForUser(t.Context(), id); len(keys) != 0 {
		t.Fatal("nothing may be minted")
	}
}

func TestCreatingAKeyWithABlankNameIsRejectedWithAMessage(t *testing.T) {
	a, id := memberApp(t)
	res := a.post(t, "/account/api-keys", keyForm(testPassword, "   ", ""))
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "a name is required") {
		t.Fatalf("missing message: %s", res.Body)
	}
	if keys, _ := a.store.ListAPIKeysForUser(t.Context(), id); len(keys) != 0 {
		t.Fatal("nothing may be minted")
	}
}

func TestTheKeyFormAsksForThePassword(t *testing.T) {
	a, _ := memberApp(t)
	if !strings.Contains(a.client.Get("/account").Body, "api-key-password-input") {
		t.Fatal("a password account must be asked to re-authenticate")
	}
}
