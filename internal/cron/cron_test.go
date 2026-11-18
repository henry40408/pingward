package cron

import (
	"encoding/json"
	"os"
	"strings"
	"testing"
	"time"
)

type fixtureCase struct {
	Anchor string   `json:"anchor"`
	Zone   string   `json:"zone"`
	Next   []string `json:"next"`
}

type fixtureFile struct {
	Valid []struct {
		Expr  string        `json:"expr"`
		Cases []fixtureCase `json:"cases"`
	} `json:"valid"`
	Invalid []struct {
		Expr  string `json:"expr"`
		Error string `json:"error"`
	} `json:"invalid"`
}

func loadFixtures(t *testing.T) fixtureFile {
	t.Helper()
	raw, err := os.ReadFile("testdata/fixtures.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixtures fixtureFile
	if err := json.Unmarshal(raw, &fixtures); err != nil {
		t.Fatal(err)
	}
	return fixtures
}

func TestInvalidExpressionsReportTheRustErrorText(t *testing.T) {
	fixtures := loadFixtures(t)
	if len(fixtures.Invalid) < 100 {
		t.Fatalf("only %d invalid fixtures", len(fixtures.Invalid))
	}
	for _, c := range fixtures.Invalid {
		_, err := Parse(c.Expr)
		if err == nil {
			t.Errorf("%q parsed, Rust refused it with %q", c.Expr, c.Error)
			continue
		}
		if err.Error() != c.Error {
			t.Errorf("%q:\n got %q\nwant %q", c.Expr, err.Error(), c.Error)
		}
	}
}

func TestValidExpressionsFireLikeTheRustCrate(t *testing.T) {
	fixtures := loadFixtures(t)
	if len(fixtures.Valid) < 100 {
		t.Fatalf("only %d valid fixtures", len(fixtures.Valid))
	}
	for _, expression := range fixtures.Valid {
		schedule, err := Parse(expression.Expr)
		if err != nil {
			t.Errorf("%q: %v", expression.Expr, err)
			continue
		}
		for _, c := range expression.Cases {
			anchor, err := time.Parse(time.RFC3339Nano, c.Anchor)
			if err != nil {
				t.Fatal(err)
			}
			zone, err := time.LoadLocation(c.Zone)
			if err != nil {
				t.Fatal(err)
			}
			var got []string
			for _, fire := range schedule.Upcoming(anchor, zone, 5) {
				got = append(got, fire.UTC().Format("2006-01-02T15:04:05.000000000Z"))
			}
			if !equalStrings(got, c.Next) {
				t.Errorf("%q anchor %s zone %s:\n got %v\nwant %v", expression.Expr, c.Anchor, c.Zone, got, c.Next)
			}
		}
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
