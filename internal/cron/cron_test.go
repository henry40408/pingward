package cron

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"strings"
	"testing"
	"time"
)

// rustCapturePath is the file the Rust capture lived in before it was deleted
// from the tree (see rust_fixtures_test.go).
const rustCapturePath = "internal/cron/testdata/fixtures.json"

// logCaptureHint prints, for a failed test, where the original capture is.
func logCaptureHint(t *testing.T) {
	if !t.Failed() {
		return
	}
	t.Logf("The Rust capture is in git history: git show $(git log --diff-filter=D --format=%%h -1 -- %s)^:%s\n"+
		"A rustValid digest covers one line per case of the capture, in order: \"<anchor> <zone> <next joined by comma>\".",
		rustCapturePath, rustCapturePath)
}

// assertRustDigest fails when got does not hash to want. It writes got to a
// temporary file, so it can be compared with the capture.
func assertRustDigest(t *testing.T, name, want, got string) {
	t.Helper()
	sum := sha256.Sum256([]byte(got))
	if hex.EncodeToString(sum[:]) == want {
		return
	}
	file, err := os.CreateTemp("", "cron-actual-*.txt")
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = file.Close() }()
	if _, err := file.WriteString(got); err != nil {
		t.Fatal(err)
	}
	t.Errorf("%s differs from the Rust capture (%d bytes of output written to %s)", name, len(got), file.Name())
}

// upcomingText lists the next five fire times for every anchor in every zone
// of rustZones, in the line format the rustValid digests were computed over.
func upcomingText(t *testing.T, schedule *Schedule, anchors []string) string {
	t.Helper()
	var text strings.Builder
	for _, anchorText := range anchors {
		anchor, err := time.Parse(time.RFC3339Nano, anchorText)
		if err != nil {
			t.Fatal(err)
		}
		for _, zoneName := range rustZones {
			zone, err := time.LoadLocation(zoneName)
			if err != nil {
				t.Fatal(err)
			}
			var fires []string
			for _, fire := range schedule.Upcoming(anchor, zone, 5) {
				fires = append(fires, fire.UTC().Format("2006-01-02T15:04:05.000000000Z"))
			}
			fmt.Fprintf(&text, "%s %s %s\n", anchorText, zoneName, strings.Join(fires, ","))
		}
	}
	return text.String()
}

func TestInvalidExpressionsReportTheRustErrorText(t *testing.T) {
	defer logCaptureHint(t)
	if len(rustInvalid) != 154 {
		t.Fatalf("%d invalid fixtures, want 154", len(rustInvalid))
	}
	for _, c := range rustInvalid {
		_, err := Parse(c.expr)
		if err == nil {
			t.Errorf("%q parsed, Rust refused it with %q", c.expr, c.error)
			continue
		}
		if err.Error() != c.error {
			t.Errorf("%q:\n got %q\nwant %q", c.expr, err.Error(), c.error)
		}
	}
}

func TestValidExpressionsFireLikeTheRustCrate(t *testing.T) {
	defer logCaptureHint(t)
	if len(rustValid) != 150 {
		t.Fatalf("%d valid fixtures, want 150", len(rustValid))
	}
	for i, expression := range rustValid {
		schedule, err := Parse(expression.expr)
		if err != nil {
			t.Errorf("%q: %v", expression.expr, err)
			continue
		}
		// The index tells apart the expression the capture lists twice.
		name := fmt.Sprintf("rustValid[%d] %q", i, expression.expr)
		assertRustDigest(t, name, expression.digest, upcomingText(t, schedule, expression.anchors))
	}
}

func equalStrings(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

func TestNextFireTimes(t *testing.T) {
	paris, err := time.LoadLocation("Europe/Paris")
	if err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name   string
		expr   string
		anchor time.Time
		zone   *time.Location
		want   []string
	}{
		{"leap day in a century divisible by 400", "0 0 0 29 2 *", time.Date(2000, 1, 1, 0, 0, 0, 0, time.UTC), time.UTC,
			[]string{"2000-02-29T00:00:00Z"}},
		{"weekdays only", "0 0 0 * * MON-FRI", time.Date(2026, 1, 3, 12, 0, 0, 0, time.UTC), time.UTC,
			[]string{"2026-01-05T00:00:00Z"}},
		{"a single weekday as a named range", "0 0 0 * * MON-MON", time.Date(2026, 1, 3, 12, 0, 0, 0, time.UTC), time.UTC,
			[]string{"2026-01-05T00:00:00Z"}},
		// 02:30 happens twice when Paris leaves summer time; the first pass is
		// only found by probing the offset a day earlier.
		{"ambiguous local time east of UTC", "0 30 2 * * *", time.Date(2026, 10, 24, 12, 0, 0, 0, time.UTC), paris,
			[]string{"2026-10-25T00:30:00Z", "2026-10-25T01:30:00Z"}},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			schedule, err := Parse(c.expr)
			if err != nil {
				t.Fatal(err)
			}
			var got []string
			for _, fire := range schedule.Upcoming(c.anchor, c.zone, len(c.want)) {
				got = append(got, fire.UTC().Format("2006-01-02T15:04:05Z"))
			}
			if !equalStrings(got, c.want) {
				t.Errorf("got %v, want %v", got, c.want)
			}
		})
	}
}

func TestLettersAtTheEndOfTheAlphabetAreNames(t *testing.T) {
	for _, name := range []string{"z", "Z"} {
		_, err := Parse("0 0 0 * * " + name)
		want := "'" + name + "' is not a valid day of the week."
		if err == nil || !strings.Contains(err.Error(), want) {
			t.Errorf("%q: got %v, want it to contain %q", name, err, want)
		}
	}
}
