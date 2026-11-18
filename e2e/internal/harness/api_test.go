package harness

import "testing"

func TestParsePingKind(t *testing.T) {
	cases := map[string]string{"success": "", "fail": "/fail", "start": "/start", "log": "/log", "exit 3": "/3", "exit 0": "/0"}
	for name, suffix := range cases {
		kind, err := ParsePingKind(name)
		if err != nil || kind.suffix != suffix {
			t.Errorf("ParsePingKind(%q) = %q, %v; want %q", name, kind.suffix, err, suffix)
		}
	}
	for _, bad := range []string{"bogus", "exit x", ""} {
		if _, err := ParsePingKind(bad); err == nil {
			t.Errorf("ParsePingKind(%q) accepted it", bad)
		}
	}
}
