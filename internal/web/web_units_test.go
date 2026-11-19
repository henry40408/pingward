package web

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/config"
	"github.com/henry40408/pingward/internal/models"
)

// Ported from the src/web.rs unit tests.

func testConfig(env map[string]string) *config.Config {
	cfg := config.Load(func(key string) (string, bool) {
		value, ok := env[key]
		return value, ok
	})
	return &cfg
}

func testDeps(env map[string]string) *Deps {
	return &Deps{Config: testConfig(env), Now: time.Now}
}

func baseCheck(now time.Time) models.Check {
	period := int64(3600)
	return models.Check{
		ID: 1, ProjectID: 1, Name: "c", PingUUID: "u", ScheduleKind: models.SchedulePeriod,
		PeriodSecs: &period, GraceSecs: 300, Timezone: "UTC", Status: models.StatusDown, CreatedAt: now,
	}
}

func TestNotifEventPillClassMapsEachEventToThePingKindPalette(t *testing.T) {
	want := map[models.EventKind]string{
		models.EventUp: "ok", models.EventDown: "fail", models.EventReminder: "start", models.EventTest: "log",
	}
	for event, class := range want {
		if got := notifEventPillClass(event); got != class {
			t.Errorf("notifEventPillClass(%s) = %q, want %q", event, got, class)
		}
	}
}

