package elevate

import (
	"testing"
	"time"
)

var t0 = time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)

func elevated(e *Elevations, handle string, now time.Time) bool {
	_, ok := e.RemainingSecs(handle, now)
	return ok
}

func TestAGrantedHandleIsElevatedAndOthersAreNot(t *testing.T) {
	e := New(TTL)
	if elevated(e, "abc", t0) {
		t.Fatal("elevated before grant")
	}
	e.Grant("abc", 1, t0)
	secs, ok := e.RemainingSecs("abc", t0)
	if !ok || secs != 900 {
		t.Fatalf("remaining = %d, %v", secs, ok)
	}
	if elevated(e, "def", t0) {
		t.Fatal("another handle is elevated")
	}
	if secs, _ := e.RemainingSecs("abc", t0.Add(100*time.Second)); secs != 800 {
		t.Fatalf("remaining after 100s = %d", secs)
	}
	if elevated(e, "abc", t0.Add(TTL)) {
		t.Fatal("must expire at exactly the TTL")
	}
}

func TestAZeroTTLNeverCountsAsElevated(t *testing.T) {
	e := New(0)
	e.Grant("abc", 1, t0)
	if elevated(e, "abc", t0) {
		t.Fatal("zero TTL elevated")
	}
}

func TestRevokeEndsTheWindowImmediately(t *testing.T) {
	e := New(TTL)
	e.Grant("abc", 1, t0)
	e.Revoke("abc")
	if elevated(e, "abc", t0) {
		t.Fatal("still elevated")
	}
	e.Revoke("never-seen")
}

func TestRevokeUserSparesOnlyTheKeptHandleAndOtherUsers(t *testing.T) {
	e := New(TTL)
	e.Grant("a1", 1, t0)
	e.Grant("a2", 1, t0)
	e.Grant("b1", 2, t0)
	e.RevokeUser(1, "a2")
	if elevated(e, "a1", t0) || !elevated(e, "a2", t0) || !elevated(e, "b1", t0) {
		t.Fatal("wrong survivors after keep")
	}
	e.RevokeUser(1, "")
	if elevated(e, "a2", t0) || !elevated(e, "b1", t0) {
		t.Fatal("wrong survivors after revoke all")
	}
}

func TestGrantingPrunesExpiredEntries(t *testing.T) {
	e := New(10 * time.Second)
	e.Grant("stale", 1, t0)
	e.Grant("fresh", 1, t0.Add(20*time.Second))
	if _, kept := e.granted["stale"]; kept {
		t.Fatal("stale entry kept")
	}
}

func TestASecondGrantRestartsTheWindow(t *testing.T) {
	e := New(TTL)
	e.Grant("abc", 1, t0)
	e.Grant("abc", 1, t0.Add(500*time.Second))
	if secs, _ := e.RemainingSecs("abc", t0.Add(500*time.Second)); secs != 900 {
		t.Fatalf("remaining = %d", secs)
	}
}
