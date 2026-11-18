package notify

import (
	"bytes"
	"context"
	"log/slog"
	"strings"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/config"
	"github.com/henry40408/pingward/internal/db"
	"github.com/henry40408/pingward/internal/models"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/testutil"
)

func channelWith(kind models.ChannelKind, configJSON string) models.Channel {
	return models.Channel{ID: 1, ProjectID: 1, Kind: kind, Name: "c", ConfigJSON: configJSON, CreatedAt: time.Now()}
}

func requireNotifier(t *testing.T, kind models.ChannelKind, configJSON string, want bool) {
	t.Helper()
	if _, ok := NotifierFor(channelWith(kind, configJSON), nil); ok != want {
		t.Errorf("%s %s: got %v, want %v", kind, configJSON, ok, want)
	}
}

func TestNotifierForBuildsEachKindWithValidConfig(t *testing.T) {
	requireNotifier(t, models.ChannelWebhook, `{"url":"http://x"}`, true)
	requireNotifier(t, models.ChannelSlack, `{"url":"http://x"}`, true)
	requireNotifier(t, models.ChannelTelegram, `{"token":"t","chat_id":"1"}`, true)
	requireNotifier(t, models.ChannelNtfy, `{"base_url":"https://ntfy.sh","topic":"t"}`, true)
	requireNotifier(t, models.ChannelPushover, `{"token":"t","user":"u"}`, true)
}

func TestNotifierForReturnsNoneOnMissingConfig(t *testing.T) {
	requireNotifier(t, models.ChannelSlack, `{}`, false)
	requireNotifier(t, models.ChannelTelegram, `{"token":"t"}`, false)
	requireNotifier(t, models.ChannelNtfy, `{"base_url":"x"}`, false)
	requireNotifier(t, models.ChannelPushover, `{"token":"t"}`, false)
	requireNotifier(t, models.ChannelWebhook, `{"url":"   "}`, false)
	requireNotifier(t, models.ChannelWebhook, `not json`, false)
}

func TestNotifierForEmailNeedsRecipient(t *testing.T) {
	requireNotifier(t, models.ChannelEmail, `{"to":"ops@example.com"}`, true)
	requireNotifier(t, models.ChannelEmail, `{}`, false)
}

// deliveryFixture is a store with a check bound to one webhook channel.
func deliveryFixture(t *testing.T, url string) (*store.Store, int64) {
	t.Helper()
	s := testutil.NewStore(t)
	now := time.Now()
	if _, err := s.DB.ExecContext(bg, "INSERT INTO users (username, is_admin, created_at) VALUES ('u', 0, $1)", db.FormatTime(now)); err != nil {
		t.Fatal(err)
	}
	if _, err := s.DB.ExecContext(bg, "INSERT INTO projects (user_id, name, created_at) VALUES (1, 'p', $1)", db.FormatTime(now)); err != nil {
		t.Fatal(err)
	}
	channelID, err := s.CreateChannel(bg, store.NewChannel{
		ProjectID: 1, Kind: models.ChannelWebhook, Name: "hook", ConfigJSON: `{"url":"` + url + `"}`}, now)
	if err != nil {
		t.Fatal(err)
	}
	checkID, err := s.CreateCheck(bg, store.NewCheck{
		ProjectID: 1, Name: "job", PingUUID: "u1", Kind: models.SchedulePeriod,
		PeriodSecs: ptr(int64(60)), GraceSecs: 30, Timezone: "UTC"}, now)
	if err != nil {
		t.Fatal(err)
	}
	if err := s.BindChannel(bg, checkID, channelID); err != nil {
		t.Fatal(err)
	}
	return s, checkID
}

func TestDeliverEventPostsAndRecordsOK(t *testing.T) {
	server := newSink(t, 200)
	s, checkID := deliveryFixture(t, server.URL)
	ev := Event{CheckID: checkID, CheckName: "job", Kind: models.EventDown, At: time.Now(), ProjectID: 1}
	Deliverer{Store: s, Retry: DefaultRetryPolicy}.DeliverEvent(bg, ev, time.Now())

	server.only(t)
	records, err := s.ListRecentNotifications(bg, checkID, 10)
	if err != nil || len(records) != 1 || records[0].Status != models.NotifyOK || records[0].Error != nil {
		t.Fatalf("%+v %v", records, err)
	}
}

