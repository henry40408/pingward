package seed

import (
	"os"
	"strings"
	"testing"
	"time"
)

// The expected values come from the reference JavaScript mulberry32 under Node.
func TestMulberry32MatchesTheJavaScriptSequence(t *testing.T) {
	rand := mulberry32{state: randSeed}
	want := []float64{0.23802160820923746, 0.32467198790982366, 0.29168690275400877, 0.6405401453375816}
	for position, expected := range want {
		if got := rand.next(); got != expected {
			t.Errorf("value %d = %v, want %v", position, got, expected)
		}
	}
}

func TestWeekdaysAreNumberedFromSundayAtOne(t *testing.T) {
	sunday := time.Date(2026, 8, 16, 0, 0, 0, 0, time.UTC).UnixMilli() // a Sunday
	if got := localFields(sunday, time.UTC).dayOfWeek; got != 1 {
		t.Errorf("Sunday = %d, want 1", got)
	}
	if got := localFields(sunday+86_400_000, time.UTC).dayOfWeek; got != 2 {
		t.Errorf("Monday = %d, want 2", got)
	}
}

func TestCronAnchorsLandOnARealFire(t *testing.T) {
	zone, err := time.LoadLocation("Europe/Berlin")
	if err != nil {
		t.Fatal(err)
	}
	const now = 1_800_000_000_000
	const expression = "0 30 2 * * *"
	last, err := lastFireAtOrBefore(expression, zone, now)
	if err != nil || last > now {
		t.Fatalf("last fire = %d, err %v", last, err)
	}
	next, err := nextFireAfter(expression, zone, last)
	if err != nil || next <= last {
		t.Fatalf("next fire = %d, err %v", next, err)
	}
	for _, fire := range []int64{last, next} {
		if matches, _ := cronMatches(expression, zone, fire); !matches {
			t.Errorf("%d is not a fire of %s", fire, expression)
		}
	}
}

func TestTheSeedScriptIsOneTransaction(t *testing.T) {
	sql, err := SQL(1_800_000_000_000)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(sql, "BEGIN;") || !strings.HasSuffix(strings.TrimSpace(sql), "COMMIT;") {
		t.Error("the script is not wrapped in one transaction")
	}
	// Every check contributes 34 runs of two pings each.
	if got := strings.Count(sql, "INSERT INTO pings"); got < len(checks)*68 {
		t.Errorf("%d ping rows, want at least %d", got, len(checks)*68)
	}
}

func TestSQLMatchesTheRustCapture(t *testing.T) {
	want, err := os.ReadFile("testdata/seed_1800000000000.sql")
	if err != nil {
		t.Fatal(err)
	}
	got, err := SQL(1_800_000_000_000)
	if err != nil {
		t.Fatal(err)
	}
	if got != string(want) {
		t.Error("the seed script differs from the Rust capture")
	}
}
