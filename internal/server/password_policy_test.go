package server_test

import (
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"testing"

	"github.com/henry40408/pingward/internal/auth"
)

func TestAdminUserCreationRefusesAPasswordUnderTheFloor(t *testing.T) {
	a, _ := adminApp(t)
	res := a.post(t, "/admin/users", newUserForm("carol", "short"))
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "at least "+strconv.Itoa(auth.MinPasswordChars)+" characters") {
		t.Fatalf("the page must name the floor: %s", res.Body)
	}
	if !userMissing(t, a, "carol") {
		t.Fatal("a rejected creation must not create the account")
	}
}

func userMissing(t *testing.T, a *app, username string) bool {
	t.Helper()
	user, err := a.store.FindUserByUsername(t.Context(), username)
	return err == nil && user == nil
}

// A bare redirect to /admin would look like success.
func TestAdminPasswordResetRefusesAndSaysSoRatherThanRedirecting(t *testing.T) {
	a, memberID := adminApp(t)
	before := *a.userByName(t, "member").PasswordHash
	res := a.post(t, userPath(memberID, "password"), url.Values{"password": {"short"}})
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "at least "+strconv.Itoa(auth.MinPasswordChars)+" characters") {
		t.Fatalf("the page must name the floor: %s", res.Body)
	}
	if after := *a.userByName(t, "member").PasswordHash; after != before {
		t.Fatal("the stored credential must be untouched")
	}
	if slicesContain(a.auditActions(t), "user.password_reset") {
		t.Fatal("a refused reset must not be audited as a reset")
	}
}

// A silent truncation would authenticate a shorter prefix than the user set.
func TestAnAdminSetOverLongPasswordIsRefusedNotTruncated(t *testing.T) {
	a, _ := adminApp(t)
	res := a.post(t, "/admin/users", newUserForm("carol", strings.Repeat("a", auth.MaxPasswordChars+1)))
	wantStatus(t, res, http.StatusOK)
	if !strings.Contains(res.Body, "at most "+strconv.Itoa(auth.MaxPasswordChars)+" characters") {
		t.Fatalf("the page must name the ceiling: %s", res.Body)
	}
	if !userMissing(t, a, "carol") {
		t.Fatal("a rejected creation must not create the account")
	}
}
