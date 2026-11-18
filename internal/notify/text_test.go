package notify

import (
	"strings"
	"testing"
	"time"
	"unicode"

	"github.com/henry40408/pingward/internal/models"
)

func ptr[T any](v T) *T { return &v }

func utc(year int, month time.Month, day, hour, minute int) time.Time {
	return time.Date(year, month, day, hour, minute, 0, 0, time.UTC)
}

func detailCheck() models.Check {
	return models.Check{
		ID: 42, ProjectID: 1, Name: "nightly-backup", PingUUID: "u", ScheduleKind: models.SchedulePeriod,
		PeriodSecs: ptr(int64(300)), GraceSecs: 60, Timezone: "Asia/Taipei", Status: models.StatusUp,
		LastPingAt: ptr(utc(2026, 7, 29, 9, 3)), CreatedAt: utc(2026, 1, 1, 0, 0),
	}
}

func detailedEvent(kind models.EventKind, cause *DownCause) Event {
	check := detailCheck()
	detail := DetailFromCheck(check, ptr("infra"), "https://pingward.example.com/")
	detail.Cause = cause
	return Event{CheckID: check.ID, CheckName: check.Name, Kind: kind, At: utc(2026, 7, 29, 10, 8), ProjectID: check.ProjectID, Detail: detail}
}

func requireContains(t *testing.T, text, want string) {
	t.Helper()
	if !strings.Contains(text, want) {
		t.Fatalf("missing %q in:\n%s", want, text)
	}
}

func requireAbsent(t *testing.T, text, unwanted string) {
	t.Helper()
	if strings.Contains(text, unwanted) {
		t.Fatalf("unexpected %q in:\n%s", unwanted, text)
	}
}

func TestReminderEventRoundtripsAndRendersStillDown(t *testing.T) {
	if models.EventReminder != "reminder" {
		t.Fatal("reminder name")
	}
	if parsed, err := models.ParseEventKind("reminder"); err != nil || parsed != models.EventReminder {
		t.Fatalf("%v %v", parsed, err)
	}
	ev := simpleEvent("backup", models.EventReminder)
	requireContains(t, EventText(ev), "STILL DOWN")
	if got := EventTitle(ev); got != "pingward: backup is STILL DOWN" {
		t.Fatal(got)
	}
}

func TestTestEventRoundtripsAndRenders(t *testing.T) {
	if parsed, err := models.ParseEventKind("test"); err != nil || parsed != models.EventTest {
		t.Fatalf("%v %v", parsed, err)
	}
	ev := simpleEvent("my-slack", models.EventTest)
	requireContains(t, EventText(ev), "test notification")
	requireContains(t, EventText(ev), "my-slack")
	if got := EventTitle(ev); got != `pingward: test notification for "my-slack"` {
		t.Fatal(got)
	}
}

func TestDownTextNamesProjectScheduleReasonAndLink(t *testing.T) {
	got := EventText(detailedEvent(models.EventDown, &DownCause{Kind: CauseOverdue}))
	want := "\U0001F534 DOWN — nightly-backup\n" +
		"Project: infra · every 5m (grace 1m)\n" +
		"No ping since 2026-07-29 17:03 CST (1h5m ago)\n" +
		"https://pingward.example.com/checks/42"
	if got != want {
		t.Fatalf("got:\n%s\nwant:\n%s", got, want)
	}
}

func TestDownTextDistinguishesFailPingFromAMissedCheckIn(t *testing.T) {
	text := EventText(detailedEvent(models.EventDown, &DownCause{Kind: CauseFailed, ExitCode: ptr(int64(1))}))
	requireContains(t, text, "Fail ping received (exit code 1)")
	requireAbsent(t, text, "No ping since")
}

func TestDownTextReportsAnOverrunRunWithItsBudget(t *testing.T) {
	text := EventText(detailedEvent(models.EventDown, &DownCause{
		Kind: CauseOverrun, MaxRuntimeSecs: 600, StartedAt: utc(2026, 7, 29, 9, 50),
	}))
	requireContains(t, text, "Run started 2026-07-29 17:50 CST (18m ago) and exceeded its 10m max runtime")
}

