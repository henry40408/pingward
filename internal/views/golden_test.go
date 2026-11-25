package views

import (
	"crypto/sha256"
	"encoding/hex"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/tz"
	"github.com/henry40408/pingward/internal/view"
)

// assertGolden compares the digest of the normalised page with the Rust
// server's. A mismatch writes the page to a temporary file; the Rust golden is
// in git history under internal/views/testdata.
func assertGolden(t *testing.T, name, html string) {
	t.Helper()
	want, ok := goldenDigests[name]
	if !ok {
		t.Fatalf("no golden digest named %s", name)
	}
	got := normalise(html)
	sum := sha256.Sum256([]byte(got))
	if hex.EncodeToString(sum[:]) == want {
		return
	}
	file, err := os.CreateTemp("", name+".actual.*.html")
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = file.Close() }()
	if _, err := file.WriteString(got); err != nil {
		t.Fatal(err)
	}
	// login-shell and dashboard-nav are cut from the login and dashboard goldens.
	source := strings.TrimSuffix(strings.TrimSuffix(name, "-shell"), "-nav")
	path := "internal/views/testdata/" + source + ".golden.html"
	t.Errorf("%s differs from the Rust golden (%d bytes of normalised output written to %s).\n"+
		"The golden is in git history: git show $(git log --diff-filter=D --format=%%h -1 -- %s)^:%s",
		name, len(got), file.Name(), path, path)
}

func TestLoginAndSetupMatchRust(t *testing.T) {
	assertGolden(t, "login", Login(AuthView{CSRF: "c"}))
	assertGolden(t, "setup", Setup(AuthView{CSRF: "c"}))
}

func TestAuthErrorsCarryTheirOwnTestID(t *testing.T) {
	if got := Login(AuthView{Error: "bad <x>"}); !strings.Contains(got, `<p class="flash err" data-testid="login-error">bad &lt;x&gt;</p>`) {
		t.Fatal(got)
	}
	if got := Setup(AuthView{Error: "no"}); !strings.Contains(got, `data-testid="setup-error">no</p>`) {
		t.Fatal(got)
	}
}

func TestEveryConfirmationPageMatchesRust(t *testing.T) {
	for _, c := range confirmCases {
		assertGolden(t, c.name, Confirm(ConfirmView{
			IsAdmin: c.isAdmin, CSRF: "c", Title: c.title, Message: c.message,
			Action: c.action, Button: c.button, Cancel: c.cancel,
		}))
	}
}

func TestProjectFormsMatchRust(t *testing.T) {
	assertGolden(t, "projects-new", ProjectForm(ProjectFormView{IsAdmin: true, Heading: "New project", Action: "/projects"}))
	assertGolden(t, "project-edit", ProjectForm(ProjectFormView{
		IsAdmin: true, Heading: "Edit project", Action: "/projects/1", Name: "Alpha", Description: "First **project**",
	}))
}

func TestCheckFormMatchesRust(t *testing.T) {
	got := CheckForm(CheckFormView{
		IsAdmin: true, Heading: "Edit check", Action: "/checks/1", Name: "nightly-backup", Description: "Runs `backup.sh`",
		ScheduleKind: "period", PeriodSecs: "1d", GraceSecs: "1h", Timezone: "UTC", MaxRuntimeSecs: "30m", NagIntervalSecs: "2h",
		Timezones: tz.Names(),
	})
	assertGolden(t, "check-edit", got)
}

func TestDashboardMatchesRust(t *testing.T) {
	assertGolden(t, "dashboard-empty", Dashboard(DashboardView{CSRF: "c"}))
	bars := []view.Bar{{Height: 5, Class: "", Title: "0s / 30m 00s"}}
	for range 3 {
		bars = append(bars, view.Bar{Height: 26, Class: "bad", Title: "Failed"})
	}
	assertGolden(t, "dashboard-down", Dashboard(DashboardView{
		IsAdmin: true, Total: 5, Up: 1, Down: 1, Status: "down",
		Groups: []ProjectGroup{{ID: 1, Name: "Alpha", Description: "First project", Count: 1, Checks: []CheckRow{{
			ID: 1, Name: "nightly-backup", Status: "down", Schedule: "every 1d · 1h grace",
			Last: "2m ago", Bars: bars, Description: "Runs backup.sh",
		}}}},
	}))
}

