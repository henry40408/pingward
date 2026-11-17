package views

import (
	"regexp"
	"strings"
	"testing"
	"time"
)

var (
	datalists    = regexp.MustCompile(`<datalist id="(tz-list|dur-list)">.*?</datalist>`)
	environment  = regexp.MustCompile(`<div class="card"><div class="ch"><h2>Environment</h2>.*?</div></div><div class="card"><div class="ch"><h2>Admin actions`)
	rustTimeZone = time.UTC
)

// at parses the nanosecond timestamps the Rust capture printed.
func at(t *testing.T, s string) time.Time {
	t.Helper()
	parsed, err := time.Parse(time.RFC3339Nano, s)
	if err != nil {
		t.Fatal(err)
	}
	return parsed.In(rustTimeZone)
}

func str(s string) *string { return &s }

func TestChannelFormsMatchRust(t *testing.T) {
	sameAsGolden(t, "channel-new", ChannelForm(ChannelFormView{IsAdmin: true, ProjectID: 1, SMTPAvailable: true}))
	sameAsGolden(t, "admin-channel-new", ChannelForm(ChannelFormView{IsAdmin: true, Admin: true, ProjectID: 4, SMTPAvailable: true}))
}

func TestAccountPagesMatchRust(t *testing.T) {
	created := at(t, "2026-11-17T15:24:56.085469908+00:00")
	lastSeen := at(t, "2026-11-17T15:29:39.702156091+00:00")
	session := SessionRow{
		Handle: "8ae14b14c8ba7c2870926f581cfc0b88729d85a86df4a386d19f6ed37a6ccba6", CreatedAt: &created, LastSeenAt: &lastSeen,
		ExpiresAt: at(t, "2026-11-20T15:24:56.085469658+00:00"), UserAgent: str("curl/8.7.1"), IP: str("192.168.215.1"), Current: true,
	}
	used := at(t, "2026-11-17T15:25:37.265458376+00:00")
	expires := at(t, "2026-12-17T15:25:17.184262335+00:00")
	keys := []APIKeyRow{
		{ID: 2, Name: "script", Prefix: "pw_00a0e648", CreatedAt: at(t, "2026-11-17T15:25:21.077579152+00:00"), LastUsedAt: &used},
		{ID: 1, Name: "ci", Prefix: "pw_3c1ab0d7", CreatedAt: at(t, "2026-11-17T15:25:17.184268460+00:00"), ExpiresAt: &expires},
	}
	account := AccountView{CSRF: "c", IsAdmin: true, HasPassword: true, Sessions: []SessionRow{session}, Keys: keys}
	sameAsGolden(t, "account", Account(account))

	seen := at(t, "2026-11-17T15:25:00.901036239+00:00")
	session.LastSeenAt = &seen
	keys[0].LastUsedAt = nil
	account.Sessions, account.Keys = []SessionRow{session}, keys
	account.NewToken = "pw_00a0e648fc5141e6888cfb1e3e5400fbb5460013c9f84498ba4a7f8d7910186f"
	sameAsGolden(t, "account-key-created", Account(account))
}

func TestAccountHidesPasswordControlsWithoutAPassword(t *testing.T) {
	got := Account(AccountView{CSRF: "c", OtherCount: 2})
	for _, absent := range []string{"Change password", "key_current_password"} {
		if strings.Contains(got, absent) {
			t.Fatalf("%q rendered for a passwordless account", absent)
		}
	}
	if !strings.Contains(got, `data-testid="session-revoke-others"`) || !strings.Contains(got, `data-testid="sessions-empty"`) || !strings.Contains(got, `data-testid="api-keys-empty"`) {
		t.Fatal(got)
	}
}

func TestAccountMarksExpiredAndSSOEntries(t *testing.T) {
	when := at(t, "2026-11-17T15:25:17+00:00")
	got := Account(AccountView{
		CSRF: "c", HasPassword: true, PasswordError: "bad <pw>", KeyError: "no", PasswordFlash: "done",
		Sessions: []SessionRow{{Handle: "h", ExpiresAt: when, SSO: true}},
		Keys:     []APIKeyRow{{ID: 4, Name: "n", Prefix: "pw_x", CreatedAt: when, ExpiresAt: &when, Expired: true}},
	})
	for _, want := range []string{
		`data-testid="session-sso">SSO</span>`, `data-testid="api-key-expired-4">expired</span>`,
		`data-testid="password-error">bad &lt;pw&gt;</p>`, `data-testid="api-key-error">no</p>`, `data-testid="password-changed-flash">done</p>`,
		`<td class="mono">—</td>`, `<td class="mono">never</td>`,
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("missing %s", want)
		}
	}
}

func TestAdminUnlockMatchesRust(t *testing.T) {
	sameAsGolden(t, "admin-unlock", AdminUnlock(AdminUnlockView{IsAdmin: true, CSRF: "c", Applies: true, TTL: "15m"}))
}

