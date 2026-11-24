package auth

import (
	"fmt"
	"strings"
	"testing"
	"time"
)

func TestValidateUsernameTrimsAndAcceptsOrdinaryNames(t *testing.T) {
	for in, want := range map[string]string{
		"  alice \n":          "alice",
		"a.b-c_d@example.com": "a.b-c_d@example.com",
		"王小明":                 "王小明",
	} {
		got, refusal := ValidateUsername(in)
		if refusal != "" || got != want {
			t.Errorf("%q: got %q, %q", in, got, refusal)
		}
	}
}

func TestValidateUsernameRefusesBlankNames(t *testing.T) {
	for _, blank := range []string{"", "   ", "\t\n"} {
		_, refusal := ValidateUsername(blank)
		if !strings.Contains(refusal, "required") {
			t.Errorf("%q: %q", blank, refusal)
		}
	}
}

func TestValidateUsernameCountsCharactersNotBytes(t *testing.T) {
	for _, ok := range []string{strings.Repeat("a", MaxUsernameChars), strings.Repeat("王", MaxUsernameChars)} {
		if _, refusal := ValidateUsername(ok); refusal != "" {
			t.Errorf("%q: %q", ok, refusal)
		}
	}
	_, refusal := ValidateUsername(strings.Repeat("a", MaxUsernameChars+1))
	if !strings.Contains(refusal, "at most 64") {
		t.Fatalf("got %q", refusal)
	}
}

func TestValidateUsernameRefusesControlCharacters(t *testing.T) {
	for _, bad := range []string{"ali\x00ce", "ali\nce", "ali\x7fce", "ali\x1b[0mce"} {
		_, refusal := ValidateUsername(bad)
		if !strings.Contains(refusal, "control") {
			t.Errorf("%q: %q", bad, refusal)
		}
	}
}

func TestHashThenVerifyRoundTrips(t *testing.T) {
	phc, err := HashPassword("correct horse battery")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(phc, "$argon2id$v=19$m=19456,t=2,p=1$") {
		t.Fatalf("unexpected PHC %q", phc)
	}
	if !VerifyPassword("correct horse battery", phc) || VerifyPassword("wrong", phc) {
		t.Fatal("verify mismatch")
	}
}

// rustPasswordVectors are PHC strings produced by the Rust v0.12.0 server
// (argon2 0.6 defaults m=19456,t=2,p=1), taken from the users table of its
// database, plus the argon2 0.5.3 vector from src/auth.rs
// verify_accepts_hash_from_argon2_0_5. The salts are random, so Go must verify
// them rather than reproduce them.
var rustPasswordVectors = []struct{ password, phc string }{
	{"correct horse battery staple", "$argon2id$v=19$m=19456,t=2,p=1$vHgAaDRCakSMcV1CO34qog$oO+4Exa3gccDO+ZP+T74abbYEm/n6dvNMAtQV5TTdsA"},
	{"日本語のパスワード十五文字以上です", "$argon2id$v=19$m=19456,t=2,p=1$IBRv0JBNjykisiXTNWm//Q$1BdvSGkAIg/c5Bm9N4W169G470UlE9N3YR3lpuAIf5g"},
	{"carol carol carol carol", "$argon2id$v=19$m=19456,t=2,p=1$eJFyRAuPyuNQWfNAjgwoRA$Bm+EasQtuLUcb/ghUJuzZiWczCQAgvtNgF/7iZq7DGg"},
	{"correct horse", "$argon2id$v=19$m=19456,t=2,p=1$7GpHNWosgZqygblv9zwMuA$mZ2ug4DZpfCWRzPvF3HZwnIhPZyrsljPLgfNdsPKXuk"},
}

func TestVerifyAcceptsRustHashes(t *testing.T) {
	const wrongPassword = "wrong horse"
	for _, vec := range rustPasswordVectors {
		if !VerifyPassword(vec.password, vec.phc) {
			t.Errorf("Rust hash for %q rejected", vec.password)
		}
		if VerifyPassword(wrongPassword, vec.phc) {
			t.Errorf("wrong password accepted for %q", vec.password)
		}
	}
}

func TestVerifyRejectsGarbageHash(t *testing.T) {
	for _, bad := range []string{
		"", "not a phc string", "$argon2id$", "$argon2i$v=19$m=19456,t=2,p=1$AAAA$AAAA",
		"$argon2id$v=16$m=19456,t=2,p=1$AAAA$AAAA", "$argon2id$v=19$m=0,t=2,p=1$AAAA$AAAA",
		"$argon2id$v=19$m=19456,t=2,p=256$AAAA$AAAA", "$argon2id$v=19$m=19456,t=2,p=1$$AAAA",
		"$argon2id$v=19$m=19456,t=2,p=1$AAAA$", "$argon2id$v=19$m=19456,t=2$AAAA$AAAA",
	} {
		if VerifyPassword("x", bad) {
			t.Errorf("%q must not verify", bad)
		}
	}
}

