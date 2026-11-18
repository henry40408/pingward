package models

import "testing"

func TestStatusRoundTripsThroughText(t *testing.T) {
	for _, status := range []CheckStatus{StatusNew, StatusUp, StatusDown, StatusPaused} {
		got, err := ParseCheckStatus(string(status))
		if err != nil || got != status {
			t.Errorf("%s: %v %v", status, got, err)
		}
	}
}

func TestUnknownStatusIsAnError(t *testing.T) {
	if _, err := ParseCheckStatus("bogus"); err == nil {
		t.Fatal("bogus status accepted")
	}
}

func TestChannelKindRoundTrips(t *testing.T) {
	kinds := []ChannelKind{ChannelWebhook, ChannelTelegram, ChannelSlack, ChannelNtfy, ChannelPushover, ChannelEmail}
	for _, kind := range kinds {
		got, err := ParseChannelKind(string(kind))
		if err != nil || got != kind {
			t.Errorf("%s: %v %v", kind, got, err)
		}
	}
	if _, err := ParseChannelKind("carrier-pigeon"); err == nil {
		t.Fatal("unknown kind accepted")
	}
}

func TestNotifyStatusRoundTrips(t *testing.T) {
	for _, status := range []NotifyStatus{NotifyOK, NotifyError} {
		if got, err := ParseNotifyStatus(string(status)); err != nil || got != status {
			t.Errorf("%s: %v %v", status, got, err)
		}
	}
}

func TestEveryEnumRefusesTheEmptyString(t *testing.T) {
	if _, err := ParseScheduleKind(""); err == nil {
		t.Error("schedule kind")
	}
	if _, err := ParsePingKind(""); err == nil {
		t.Error("ping kind")
	}
	if _, err := ParseEventKind("Down"); err == nil {
		t.Error("event kind is case-sensitive")
	}
	if got, err := ParseEventKind("test"); err != nil || got != EventTest {
		t.Errorf("event kind: %v %v", got, err)
	}
}

func TestPingSummaryOmitsTheBody(t *testing.T) {
	summary := Ping{ID: 1, CheckID: 2, Kind: PingFail, Body: "large"}.Summary()
	if summary.ID != 1 || summary.CheckID != 2 || summary.Kind != PingFail {
		t.Fatalf("%+v", summary)
	}
}