func TestDeliverEventRecordsErrorWhenChannelFails(t *testing.T) {
	server := newSink(t, 500)
	s, checkID := deliveryFixture(t, server.URL)
	ev := Event{CheckID: checkID, CheckName: "job", Kind: models.EventDown, At: time.Now(), ProjectID: 1}
	retry := RetryPolicy{MaxAttempts: 2, BaseBackoff: time.Millisecond}
	Deliverer{Store: s, Retry: retry}.DeliverEvent(bg, ev, time.Now())

	if len(server.requests) != 2 {
		t.Fatalf("attempts %d", len(server.requests))
	}
	records, _ := s.ListRecentNotifications(bg, checkID, 10)
	if len(records) != 1 || records[0].Status != models.NotifyError ||
		records[0].Error == nil || *records[0].Error != "notify failed: status 500 Internal Server Error" {
		t.Fatalf("%+v", records)
	}
}

type countingNotifier struct{ failures, calls int }

func (n *countingNotifier) Send(context.Context, Event) error {
	n.calls++
	if n.calls <= n.failures {
		return &Error{Message: "boom"}
	}
	return nil
}

func TestSendWithRetryBacksOffAndGivesUp(t *testing.T) {
	flaky := &countingNotifier{failures: 2}
	policy := RetryPolicy{MaxAttempts: 3, BaseBackoff: 20 * time.Millisecond}
	start := time.Now()
	if err := SendWithRetry(bg, flaky, Event{}, policy); err != nil || flaky.calls != 3 {
		t.Fatalf("%v after %d calls", err, flaky.calls)
	}
	if elapsed := time.Since(start); elapsed < 60*time.Millisecond { // 20ms + 40ms
		t.Fatalf("backoff too short: %v", elapsed)
	}
	broken := &countingNotifier{failures: 99}
	if err := SendWithRetry(bg, broken, Event{}, policy); err == nil || err.Error() != "notify failed: boom" || broken.calls != 3 {
		t.Fatalf("%v after %d calls", err, broken.calls)
	}
}

func TestBuildEmailSetsHeadersAndBuilds(t *testing.T) {
	message, err := buildEmail("alerts@example.com", "ops@example.com", simpleEvent("backup", models.EventDown))
	if err != nil {
		t.Fatal(err)
	}
	raw := string(message.data)
	for _, want := range []string{"From: <alerts@example.com>", "To: <ops@example.com>", "Subject: pingward: backup is DOWN",
		"Content-Type: text/plain; charset=utf-8", "Content-Transfer-Encoding: quoted-printable", "MIME-Version: 1.0"} {
		requireContains(t, raw, want)
	}
}

func TestBuildEmailKeepsAHostileCheckNameOutOfTheHeaders(t *testing.T) {
	ev := simpleEvent("x\r\nBcc: evil@example.com", models.EventDown)
	message, err := buildEmail("alerts@example.com", "ops@example.com", ev)
	if err != nil {
		t.Fatal(err)
	}
	headers, _, _ := strings.Cut(string(message.data), "\r\n\r\n")
	for _, line := range strings.Split(headers, "\r\n") {
		if strings.HasPrefix(line, "Bcc:") {
			t.Fatalf("injected header %q", line)
		}
	}
}

func TestBuildEmailRejectsBadAddress(t *testing.T) {
	ev := simpleEvent("backup", models.EventDown)
	if _, err := buildEmail("not-an-address", "ops@example.com", ev); err == nil {
		t.Fatal("bad from accepted")
	}
	if _, err := buildEmail("alerts@example.com", "ops@example.com\r\nBcc: x@y.z", ev); err == nil {
		t.Fatal("injected recipient accepted")
	}
}

