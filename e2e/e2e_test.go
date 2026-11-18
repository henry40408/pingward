package e2e

import (
	"fmt"
	"os"
	"runtime"
	"strings"
	"testing"

	"github.com/cucumber/godog"

	"github.com/henry40408/pingward/e2e/internal/harness"
)

const (
	defaultFeatures    = "features"
	concurrencyCeiling = 4
)

// TestE2E is the suite. Each scenario starts its own server, database and
// browser, since POST /setup succeeds only once.
func TestE2E(t *testing.T) {
	// First, so a malformed E2E_SHARD fails before any browser starts.
	shard, err := harness.ParseShard(os.Getenv("E2E_SHARD"))
	if err != nil {
		t.Fatal(err)
	}
	if err := preflight(); err != nil {
		t.Fatal(err)
	}
	suite := godog.TestSuite{
		Name: "pingward",
		ScenarioInitializer: func(sc *godog.ScenarioContext) {
			initializeScenario(sc, shard)
		},
		Options: &godog.Options{
			Format: "pretty",
			// ANSI codes make CI logs unreadable.
			NoColors: os.Getenv("CI") != "",
			Output:   os.Stdout,
			Paths:    featurePaths(),
			// One scenario per core: more overloads a small CI runner until
			// pages settle slower than the steps wait.
			Concurrency: min(runtime.NumCPU(), concurrencyCeiling),
			// Undefined and pending steps fail rather than skip.
			Strict: true,
		},
	}
	if status := suite.Run(); status != 0 {
		t.Fatalf("the suite failed with status %d", status)
	}
	if shard != nil {
		if err := shard.EnsureSelected(); err != nil {
			t.Fatal(err)
		}
	}
}

// preflight opens and closes one browser, so a missing Chromium gives one
// clear error rather than one per scenario.
func preflight() error {
	fmt.Fprintf(os.Stderr, "e2e: browser: %s\n", harness.BrowserChoice())
	opened, err := harness.Open()
	if err != nil {
		return err
	}
	opened.Close()
	return nil
}

// featurePaths is E2E_FEATURES, a comma-separated list of feature paths, for
// running one file locally. It defaults to every feature.
func featurePaths() []string {
	value := os.Getenv("E2E_FEATURES")
	if value == "" {
		return []string{defaultFeatures}
	}
	return strings.Split(value, ",")
}