func TestUpTextReportsThePreviousPingNotACause(t *testing.T) {
	text := EventText(detailedEvent(models.EventUp, nil))
	if !strings.HasPrefix(text, "\U0001F7E2 UP — nightly-backup") {
		t.Fatal(text)
	}
	requireContains(t, text, "Recovered; previous ping 2026-07-29 17:03 CST (1h5m ago)")
}

func TestReminderTextReportsTheLastPingNeutrally(t *testing.T) {
	text := EventText(detailedEvent(models.EventReminder, nil))
	if !strings.HasPrefix(text, "\U0001F534 STILL DOWN — nightly-backup") {
		t.Fatal(text)
	}
	requireContains(t, text, "Last ping 2026-07-29 17:03 CST")
	requireAbsent(t, text, "No ping since")
}

func TestTextDegradesToHeadlineAndReasonWithoutDetail(t *testing.T) {
	ev := simpleEvent("backup", models.EventDown)
	if got := EventText(ev); got != "\U0001F534 DOWN — backup\nNo ping received yet" {
		t.Fatal(got)
	}
	if got := EventTitle(ev); got != "pingward: backup is DOWN" {
		t.Fatal(got)
	}
}

func TestTitleIncludesTheProjectAndStaysSingleLine(t *testing.T) {
	ev := detailedEvent(models.EventDown, &DownCause{Kind: CauseOverdue})
	ev.CheckName = "nightly\nbackup"
	ev.Detail.ProjectName = ptr("in\tfra")
	title := EventTitle(ev)
	if title != "pingward: in fra/nightly backup is DOWN" {
		t.Fatal(title)
	}
	if strings.ContainsFunc(title, unicode.IsControl) {
		t.Fatal("control character in title")
	}
}

func TestDisplayTimezoneOverridesTheChecksOwnZone(t *testing.T) {
	ev := detailedEvent(models.EventDown, &DownCause{Kind: CauseOverdue})
	requireContains(t, EventText(ev), "17:03 CST")
	ev.Detail = ev.Detail.WithDisplayTimezone(ptr("Europe/Berlin"))
	text := EventText(ev)
	requireContains(t, text, "11:03 CEST")
	requireAbsent(t, text, "CST")
}

func TestABlankDisplayTimezoneKeepsTheChecksZone(t *testing.T) {
	ev := detailedEvent(models.EventDown, &DownCause{Kind: CauseOverdue})
	for _, blank := range []*string{nil, ptr(""), ptr("   ")} {
		detail := ev.Detail.WithDisplayTimezone(blank)
		if detail.Timezone == nil || *detail.Timezone != "Asia/Taipei" {
			t.Fatalf("blank %v changed the zone to %v", blank, detail.Timezone)
		}
	}
}

func TestNoBaseURLMeansNoLink(t *testing.T) {
	check := detailCheck()
	if detail := DetailFromCheck(check, nil, ""); detail.URL != nil {
		t.Fatalf("%v", *detail.URL)
	}
	if detail := DetailFromCheck(check, nil, "https://x.test"); detail.URL == nil || *detail.URL != "https://x.test/checks/42" {
		t.Fatalf("%v", detail.URL)
	}
}

func TestScheduleSummaryRendersBothKinds(t *testing.T) {
	check := detailCheck()
	if got := ScheduleSummary(check); got != "every 5m (grace 1m)" {
		t.Fatal(got)
	}
	check.GraceSecs = 0
	if got := ScheduleSummary(check); got != "every 5m" {
		t.Fatal(got)
	}
	check.ScheduleKind = models.ScheduleCron
	check.CronExpr = ptr("0 0 * * * *")
	if got := ScheduleSummary(check); got != `cron "0 0 * * * *" Asia/Taipei` {
		t.Fatal(got)
	}
}

func TestAnUnknownTimezoneRendersInUTC(t *testing.T) {
	for _, name := range []string{"Mars/Base", "Local", ""} {
		if got := fmtAt(utc(2026, 7, 29, 9, 3), &name); got != "2026-07-29 09:03 UTC" {
			t.Errorf("%q: %s", name, got)
		}
	}
}