func TestEmailNotifierErrorsWhenSMTPUnconfigured(t *testing.T) {
	err := NewEmail((*config.SMTPConfig)(nil), "ops@example.com").Send(bg, simpleEvent("backup", models.EventDown))
	if err == nil || err.Error() != "notify failed: instance SMTP not configured" {
		t.Fatalf("%v", err)
	}
}

func TestWebhookPayloadKeepsOldKeysAndAddsContext(t *testing.T) {
	server := newSink(t, 200)
	ev := detailedEvent(models.EventDown, &DownCause{Kind: CauseFailed, ExitCode: ptr(int64(2))})
	if err := NewWebhook(server.URL).Send(bg, ev); err != nil {
		t.Fatal(err)
	}
	body := server.only(t).body
	for _, want := range []string{`"check":"nightly-backup"`, `"event":"down"`, `"project_id":1`, `"project":"infra"`,
		`"url":"https://pingward.example.com/checks/42"`, `"cause":"failed"`, `"exit_code":2`,
		`"last_ping_at":"2026-07-29T09:03:00+00:00"`, `"at":"2026-07-29T10:08:00+00:00"`} {
		requireContains(t, body, want)
	}
}

func TestNtfySetsClickHeaderToTheCheckURL(t *testing.T) {
	server := newSink(t, 200)
	ev := detailedEvent(models.EventDown, &DownCause{Kind: CauseOverdue})
	if err := NewNtfy(server.URL, "topic", nil).Send(bg, ev); err != nil {
		t.Fatal(err)
	}
	if got := server.only(t).header.Get("Click"); got != "https://pingward.example.com/checks/42" {
		t.Fatal(got)
	}
}

func TestNtfySkipsAClickURLThatIsNotHeaderSafe(t *testing.T) {
	for _, unsafe := range []string{"", "https://x.test/a b", "https://x.test/a\nb", "https://x.test/ "} {
		server := newSink(t, 200)
		ev := simpleEvent("backup", models.EventDown)
		ev.Detail.URL = &unsafe
		if err := NewNtfy(server.URL, "topic", nil).Send(bg, ev); err != nil {
			t.Fatalf("%q: %v", unsafe, err)
		}
		if got := server.only(t).header.Get("Click"); got != "" {
			t.Errorf("%q sent as Click %q", unsafe, got)
		}
	}
}

func TestPushoverSendsTheCheckURLAsASupplementaryAction(t *testing.T) {
	server := newSink(t, 200)
	ev := detailedEvent(models.EventDown, &DownCause{Kind: CauseOverdue})
	if err := newPushover("t", "u", server.URL).Send(bg, ev); err != nil {
		t.Fatal(err)
	}
	body := server.only(t).body
	requireContains(t, body, "url_title=Open+in+pingward")
	requireContains(t, body, "priority=1")
}

func TestTransportErrorsAreClassifiedWithoutTheURL(t *testing.T) {
	loop := newLoopingRedirector(t)
	err := NewWebhook(loop.URL+"/secret-token").Send(bg, simpleEvent("x", models.EventDown))
	if err == nil || err.Error() != "notify failed: too many redirects" {
		t.Fatalf("%v", err)
	}
}

type failingStore struct {
	channels    []models.Channel
	channelsErr error
	recordErr   error
	recorded    int
}

func (s *failingStore) ChannelsForCheck(context.Context, int64) ([]models.Channel, error) {
	return s.channels, s.channelsErr
}

func (s *failingStore) RecordNotification(context.Context, store.NewNotification, time.Time) error {
	s.recorded++
	return s.recordErr
}

func TestDeliverEventRecordsNothingWhenChannelsCannotBeLoaded(t *testing.T) {
	failing := &failingStore{channelsErr: context.DeadlineExceeded}
	Deliverer{Store: failing, Retry: DefaultRetryPolicy}.DeliverEvent(bg, Event{CheckID: 1, Kind: models.EventDown}, time.Now())
	if failing.recorded != 0 {
		t.Fatalf("recorded %d notifications", failing.recorded)
	}
}

