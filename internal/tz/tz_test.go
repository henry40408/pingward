package tz

import (
	"encoding/json"
	"os"
	"testing"
)

func TestNamesMatchTheChronoTzList(t *testing.T) {
	if len(Names()) != 597 {
		t.Fatalf("got %d names, chrono-tz 0.10.4 has 597", len(Names()))
	}
	if Names()[0] != "Africa/Abidjan" {
		t.Fatalf("first name = %q", Names()[0])
	}
}

func TestEveryNameLoadsFromEmbeddedTzdata(t *testing.T) {
	for _, name := range Names() {
		if _, err := Load(name); err != nil {
			t.Errorf("%s: %v", name, err)
		}
	}
}

func TestCanonicalAgreesWithChronoTzParse(t *testing.T) {
	raw, err := os.ReadFile("testdata/chrono_tz_parse.json")
	if err != nil {
		t.Fatal(err)
	}
	var probes []struct {
		Input string  `json:"input"`
		Name  *string `json:"name"`
	}
	if err := json.Unmarshal(raw, &probes); err != nil {
		t.Fatal(err)
	}
	for _, probe := range probes {
		got, ok := Canonical(probe.Input)
		switch {
		case probe.Name == nil && ok:
			t.Errorf("%q accepted, chrono-tz refuses it", probe.Input)
		case probe.Name != nil && (!ok || got != *probe.Name):
			t.Errorf("%q = %q,%v want %q", probe.Input, got, ok, *probe.Name)
		}
	}
}

func TestLoadRefusesWhatChronoTzRefuses(t *testing.T) {
	for _, name := range []string{"utc", "asia/taipei", "Local", "", " UTC", "Factory"} {
		if _, err := Load(name); err == nil {
			t.Errorf("Load(%q) succeeded", name)
		}
	}
}