func TestPasswordPolicyEnforcesBothBounds(t *testing.T) {
	if ValidatePassword(strings.Repeat("a", MinPasswordChars-1)) != "Password must be at least 15 characters." {
		t.Error("min bound")
	}
	if ValidatePassword(strings.Repeat("a", MinPasswordChars)) != "" {
		t.Error("min accepted")
	}
	if ValidatePassword(strings.Repeat("a", MaxPasswordChars)) != "" {
		t.Error("max accepted")
	}
	if ValidatePassword(strings.Repeat("a", MaxPasswordChars+1)) != "Password must be at most 128 characters." {
		t.Error("max bound")
	}
}

func TestPasswordPolicyHasNoCompositionRules(t *testing.T) {
	for _, pw := range []string{
		"correcthorsebatterystaple", "123456789012345", "correct horse battery staple",
		"正確的馬電池釘書針這樣就夠長了吧", strings.Repeat("\t", 15),
	} {
		if msg := ValidatePassword(pw); msg != "" {
			t.Errorf("%q refused: %s", pw, msg)
		}
	}
}

func TestPasswordLengthIsCountedInCharactersNotBytes(t *testing.T) {
	cjk := "密碼密碼密碼密碼密碼密碼密碼"
	if len([]rune(cjk)) != MinPasswordChars-1 || len(cjk) <= MinPasswordChars {
		t.Fatal("fixture broken")
	}
	if ValidatePassword(cjk) == "" {
		t.Fatal("14 characters must be refused")
	}
}

func TestVerifyPasswordOrDummyHandlesAMissingHash(t *testing.T) {
	phc, err := HashPassword("correct horse battery")
	if err != nil {
		t.Fatal(err)
	}
	if !VerifyPasswordOrDummy("correct horse battery", &phc) {
		t.Error("right password")
	}
	if VerifyPasswordOrDummy("wrong", &phc) {
		t.Error("wrong password")
	}
	if VerifyPasswordOrDummy("anything at all", nil) {
		t.Error("missing hash must not verify")
	}
	dummy := dummyPasswordHash()
	if !strings.HasPrefix(dummy, "$argon2") || VerifyPassword("", dummy) {
		t.Error("dummy hash")
	}
	if dummy != dummyPasswordHash() {
		t.Error("dummy must be built once")
	}
}

func TestLogUsernameTruncates(t *testing.T) {
	if LogUsername("alice") != "alice" {
		t.Error("short name changed")
	}
	for _, r := range []string{"a", "漢"} {
		cut := LogUsername(strings.Repeat(r, logUsernameMaxChars*2))
		if len([]rune(cut)) != logUsernameMaxChars+1 || !strings.HasSuffix(cut, "…") {
			t.Errorf("%q: bad cut %q", r, cut)
		}
	}
	forged := "bob\nsession.created user_id=1"
	if LogUsername(forged) != forged {
		t.Error("newline must survive for the caller to quote")
	}
	if strings.Contains(fmt.Sprintf("%q", LogUsername(forged)), "\n") {
		t.Error("the quoted rendering must neutralise the newline")
	}
}

func TestSessionCookieNameIsPrefixedOnlyWhenSecure(t *testing.T) {
	if SessionCookieName(true) != SessionCookieHostPrefixed || SessionCookieName(false) != SessionCookieBase {
		t.Fatal("cookie names")
	}
}

func TestSessionTokenIsUniqueUUID(t *testing.T) {
	a, b := NewSessionToken(), NewSessionToken()
	if a == b || len(a) != 36 {
		t.Fatalf("%q %q", a, b)
	}
}

func TestSessionLogHandleIsNeverTheRawID(t *testing.T) {
	id := NewSessionToken()
	h := SessionLogHandle(id)
	if len(h) != 16 || strings.Contains(h, id) || h == id {
		t.Fatalf("handle %q", h)
	}
}

func ts(y int, m time.Month, d int) time.Time {
	return time.Date(y, m, d, 0, 0, 0, 0, time.UTC)
}

func ptr(t time.Time) *time.Time { return &t }

func TestRefreshedExpirySlidesOnlyPastTheHalfLife(t *testing.T) {
	now := ts(2026, 1, 1)
	created := ptr(now.Add(-24 * time.Hour))
	if got := RefreshedExpiry(created, now.Add(SessionIdleTTL-time.Hour), now); got != nil {
		t.Fatalf("more than half remains: %+v", got)
	}
	got := RefreshedExpiry(created, now.Add(SessionIdleTTL/2-time.Hour), now)
	if got == nil || got.Kind != RenewalSlid || !got.ExpiresAt.Equal(now.Add(SessionIdleTTL)) {
		t.Fatalf("got %+v", got)
	}
}

