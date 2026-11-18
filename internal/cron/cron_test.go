package cron

import (
	"encoding/json"
	"os"
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
