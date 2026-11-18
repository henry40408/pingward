package harness

import (
	"slices"
	"testing"
)

func TestOptionsFromTags(t *testing.T) {
	options := OptionsFromTags([]string{"fast-scan", "smtp-env", "trusted-proxy"})
	if options.ScanIntervalSecs != 1 {
		t.Fatalf("scan interval = %d, want 1", options.ScanIntervalSecs)
	}
	for _, want := range []string{
		"PINGWARD_SMTP_HOST=smtp.e2e.test",
		"PINGWARD_SMTP_FROM=alerts@e2e.test",
		"PINGWARD_SMTP_PASSWORD=e2e-secret-password",
		"PINGWARD_TRUSTED_PROXIES=127.0.0.1",
	} {
		if !slices.Contains(options.ExtraEnv, want) {
			t.Errorf("ExtraEnv lacks %s: %v", want, options.ExtraEnv)
		}
	}
}

func TestOptionsFromNoTagsChangeNothing(t *testing.T) {
	options := OptionsFromTags([]string{"nojs"})
	if options.ScanIntervalSecs != 0 || len(options.ExtraEnv) != 0 {
		t.Fatalf("options = %+v, want zero", options)
	}
}