func TestAdminUnlockStates(t *testing.T) {
	cases := map[string]AdminUnlockView{
		`data-testid="unlock-not-applicable"`: {Applies: false},
		`data-testid="unlock-state">Already confirmed &mdash; these actions stay available for another 3m.`: {Applies: true, Remaining: "3m"},
		`data-testid="unlock-bounced">b</p>`: {Applies: true, BouncedFlash: "b"},
		`data-testid="unlock-error">e</p>`:   {Applies: true, Error: "e"},
	}
	for want, view := range cases {
		if got := AdminUnlock(view); !strings.Contains(got, want) {
			t.Errorf("missing %s", want)
		}
	}
}

func TestAdminAuditFilterEmptyStateMatchesRust(t *testing.T) {
	got := string(AdminAudit(AdminAuditView{
		Actors: []string{"admin"}, Actions: []string{"admin.access", "settings.update", "user.create", "user.set_admin"}, Filtered: true, FilterAction: "unknown",
	}))
	want := golden(t, "admin-audit-filter")
	sameHTML(t, "admin-audit-filter", want, got)
}

func auditRows(t *testing.T) []AuditRow {
	t.Helper()
	row := func(iso, detail, action, target string) AuditRow {
		return AuditRow{
			Time: strings.Replace(iso[:19], "T", " ", 1) + " UTC", ISO: iso, Actor: "admin", Action: action, Target: target,
			MethodPath: "POST /admin/settings", TargetOwner: "—", Detail: detail, Expandable: true,
		}
	}
	return []AuditRow{
		row("2026-11-17T15:26:49.306835686+00:00", "scan_interval=30 nag_interval=3600", "settings.update", "—"),
		row("2026-11-17T15:25:58.944365260+00:00", "scan_interval=1 nag_interval=3", "settings.update", "—"),
		row("2026-11-17T15:25:17.203925110+00:00", "scan_interval=30 nag_interval=3600 pings_retention_days=30 notifications_retention_days=60 audit_retention_days=90 display_timezone=Asia/Taipei", "settings.update", "—"),
		{Time: "2026-11-17 15:25:00 UTC", ISO: "2026-11-17T15:25:00.990279103+00:00", Actor: "admin", Action: "user.create", Target: "user #2",
			MethodPath: "—", TargetOwner: "—", Detail: "member", Expandable: true},
	}
}

func recentFailures(t *testing.T) []FailedNotification {
	t.Helper()
	smtp := "notify failed: smtp send failed: Connection error: failed to lookup address information: Name does not resolve"
	bad := "notify failed: status 400 Bad Request"
	unauthorized := "notify failed: status 401 Unauthorized"
	n := func(iso, event, message string) FailedNotification {
		return FailedNotification{CreatedAt: at(t, iso), Event: event, CheckID: 6, Error: message}
	}
	const a, b, c, d = "2026-11-17T15:26:17.395988706+00:00", "2026-11-17T15:26:14.372389599+00:00", "2026-11-17T15:26:11.353443081+00:00", "2026-11-17T15:26:08.324645384+00:00"
	return []FailedNotification{
		n(a, "reminder", smtp), n(a, "reminder", bad), n(b, "reminder", smtp), n(a, "reminder", unauthorized), n(b, "reminder", bad),
		n(c, "reminder", smtp), n(b, "reminder", unauthorized), n(c, "reminder", bad), n(d, "down", smtp), n(c, "reminder", unauthorized),
	}
}

func TestAdminPageMatchesRustExceptTheEnvironmentTable(t *testing.T) {
	audit := AdminAudit(AdminAuditView{Rows: auditRows(t), Actors: []string{"admin"}, Actions: []string{"settings.update", "user.create"}})
	got := Admin(AdminView{
		CSRF: "c", AuditPartial: audit,
		UserCount: 2, ProjectCount: 3, Checks: 6, Pings24h: 11,
		Status:  StatusCounts{New: 2, Up: 1, Down: 1, Paused: 2},
		Down:    []DownCheck{{ID: 1, Name: "nightly-backup", Project: "Alpha", Owner: "admin"}},
		NotifOK: 32, NotifErr: 12,
		ChannelFailures: []ChannelFailure{
			{"ch-telegram", 0, 4}, {"ch-pushover", 0, 4}, {"ch-email", 0, 4}, {"ch-webhook", 15, 0}, {"ch-slack", 4, 0}, {"ch-ntfy", 13, 0},
		},
		RecentFailures: recentFailures(t),
		LastScan:       LoopStamp{At: "2026-11-17T15:29:19.685711823+00:00", UTC: "2026-11-17 15:29:19 UTC", Ago: "20s ago"},
		LastPrune:      LoopStamp{At: "2026-11-17T15:24:38.290708752+00:00", UTC: "2026-11-17 15:24:38 UTC", Ago: "5m ago"},
		ScanInterval:   "30s", NagInterval: "1h", PingsRetentionDays: "30", NotificationsRetentionDays: "60", AuditRetentionDays: "90",
		DisplayTimezone:  "Asia/Taipei",
		Users:            []AdminUserRow{{ID: 1, Username: "admin", IsAdmin: true, IsSelf: true}, {ID: 2, Username: "bob"}},
		ElevationFlash:   "Confirmed. If an action was refused a moment ago it was not performed — do it again now.",
		ElevationApplies: true, ElevationRemaining: "12m57s",
		Projects: []AdminProjectRow{{1, "Alpha", "admin"}, {2, "Beta", "admin"}, {4, "BobProject", "bob"}},
	})
	strip := func(html string) string {
		html = datalists.ReplaceAllString(html, "")
		return environment.ReplaceAllString(html, "ENV")
	}
	sameHTML(t, "admin", strip(golden(t, "admin")), strip(got))
}

