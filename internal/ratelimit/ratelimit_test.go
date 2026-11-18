package ratelimit

import (
	"fmt"
	"strings"
	"sync"
	"testing"
	"time"
)

var t0 = time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)

func ip(last int) string { return fmt.Sprintf("203.0.113.%d", last) }

func newLimiter(max uint32, window time.Duration) *Limiter[string] {
	return New[string](max, window, t0)
}

func (l *Limiter[K]) tracked() int {
	l.mu.Lock()
	defer l.mu.Unlock()
	return len(l.perKey)
}

func TestAllowsUpToMaxAttempts(t *testing.T) {
	l := newLimiter(5, time.Minute)
	for range 5 {
		if !l.Charge(ip(1), t0) {
			t.Fatal("attempt within the limit refused")
		}
	}
	if l.Charge(ip(1), t0) {
		t.Fatal("attempt over the limit allowed")
	}
}

func TestWindowExpiryResetsTheCounter(t *testing.T) {
	l := newLimiter(1, 0) // a zero window is elapsed the moment it is recorded
	for range 2 {
		if !l.Charge(ip(2), t0) {
			t.Fatal("zero window must never refuse")
		}
	}
	m := newLimiter(1, time.Minute)
	m.Charge(ip(2), t0)
	if m.Charge(ip(2), t0.Add(59*time.Second)) {
		t.Fatal("still inside the window")
	}
	if !m.Charge(ip(2), t0.Add(60*time.Second)) {
		t.Fatal("window must reset at exactly its length")
	}
}

func TestDistinctKeysHaveIndependentBuckets(t *testing.T) {
	l := newLimiter(1, time.Minute)
	if !l.Charge(ip(3), t0) || l.Charge(ip(3), t0) || !l.Charge(ip(4), t0) {
		t.Fatal("buckets are not independent")
	}
}

func TestReleaseReturnsTheReservedAttempt(t *testing.T) {
	l := newLimiter(2, time.Minute)
	for range 10 {
		if !l.Charge(ip(5), t0) {
			t.Fatal("charge refused")
		}
		l.Release(ip(5))
	}
	if l.tracked() != 0 {
		t.Fatal("released bucket must be removed")
	}
}

func TestClearEmptiesTheBucketWhereReleaseRefundsOne(t *testing.T) {
	refunded := newLimiter(10, time.Minute)
	cleared := newLimiter(10, time.Minute)
	for range 9 {
		refunded.Charge("alice", t0)
		cleared.Charge("alice", t0)
	}
	if !refunded.Charge("alice", t0) || !cleared.Charge("alice", t0) {
		t.Fatal("tenth attempt must be allowed")
	}
	refunded.Release("alice")
	cleared.Clear("alice")

	if !refunded.Charge("alice", t0) || refunded.Charge("alice", t0) {
		t.Fatal("refunded: exactly one attempt must be left")
	}
	for range 10 {
		if !cleared.Charge("alice", t0) {
			t.Fatal("cleared: whole window must be available")
		}
	}
	if cleared.Charge("alice", t0) {
		t.Fatal("cleared: eleventh attempt must be refused")
	}
}

func TestClearOnAnUntrackedKeyIsANoOp(t *testing.T) {
	l := newLimiter(2, time.Minute)
	l.Clear("never-seen")
	if !l.Charge("never-seen", t0) {
		t.Fatal("charge refused")
	}
}

func TestAccountKeyBoundsLengthWithoutTouchingCase(t *testing.T) {
	if AccountKey("Alice") != "Alice" || AccountKey("Alice") == AccountKey("alice") {
		t.Fatal("case must be preserved")
	}
	if AccountKey("  bob  ") != "  bob  " {
		t.Fatal("whitespace must be preserved")
	}
	for _, unit := range []string{"x", "漢"} {
		key := AccountKey(strings.Repeat(unit, 10_000))
		if got := len([]rune(key)); got != accountKeyMaxChars {
			t.Fatalf("%q: kept %d chars", unit, got)
		}
	}
	if AccountKey(strings.Repeat("x", 64)) != strings.Repeat("x", 64) {
		t.Fatal("a key of exactly 64 chars is kept whole")
	}
}

func TestMapIsPrunedAtCapacity(t *testing.T) {
	l := newLimiter(5, time.Minute)
	l.maxEntries = 4
	for n := range 50 {
		l.Charge(ip(n), t0)
	}
	if l.tracked() > 4 {
		t.Fatalf("tracked %d keys over the cap", l.tracked())
	}
}