func TestDeliverEventWithoutChannelsRecordsNothing(t *testing.T) {
	failing := &failingStore{}
	Deliverer{Store: failing, Retry: DefaultRetryPolicy}.DeliverEvent(bg, Event{CheckID: 1, Kind: models.EventDown}, time.Now())
	if failing.recorded != 0 {
		t.Fatalf("recorded %d notifications", failing.recorded)
	}
}

// A failing history write must not stop the remaining channels.
func TestDeliverEventKeepsGoingWhenRecordingFails(t *testing.T) {
	server := newSink(t, 200)
	channel := channelWith(models.ChannelWebhook, `{"url":"`+server.URL+`"}`)
	failing := &failingStore{channels: []models.Channel{channel, channel}, recordErr: context.DeadlineExceeded}
	Deliverer{Store: failing, Retry: DefaultRetryPolicy}.DeliverEvent(bg, Event{CheckID: 1, Kind: models.EventDown, At: time.Now()}, time.Now())
	if len(server.requests) != 2 || failing.recorded != 2 {
		t.Fatalf("requests %d, recorded %d", len(server.requests), failing.recorded)
	}
}

func TestSendWithRetryDoesNotSleepAfterTheFinalFailure(t *testing.T) {
	broken := &countingNotifier{failures: 99}
	policy := RetryPolicy{MaxAttempts: 1, BaseBackoff: 3 * time.Second}
	start := time.Now()
	if err := SendWithRetry(bg, broken, Event{}, policy); err == nil || broken.calls != 1 {
		t.Fatalf("%v after %d calls", err, broken.calls)
	}
	if elapsed := time.Since(start); elapsed > time.Second {
		t.Fatalf("slept after the last attempt: %v", elapsed)
	}
}

func TestSendWithRetryMakesExactlyMaxAttemptsCalls(t *testing.T) {
	for _, attempts := range []int{0, 1, 2, 4} {
		broken := &countingNotifier{failures: 99}
		policy := RetryPolicy{MaxAttempts: attempts, BaseBackoff: time.Millisecond}
		if err := SendWithRetry(bg, broken, Event{}, policy); err == nil {
			t.Fatalf("attempts=%d: want error", attempts)
		}
		if want := max(attempts, 1); broken.calls != want {
			t.Errorf("MaxAttempts %d: %d calls, want %d", attempts, broken.calls, want)
		}
	}
}

type recordingStore struct {
	channels  []models.Channel
	recordErr error
}

func (s recordingStore) ChannelsForCheck(context.Context, int64) ([]models.Channel, error) {
	return s.channels, nil
}

func (s recordingStore) RecordNotification(context.Context, store.NewNotification, time.Time) error {
	return s.recordErr
}

func captureDefaultLog(t *testing.T) *bytes.Buffer {
	t.Helper()
	var out bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewTextHandler(&out, nil)))
	t.Cleanup(func() { slog.SetDefault(previous) })
	return &out
}

func TestDeliverEventLogsARecordFailureOnlyWhenRecordingFails(t *testing.T) {
	server := newSink(t, 200)
	channels := []models.Channel{channelWith(models.ChannelWebhook, `{"url":"`+server.URL+`"}`)}
	ev := simpleEvent("job", models.EventDown)

	out := captureDefaultLog(t)
	Deliverer{Store: recordingStore{channels: channels}, Retry: DefaultRetryPolicy}.DeliverEvent(bg, ev, time.Now())
	if strings.Contains(out.String(), "failed to record notification") {
		t.Fatalf("logged a failure although recording succeeded: %s", out)
	}

	out.Reset()
	failing := recordingStore{channels: channels, recordErr: context.DeadlineExceeded}
	Deliverer{Store: failing, Retry: DefaultRetryPolicy}.DeliverEvent(bg, ev, time.Now())
	if !strings.Contains(out.String(), "failed to record notification") {
		t.Fatalf("a recording failure was not logged: %s", out)
	}
}