func TestProjectPageMatchesRust(t *testing.T) {
	check := func(id int64, name, status, schedule, description string, noChannel bool) ProjectCheckRow {
		return ProjectCheckRow{ID: id, Name: name, Status: status, Schedule: schedule, Description: description, NoChannel: noChannel}
	}
	channels := []ProjectChannelRow{}
	for i, kind := range []string{"webhook", "slack", "telegram", "ntfy", "pushover", "email"} {
		channels = append(channels, ProjectChannelRow{ID: int64(i + 1), Name: "ch-" + kind, Kind: kind})
	}
	assertGolden(t, "project", Project(ProjectView{
		IsAdmin: true, Project: models.Project{ID: 1, Name: "Alpha"},
		DescriptionHTML: "<p>First <strong>project</strong></p>",
		Checks: []ProjectCheckRow{
			check(1, "nightly-backup", "down", "every 1d · 1h grace", "Runs backup.sh", false),
			check(2, "weekly-report", "up", "0 0 9 * * Mon * · 10m grace", "", true),
			check(6, "flaky", "paused", "every 2s · 1s grace", "", false),
			check(7, "slow-job", "paused", "every 1h · 1h grace", "", false),
		},
		Channels: channels,
	}))
}

func TestNewCheckPageMatchesRust(t *testing.T) {
	due := time.Date(2026, 11, 17, 15, 31, 6, 949262565, time.UTC)
	now := due.Add(-3*time.Minute - 5*time.Second)
	check := models.Check{ID: 3, ProjectID: 2, Name: "heartbeat", Status: models.StatusNew}
	pings := CheckPings(CheckPingsView{HistoryFilter: HistoryFilter{Base: "", CheckID: 3}})
	notifs := CheckNotifs(CheckNotifsView{HistoryFilter: HistoryFilter{CheckID: 3}})
	got := Check(CheckView{
		IsAdmin: true, Check: check, ProjectName: "Beta", Status: "new", Since: "updated never",
		NextDue:  view.NextDueFor(check, &due, now),
		Schedule: "every 5m · 1m grace", PingURL: "http://localhost:18080/ping/f009852c-3f49-4586-9204-207afa2c1627",
		PingsPartial: pings, NotifsPartial: notifs,
	})
	assertGolden(t, "check-new-state", got)
}

func TestHistoryFragmentsMatchRust(t *testing.T) {
	notifs := CheckNotifs(CheckNotifsView{HistoryFilter: HistoryFilter{CheckID: 3}})
	assertGolden(t, "check-notifications-empty", string(notifs))
	rows := []PingRow{
		{Time: "2026-11-17 15:25:17 UTC", ISO: "2026-11-17T15:25:17.081501953+00:00", PillClass: "log", KindLabel: "log", Exit: "—", Duration: "—", Source: "192.168.215.1", Body: "backup finished ok"},
	}
	got := string(CheckPings(CheckPingsView{HistoryFilter: HistoryFilter{CheckID: 1, Older: "/checks/1/pings?pa=4"}, Rows: rows}))
	for _, want := range []string{
		`<tr class="toggle" data-testid="ping-row"><td class="mono"><span class="caret">▸</span>`,
		`<tr class="exp"><td colspan="5"><div class="out log"><span class="tag">captured output · POST body</span>backup finished ok</div></td></tr>`,
		`<a class="btn" href="/checks/1/pings?pa=4" data-testid="pings-older">Older →</a>`,
		`<span class="btn disabled" aria-disabled="true" data-testid="pings-newer">← Newer</span>`,
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("missing %s in %s", want, got)
		}
	}
}

func TestWithheldPingURLHidesTheCredentialAndItsHelp(t *testing.T) {
	got := Check(CheckView{
		Admin: true, Check: models.Check{ID: 9, ProjectID: 1}, PingURL: "http://x/ping/secret", PingURLHidden: true,
	})
	if strings.Contains(got, "secret") || !strings.Contains(got, `action="/admin/checks/9/ping-url"`) {
		t.Fatal(got)
	}
}
