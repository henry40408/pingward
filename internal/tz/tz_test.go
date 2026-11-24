package tz

import "testing"

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

// chronoTzParseProbes were captured from chrono-tz 0.10.4 `str::parse::<Tz>()`:
// want is the canonical name, or "" where chrono-tz refuses the input.
var chronoTzParseProbes = []struct{ input, want string }{
	{"UTC", "UTC"},
	{"utc", ""},
	{"Utc", ""},
	{"UCT", "UCT"},
	{"Asia/Taipei", "Asia/Taipei"},
	{"asia/taipei", ""},
	{"ASIA/TAIPEI", ""},
	{"America/New_York", "America/New_York"},
	{"america/new_york", ""},
	{"Etc/GMT+5", "Etc/GMT+5"},
	{"etc/gmt+5", ""},
	{"GMT", "GMT"},
	{"gmt", ""},
	{"Zulu", "Zulu"},
	{"zulu", ""},
	{"EST", "EST"},
	{"est", ""},
	{"EST5EDT", "EST5EDT"},
	{"Local", ""},
	{"local", ""},
	{"Not/AZone", ""},
	{"", ""},
	{" UTC", ""},
	{"UTC ", ""},
	{"Asia/Taipei ", ""},
	{"Taipei", ""},
	{"Asia/Kolkata", "Asia/Kolkata"},
	{"Asia/Calcutta", "Asia/Calcutta"},
	{"US/Eastern", "US/Eastern"},
	{"us/eastern", ""},
	{"Europe/Kyiv", "Europe/Kyiv"},
	{"Europe/Kiev", "Europe/Kiev"},
	{"America/Argentina/Buenos_Aires", "America/Argentina/Buenos_Aires"},
	{"america/argentina/buenos_aires", ""},
	{"UTC+8", ""},
	{"+08:00", ""},
	{"Asia//Taipei", ""},
	{"/UTC", ""},
	{"UTC/", ""},
	{"../UTC", ""},
	{"posixrules", ""},
	{"right/UTC", ""},
	{"Factory", ""},
	{"localtime", ""},
	{"Asia/Taipei\n", ""},
	{"Asia/Tapei", ""},
	{"Etc/UTC", "Etc/UTC"},
	{"Etc/Greenwich", "Etc/Greenwich"},
	{"Greenwich", "Greenwich"},
	{"Japan", "Japan"},
	{"japan", ""},
	{"Asia/Ho_Chi_Minh", "Asia/Ho_Chi_Minh"},
	{"Asia/Saigon", "Asia/Saigon"},
	{"WET", "WET"},
	{"CET", "CET"},
	{"MST7MDT", "MST7MDT"},
	{"PST8PDT", "PST8PDT"},
	{"Navajo", "Navajo"},
	{"ROC", "ROC"},
	{"PRC", "PRC"},
}

func TestCanonicalAgreesWithChronoTzParse(t *testing.T) {
	for _, probe := range chronoTzParseProbes {
		got, ok := Canonical(probe.input)
		switch {
		case probe.want == "" && ok:
			t.Errorf("%q accepted, chrono-tz refuses it", probe.input)
		case probe.want != "" && (!ok || got != probe.want):
			t.Errorf("%q = %q,%v want %q", probe.input, got, ok, probe.want)
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