func TestExpiredEntriesArePrunedWhenCapacityIsReached(t *testing.T) {
	l := newLimiter(5, time.Minute)
	l.maxEntries = 4
	for n := range 4 {
		l.Charge(ip(n), t0)
	}
	later := t0.Add(2 * time.Minute)
	if !l.Charge(ip(99), later) {
		t.Fatal("fresh key refused")
	}
	if l.tracked() >= 4 {
		t.Fatalf("map was not pruned: len = %d", l.tracked())
	}
}

func TestCapacitySprayDoesNotResetAnExistingCounter(t *testing.T) {
	l := newLimiter(1, time.Minute)
	l.maxEntries = 4
	l.Charge(ip(200), t0)
	if l.Charge(ip(200), t0) {
		t.Fatal("second attempt must be refused")
	}
	for n := range 50 {
		l.Charge(ip(n), t0)
	}
	if l.Charge(ip(200), t0) {
		t.Fatal("spray reset the counter")
	}
}

func TestConcurrentAttemptsCannotExceedTheLimit(t *testing.T) {
	const goroutines = 64
	l := newLimiter(5, time.Minute)
	start := make(chan struct{})
	var wg sync.WaitGroup
	var mu sync.Mutex
	allowed := 0
	for range goroutines {
		wg.Add(1)
		go func() {
			defer wg.Done()
			<-start
			if l.Charge(ip(7), t0) {
				mu.Lock()
				allowed++
				mu.Unlock()
			}
		}()
	}
	close(start)
	wg.Wait()
	if allowed != 5 {
		t.Fatalf("allowed %d, want 5", allowed)
	}
}

func TestTheOverflowBucketIsFinite(t *testing.T) {
	const max = 2
	l := newLimiter(max, time.Minute)
	l.maxEntries = 4
	for n := range 4 {
		if !l.Charge(ip(n), t0) {
			t.Fatal("charge refused")
		}
	}
	for n := range max * overflowFactor {
		if !l.Charge(ip(100+n), t0) {
			t.Fatalf("attempt %d is inside the shared budget", n)
		}
	}
	if l.Charge(ip(200), t0) {
		t.Fatal("the shared overflow budget must run out")
	}
	// Not a global lockout: an address with its own bucket still works.
	l.Release(ip(0))
	if !l.Charge(ip(0), t0) {
		t.Fatal("an address with its own bucket must still work")
	}
}

func TestTheOverflowBucketRefillsWithItsWindow(t *testing.T) {
	l := newLimiter(1, 0)
	l.maxEntries = 0
	for range overflowFactor * 3 {
		if !l.Charge(ip(1), t0) {
			t.Fatal("zero window must never refuse")
		}
	}
}

func TestReleaseRefundsTheOverflowBucket(t *testing.T) {
	l := newLimiter(1, time.Minute)
	l.maxEntries = 4
	for n := range 4 {
		l.Charge(ip(n), t0)
	}
	for range overflowFactor * 3 {
		if !l.Charge(ip(150), t0) {
			t.Fatal("charge refused")
		}
		l.Release(ip(150))
	}
	if l.overflow.count != 0 {
		t.Fatalf("overflow count = %d", l.overflow.count)
	}
}

func TestReleaseSaturatesAtZero(t *testing.T) {
	l := newLimiter(1, time.Minute)
	l.Release("unknown")
	if l.overflow.count != 0 {
		t.Fatal("overflow count must not underflow")
	}
}

func TestAStringKeyedLimiterCountsPerKey(t *testing.T) {
	l := newLimiter(2, time.Minute)
	for attempt, want := range []bool{true, true, false} {
		if got := l.Charge("alice", t0); got != want {
			t.Fatalf("alice attempt %d: allowed=%v", attempt+1, got)
		}
	}
	if !l.Charge("bob", t0) {
		t.Fatal("a lockout is never global")
	}
}

func TestAnEntryExpiresExactlyWhenItsWindowEnds(t *testing.T) {
	l := newLimiter(5, time.Minute)
	l.maxEntries = 2
	l.Charge(ip(1), t0)
	l.Charge(ip(2), t0)
	l.Charge(ip(3), t0.Add(time.Minute))
	if l.tracked() != 1 {
		t.Fatalf("both entries are a full window old and must be pruned: len = %d", l.tracked())
	}
}

func TestReleaseDropsATrackedKeyWhoseCountIsAlreadyZero(t *testing.T) {
	l := newLimiter(5, time.Minute)
	l.perKey["k"] = &counter{started: t0}
	l.Release("k")
	if l.tracked() != 0 {
		t.Fatalf("an empty bucket must be dropped, not wrapped around: %+v", l.perKey["k"])
	}
}