func TestStatusSinceLabelDownNeverPingedReadsNoPingsYet(t *testing.T) {
	now := time.Now()
	if got, want := statusSinceLabel(baseCheck(now), now), "down · no pings yet · not acknowledged"; got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestStatusSinceLabelDownWithPingShowsRelativeTime(t *testing.T) {
	now := time.Now()
	check := baseCheck(now)
	last := now.Add(-120 * time.Second)
	check.LastPingAt = &last
	if got, want := statusSinceLabel(check, now), "down · 2m ago · not acknowledged"; got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestScheduleLabelUsesDurationFormatAndShowsCronGrace(t *testing.T) {
	check := baseCheck(time.Now())
	if got, want := scheduleLabel(check), "every 1h · 5m grace"; got != want {
		t.Errorf("got %q, want %q", got, want)
	}
	expr := "0 0 * * * *"
	check.ScheduleKind, check.PeriodSecs, check.CronExpr, check.GraceSecs = models.ScheduleCron, nil, &expr, 600
	if got, want := scheduleLabel(check), "0 0 * * * * · 10m grace"; got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

// parse_opt_positive's Go counterpart is the days setting kind.
func TestPositiveIntegerSettingBlankIsEmpty(t *testing.T) {
	for _, blank := range []string{"", "   "} {
		if got, err := validateSetting(blank, "x", settingDays); err != nil || got != "" {
			t.Errorf("validateSetting(%q) = %q, %v; want blank and no error", blank, got, err)
		}
	}
}

func TestPositiveIntegerSettingAcceptsPositive(t *testing.T) {
	if got, err := validateSetting("5", "x", settingDays); err != nil || got != "5" {
		t.Errorf("got %q, %v", got, err)
	}
}

func TestPositiveIntegerSettingRejectsZeroNegativeAndNonNumeric(t *testing.T) {
	_, err := validateSetting("0", "Scan interval", settingDays)
	if err == nil || err.Error() != "Scan interval must be a positive integer." {
		t.Errorf("zero: got %v", err)
	}
	for _, bad := range []string{"-3", "abc"} {
		if _, err := validateSetting(bad, "x", settingDays); err == nil {
			t.Errorf("%q must be refused", bad)
		}
	}
}

func TestOptionalDurationBlankIsNone(t *testing.T) {
	for _, blank := range []string{"", "   "} {
		if got, refusal := parseOptionalDuration(blank, "x"); got != nil || refusal != "" {
			t.Errorf("parseOptionalDuration(%q) = %v, %q", blank, got, refusal)
		}
	}
}

func TestOptionalDurationAcceptsHumanReadable(t *testing.T) {
	if got, refusal := parseOptionalDuration("5m", "x"); refusal != "" || got == nil || *got != 300 {
		t.Errorf("got %v, %q", got, refusal)
	}
}

func TestOptionalDurationRejectsZeroNegativeAndInvalid(t *testing.T) {
	for _, bad := range []string{"0", "-3", "1x"} {
		got, refusal := parseOptionalDuration(bad, "x")
		if got != nil || refusal != "x must be a positive duration (e.g. 30, 5m, 1h30m)." {
			t.Errorf("%q: got %v, %q", bad, got, refusal)
		}
	}
}

func baseProjectInput() projectInput { return projectInput{name: "proj"} }

func wantSeconds(t *testing.T, label string, got *int64, want int64) {
	t.Helper()
	if got == nil || *got != want {
		t.Errorf("%s = %v, want %d", label, got, want)
	}
}

func TestValidateProjectAcceptsBlankAndPositive(t *testing.T) {
	settings, refusal := validateProject(baseProjectInput())
	if refusal != "" || settings.Name != "proj" || settings.Description != "" || settings.ScanIntervalSecs != nil || settings.NagIntervalSecs != nil {
		t.Errorf("blank form: %+v, %q", settings, refusal)
	}
	input := baseProjectInput()
	input.scan, input.nag = "15", "25"
	settings, refusal = validateProject(input)
	if refusal != "" {
		t.Fatal(refusal)
	}
	wantSeconds(t, "scan", settings.ScanIntervalSecs, 15)
	wantSeconds(t, "nag", settings.NagIntervalSecs, 25)
}

func TestValidateProjectRejectsNonNumericAndZero(t *testing.T) {
	input := baseProjectInput()
	input.scan = "abc"
	if _, refusal := validateProject(input); refusal == "" {
		t.Error("a non-numeric scan interval must be refused")
	}
	input = baseProjectInput()
	input.nag = "0"
	if _, refusal := validateProject(input); refusal == "" {
		t.Error("a zero nag interval must be refused")
	}
}

func TestValidateProjectAcceptsHumanReadableDurations(t *testing.T) {
	input := baseProjectInput()
	input.scan, input.nag = "5m", "1h"
	settings, refusal := validateProject(input)
	if refusal != "" {
		t.Fatal(refusal)
	}
	wantSeconds(t, "scan", settings.ScanIntervalSecs, 300)
	wantSeconds(t, "nag", settings.NagIntervalSecs, 3600)
}

func TestValidateProjectRejectsAnEmptyOrWhitespaceOnlyName(t *testing.T) {
	for _, name := range []string{"", "   "} {
		input := baseProjectInput()
		input.name = name
		if _, refusal := validateProject(input); refusal != "Name is required." {
			t.Errorf("name %q: refusal %q", name, refusal)
		}
	}
}

func TestValidateProjectTrimsTheName(t *testing.T) {
	input := baseProjectInput()
	input.name = "  Nightly jobs  "
	settings, refusal := validateProject(input)
	if refusal != "" || settings.Name != "Nightly jobs" {
		t.Errorf("got %q, %q", settings.Name, refusal)
	}
}

func TestReadableSettingDurationFormatsSecondsAndPassesThroughTheRest(t *testing.T) {
	cases := map[string]string{"3600": "1h", "45": "45s", "": "", "0": "0", "abc": "abc"}
	for input, want := range cases {
		if got := readableSettingDuration(input); got != want {
			t.Errorf("readableSettingDuration(%q) = %q, want %q", input, got, want)
		}
	}
}

// flashRequest carries a flash cookie built by the production builder, so name
// and signature are real.
func flashRequest(d *Deps, surface string) *http.Request {
	request := httptest.NewRequest(http.MethodGet, "/", nil)
	request.AddCookie(d.flashCookie(surface))
	return request
}

func takeFlashFor(d *Deps, request *http.Request, surface string) (string, bool, *httptest.ResponseRecorder) {
	recorder := httptest.NewRecorder()
	message, ok := d.takeFlash(recorder, request, surface)
	return message, ok, recorder
}

func TestTakeFlashMapsEachSurfaceToItsOwnMessage(t *testing.T) {
	d := testDeps(nil)
	want := map[string]string{
		"settings": "Settings saved.", "channels": "Notify channels saved.",
		"users_blocked":       "That action was refused: you cannot remove your own access, and the last enabled admin cannot be removed.",
		"forward_auth_logout": "Signed out locally, but you're authenticated through your reverse proxy — this app can't end that session. To sign out completely, log out at your proxy or SSO provider.",
	}
	for surface, message := range want {
		got, ok, _ := takeFlashFor(d, flashRequest(d, surface), surface)
		if !ok || got != message {
			t.Errorf("%s: got %q, %t", surface, got, ok)
		}
	}
}

func TestTakeFlashIgnoresAFlashSetForAnotherSurface(t *testing.T) {
	// Path "/" means every page sees it; only its own page may consume it.
	d := testDeps(nil)
	request := flashRequest(d, "channels")
	if _, ok, recorder := takeFlashFor(d, request, "settings"); ok || len(recorder.Result().Cookies()) != 0 {
		t.Fatal("the settings page consumed the channels flash")
	}
	if message, ok, _ := takeFlashFor(d, request, "channels"); !ok || message != "Notify channels saved." {
		t.Errorf("the channels page lost its flash: %q, %t", message, ok)
	}
}

func TestTakeFlashWithoutACookieIsNone(t *testing.T) {
	d := testDeps(nil)
	if _, ok, _ := takeFlashFor(d, httptest.NewRequest(http.MethodGet, "/", nil), "settings"); ok {
		t.Error("a flash appeared from nowhere")
	}
}

func TestTakeFlashNeverRendersAnUnknownCookieValue(t *testing.T) {
	// Even with a matching surface, an unknown key maps to no message.
	d := testDeps(nil)
	if message, ok, _ := takeFlashFor(d, flashRequest(d, "<script>"), "<script>"); ok || message != "" {
		t.Errorf("rendered %q", message)
	}
}

// An unsigned flash (a sibling subdomain over plain HTTP) is ignored by both readers.
func TestAnUnsignedFlashCookieIsIgnored(t *testing.T) {
	d := testDeps(nil)
	request := httptest.NewRequest(http.MethodGet, "/", nil)
	request.AddCookie(&http.Cookie{Name: d.flashCookieName(), Value: "settings"})
	if _, ok, _ := takeFlashFor(d, request, "settings"); ok {
		t.Error("an unsigned settings flash was accepted")
	}

	request = httptest.NewRequest(http.MethodGet, "/", nil)
	request.AddCookie(&http.Cookie{Name: d.flashCookieName(), Value: "password_reset_keys:1:99"})
	if got := d.takePasswordResetKeysFlash(httptest.NewRecorder(), request); got != "" {
		t.Errorf("an unsigned password-reset flash was accepted: %q", got)
	}

	// Signed under a different secret: same rejection.
	other := testDeps(map[string]string{"PINGWARD_SECRET": "another-test-secret-32-bytes-x"})
	if _, ok, _ := takeFlashFor(d, flashRequest(other, "settings"), "settings"); ok {
		t.Error("a flash signed with another secret was accepted")
	}
}

// __Host- requires Secure, Path=/ and no Domain.
func TestASecureFlashCookieIsHostPrefixedAndPrefixLegal(t *testing.T) {
	secure := testDeps(map[string]string{"PINGWARD_COOKIE_SECURE": "true"})
	cookie := secure.flashCookie("settings")
	if cookie.Name != flashCookieHostPrefixed || !cookie.Secure || cookie.Path != "/" || cookie.Domain != "" {
		t.Errorf("secure flash cookie: %+v", cookie)
	}

	// Unprefixed when not Secure, or the browser rejects it.
	plain := testDeps(nil)
	if plain.Config.CookieSecure {
		t.Fatal("the default test config must not be Secure")
	}
	if name := plain.flashCookie("settings").Name; name != flashCookieBase {
		t.Errorf("plain flash cookie name = %q", name)
	}
}

func baseCheckInput() checkInput {
	return checkInput{name: "c", scheduleKind: "period", periodSecs: "3600", graceSecs: "300", timezone: "UTC"}
}

func wantCheckRefusal(t *testing.T, in checkInput, want string) {
	t.Helper()
	if _, refusal := validateCheck(in); refusal != want {
		t.Errorf("refusal = %q, want %q", refusal, want)
	}
}

func TestValidateCheckAcceptsAValidPeriodForm(t *testing.T) {
	if _, refusal := validateCheck(baseCheckInput()); refusal != "" {
		t.Errorf("refused: %s", refusal)
	}
}

func TestValidateCheckRejectsAnUnknownTimezone(t *testing.T) {
	in := baseCheckInput()
	in.timezone = "Asia/Taipeh"
	_, refusal := validateCheck(in)
	if !strings.Contains(refusal, "Asia/Taipeh") || !strings.Contains(refusal, "IANA") {
		t.Errorf("got %q", refusal)
	}
}

func TestValidateCheckCarriesTheValidatedTimezone(t *testing.T) {
	in := baseCheckInput()
	in.timezone = "  Asia/Taipei  "
	valid, _ := validateCheck(in)
	if valid.timezone != "Asia/Taipei" {
		t.Errorf("timezone = %q", valid.timezone)
	}
}

func TestValidateTimezoneTreatsBlankAsUTCAndCanonicalizes(t *testing.T) {
	for raw, want := range map[string]string{"": "UTC", "   ": "UTC", "Europe/Berlin": "Europe/Berlin"} {
		if got, err := validateTimezone(raw); err != nil || got != want {
			t.Errorf("validateTimezone(%q) = %q, %v; want %q", raw, got, err, want)
		}
	}
	if _, err := validateTimezone("Mars/Olympus"); err == nil {
		t.Error("Mars/Olympus must be refused")
	}
}

// Blank means unset for a setting, so the check's own zone still applies.
func TestOptionalTimezoneSettingTreatsBlankAsUnset(t *testing.T) {
	if got, err := validateSetting("", "x", settingTimezone); err != nil || got != "" {
		t.Errorf("blank: %q, %v", got, err)
	}
	if got, err := validateSetting(" Europe/Berlin ", "x", settingTimezone); err != nil || got != "Europe/Berlin" {
		t.Errorf("padded: %q, %v", got, err)
	}
	if _, err := validateSetting("nonsense", "x", settingTimezone); err == nil {
		t.Error("nonsense must be refused")
	}
}

func TestValidateCheckRejectsAnEmptyOrWhitespaceOnlyName(t *testing.T) {
	for _, name := range []string{"", "   "} {
		in := baseCheckInput()
		in.name = name
		wantCheckRefusal(t, in, "Name is required.")
	}
}

func TestValidateCheckTrimsTheName(t *testing.T) {
	in := baseCheckInput()
	in.name = "  backup  "
	if valid, _ := validateCheck(in); valid.name != "backup" {
		t.Errorf("name = %q", valid.name)
	}
}

func TestValidateCheckAcceptsPositiveOverrides(t *testing.T) {
	in := baseCheckInput()
	in.scanInterval, in.maxRuntime, in.nag = "10", "20", "30"
	valid, refusal := validateCheck(in)
	if refusal != "" {
		t.Fatal(refusal)
	}
	wantSeconds(t, "scan", valid.scanInterval, 10)
	wantSeconds(t, "max runtime", valid.maxRuntime, 20)
	wantSeconds(t, "nag", valid.nag, 30)
}

func TestValidateCheckRejectsABadOverride(t *testing.T) {
	in := baseCheckInput()
	in.scanInterval = "abc"
	wantCheckRefusal(t, in, "Scan interval must be a positive duration (e.g. 30, 5m, 1h30m).")
	in = baseCheckInput()
	in.maxRuntime = "0"
	wantCheckRefusal(t, in, "Max runtime must be a positive duration (e.g. 30, 5m, 1h30m).")
}

func TestValidateCheckAcceptsHumanReadableDurations(t *testing.T) {
	in := checkInput{
		name: "c", scheduleKind: "period", periodSecs: "1h30m", graceSecs: "5m", timezone: "UTC",
		scanInterval: "30s", maxRuntime: "2m", nag: "1h",
	}
	valid, refusal := validateCheck(in)
	if refusal != "" {
		t.Fatal(refusal)
	}
	wantSeconds(t, "period", valid.periodSecs, 5400)
	if valid.graceSecs != 300 {
		t.Errorf("grace = %d", valid.graceSecs)
	}
	wantSeconds(t, "scan", valid.scanInterval, 30)
	wantSeconds(t, "max runtime", valid.maxRuntime, 120)
	wantSeconds(t, "nag", valid.nag, 3600)
}

func TestValidateCheckScheduleRefusals(t *testing.T) {
	cases := []struct {
		mutate func(*checkInput)
		want   string
	}{
		{func(in *checkInput) { in.scheduleKind = "weekly" }, "Invalid schedule kind."},
		{func(in *checkInput) { in.periodSecs = " " }, "Period is required in period mode."},
		{func(in *checkInput) { in.periodSecs = "0" }, "Period must be greater than 0."},
		{func(in *checkInput) { in.graceSecs = "x" }, "Grace period must be a duration (e.g. 30, 5m, 1h30m)."},
		{func(in *checkInput) { in.graceSecs = "-1" }, "Grace period must be 0 or more."},
		{func(in *checkInput) { in.scheduleKind, in.cronExpr = "cron", "" }, "Cron expression is required in cron mode."},
	}
	for _, c := range cases {
		in := baseCheckInput()
		c.mutate(&in)
		wantCheckRefusal(t, in, c.want)
	}
	in := baseCheckInput()
	in.scheduleKind, in.cronExpr = "cron", "not a cron"
	if _, refusal := validateCheck(in); !strings.HasPrefix(refusal, "Invalid cron expression: ") || !strings.HasSuffix(refusal, ".") {
		t.Errorf("got %q", refusal)
	}
}