func TestRefreshedExpiryClampsToTheAbsoluteCap(t *testing.T) {
	created := ts(2026, 1, 1)
	now := created.Add(29*24*time.Hour + 23*time.Hour)
	cap := created.Add(SessionAbsoluteMax)
	got := RefreshedExpiry(&created, now, now)
	if got == nil || !got.ExpiresAt.Equal(cap) || got.Kind != RenewalSlid {
		t.Fatalf("got %+v", got)
	}
}

func TestRefreshedExpiryStopsAtTheCap(t *testing.T) {
	created := ts(2026, 1, 1)
	cap := created.Add(SessionAbsoluteMax)
	if RefreshedExpiry(&created, cap, cap) != nil || RefreshedExpiry(&created, cap.Add(time.Hour), cap) != nil {
		t.Fatal("must not renew at or past the cap")
	}
}

func TestRefreshedExpiryWithoutCreatedAtHasNoCap(t *testing.T) {
	now := ts(2026, 1, 1)
	got := RefreshedExpiry(nil, now.Add(time.Hour), now)
	if got == nil || got.Kind != RenewalSlid || !got.ExpiresAt.Equal(now.Add(SessionIdleTTL)) {
		t.Fatalf("got %+v", got)
	}
}

func TestRefreshedExpiryShortensALegacyRowImmediately(t *testing.T) {
	now := ts(2026, 1, 1)
	created := now.Add(-24 * time.Hour)
	legacy := created.Add(SessionAbsoluteMax - time.Second)
	got := RefreshedExpiry(&created, legacy, now)
	if got == nil || got.Kind != RenewalClamped || !got.ExpiresAt.Equal(now.Add(SessionIdleTTL)) {
		t.Fatalf("got %+v", got)
	}
}

func TestRefreshedExpiryDoesNotShortenAFreshlyCreatedRow(t *testing.T) {
	now := ts(2026, 1, 1)
	if RefreshedExpiry(&now, now.Add(SessionIdleTTL), now) != nil {
		t.Fatal("exactly now+idle must not write")
	}
}

func TestIsPastAbsoluteCapBoundary(t *testing.T) {
	created := ts(2026, 1, 1)
	cap := created.Add(SessionAbsoluteMax)
	if !IsPastAbsoluteCap(&created, cap) || IsPastAbsoluteCap(&created, cap.Add(-time.Second)) ||
		IsPastAbsoluteCap(nil, cap.Add(365*24*time.Hour)) {
		t.Fatal("cap boundary")
	}
}

// A hash made by the argon2 0.5 crate, the one the Rust build stored.
func TestVerifyAcceptsAHashFromArgon2Crate05(t *testing.T) {
	const phc = "$argon2id$v=19$m=19456,t=2,p=1$7GpHNWosgZqygblv9zwMuA$mZ2ug4DZpfCWRzPvF3HZwnIhPZyrsljPLgfNdsPKXuk"
	if !VerifyPassword("correct horse", phc) {
		t.Fatal("the stored Rust hash must verify")
	}
	if VerifyPassword("wrong horse", phc) {
		t.Fatal("a wrong password must not verify")
	}
}

func TestParsePHCAcceptsTheLargestThreadCount(t *testing.T) {
	const tail = "$AAAA$AAAA"
	p, ok := parsePHC("$argon2id$v=19$m=19456,t=2,p=255" + tail)
	if !ok || p.threads != 255 {
		t.Fatalf("p=255: got (%+v, %v)", p, ok)
	}
	if _, ok := parsePHC("$argon2id$v=19$m=19456,t=2,p=256" + tail); ok {
		t.Fatal("p=256 must be refused")
	}
}

func TestRefreshedExpiryDoesNotSlideAtExactlyHalfTheIdleWindow(t *testing.T) {
	now := ts(2026, 1, 1)
	created := ptr(now.Add(-24 * time.Hour))
	if got := RefreshedExpiry(created, now.Add(SessionIdleTTL/2), now); got != nil {
		t.Fatalf("exactly half remains: %+v", got)
	}
	if got := RefreshedExpiry(created, now.Add(SessionIdleTTL/2-time.Nanosecond), now); got == nil {
		t.Fatal("just under half remains: want a renewal")
	}
}

func TestLogUsernameKeepsExactlyTheMaximumLength(t *testing.T) {
	exact := strings.Repeat("a", logUsernameMaxChars)
	if got := LogUsername(exact); got != exact {
		t.Fatalf("got %q, want it untouched", got)
	}
	if got := LogUsername(exact + "a"); got != exact+"…" {
		t.Fatalf("one over: got %q", got)
	}
}