func TestAdminPageGatesControlsWhileLocked(t *testing.T) {
	got := Admin(AdminView{
		CSRF: "c", ElevationApplies: true, ElevationLocked: true,
		Users: []AdminUserRow{{ID: 2, Username: "bob"}, {ID: 3, Username: "cy", IsAdmin: true}},
		Env: []EnvGroup{{Title: "Server", Rows: []EnvSetting{
			{Var: "PINGWARD_SMTP_PASSWORD", Kind: EnvSecretSet}, {Var: "A", Kind: EnvText, Value: "<v>"}, {Var: "B", Kind: EnvUnset}, {Var: "C", Kind: EnvSecretNotSet},
		}}},
	})
	for _, want := range []string{
		`action="/admin/users" data-reauth="create this user"`,
		`action="/admin/users/2/password" data-reauth="reset this user's password"`,
		`action="/admin/users/2/admin" data-reauth="grant admin rights"`,
		`action="/admin/users/3/admin" data-confirm="Revoke admin rights from this user?"`,
		`data-testid="elevation-confirm-link"`,
		`<td data-testid="env-smtp-password"><span class="pill ok">configured</span></td>`,
		`<td><code>&lt;v&gt;</code></td>`,
		`<td><span class="note">not set</span></td><td class="mono"></td>`,
	} {
		if !strings.Contains(got, want) {
			t.Errorf("missing %s", want)
		}
	}
	if strings.Contains(got, `action="/admin/users/3/admin" data-confirm="Revoke admin rights from this user?" data-reauth`) {
		t.Error("revoking admin must not be gated")
	}
}

func TestChannelEditPagesMatchRust(t *testing.T) {
	edits := []ChannelEditView{
		{ID: 1, Kind: "webhook", Name: "ch-webhook", HasWebhookURL: true},
		{ID: 2, Kind: "slack", Name: "ch-slack", HasSlackURL: true},
		{ID: 3, Kind: "telegram", Name: "ch-telegram", HasTelegramToken: true, TelegramChatID: "42"},
		{ID: 4, Kind: "ntfy", Name: "ch-ntfy", HasNtfyToken: true, NtfyBaseURL: "http://host.docker.internal:19999", NtfyTopic: "pw"},
		{ID: 5, Kind: "pushover", Name: "ch-pushover", HasPushoverToken: true, HasPushoverUser: true},
		{ID: 6, Kind: "email", Name: "ch-email", EmailTo: "ops@example.com"},
	}
	for _, edit := range edits {
		name := "channel-edit-" + num(edit.ID)
		sameAsGolden(t, name, ChannelForm(ChannelFormView{IsAdmin: true, ProjectID: 1, SMTPAvailable: true, Edit: &edit}))
	}
	edit := ChannelEditView{ID: 8, Kind: "webhook", Name: "bob-hook", HasWebhookURL: true}
	sameAsGolden(t, "admin-channel-edit", ChannelForm(ChannelFormView{IsAdmin: true, Admin: true, ProjectID: 4, SMTPAvailable: true, Edit: &edit}))
}

func TestChannelFormNeverRendersASecretAndGatesEmailOnSMTP(t *testing.T) {
	if got := ChannelForm(ChannelFormView{CSRF: "c", ProjectID: 1}); strings.Contains(got, `value="email"`) || strings.Contains(got, `data-kind="email"`) {
		t.Fatal("email offered without SMTP")
	}
	edit := ChannelEditView{ID: 6, Kind: "email", Name: "n", EmailTo: "a@b"}
	if got := ChannelForm(ChannelFormView{CSRF: "c", ProjectID: 1, Edit: &edit, Error: "bad"}); !strings.Contains(got, `data-kind="email"`) || !strings.Contains(got, `<p class="flash err">bad</p>`) {
		t.Fatal("an existing email channel must stay editable with SMTP off")
	}
}
