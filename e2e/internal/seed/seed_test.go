package seed

import (
	"crypto/sha256"
	"encoding/hex"
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

// rustSeedDigest is the sha256 of the Rust e2e/src/seed.rs seed_sql(1_800_000_000_000)
// output, captured once in a container before the Rust code was removed. The Go
// port must reproduce it byte for byte, which pins the Mulberry32 draw order and
// the float arithmetic. Do not recompute it to make a test pass.
const rustSeedDigest = "38a30753c3e12f6ad0f06c9d55717da8c91ca67554f44a2277ca091d2c78b41c"

func TestSQLMatchesTheRustCapture(t *testing.T) {
	got, err := SQL(1_800_000_000_000)
	if err != nil {
		t.Fatal(err)
	}
	sum := sha256.Sum256([]byte(got))
	if hex.EncodeToString(sum[:]) == rustSeedDigest {
		return
	}
	file, err := os.CreateTemp("", "seed.actual.*.sql")
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = file.Close() }()
	if _, err := file.WriteString(got); err != nil {
		t.Fatal(err)
	}
	const capturePath = "e2e/internal/seed/testdata/seed_1800000000000.sql"
	t.Errorf("the seed script differs from the Rust capture (%d bytes written to %s).\n"+
		"The capture is in git history: git show $(git log --diff-filter=D --format=%%h -1 -- %s)^:%s",
		len(got), file.Name(), capturePath, capturePath)
}
