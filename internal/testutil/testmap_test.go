package testutil

import (
	"bufio"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

// minimumMappedTests is the number of Rust tests with a Go counterpart; raise
// it as the pending list shrinks. Before switching over, testdata/
// test_map_pending.txt must be empty or every entry justified in the exec log.
const minimumMappedTests = 709

func readEntries(t *testing.T, name string) []string {
	t.Helper()
	file, err := os.Open(filepath.Join("..", "..", "testdata", name))
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = file.Close() }()
	var entries []string
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line != "" && !strings.HasPrefix(line, "#") {
			entries = append(entries, line)
		}
	}
	return entries
}

// TestEveryMappedGoTestExists keeps testdata/test_map.txt honest: each Go test
// it names must be declared in its package, and the map may not shrink.
func TestEveryMappedGoTestExists(t *testing.T) {
	entries := readEntries(t, "test_map.txt")
	for _, line := range entries {
		fields := strings.Fields(line)
		if len(fields) != 2 {
			t.Errorf("malformed line %q", line)
			continue
		}
		dir, name, ok := strings.Cut(fields[1], "::")
		if !ok {
			t.Errorf("malformed line %q", line)
			continue
		}
		if !declaresTest(t, filepath.Join("..", "..", dir), name) {
			t.Errorf("%s: %s is not declared in %s", fields[0], name, dir)
		}
	}
	if len(entries) < minimumMappedTests {
		t.Errorf("only %d Rust tests are mapped, expected at least %d", len(entries), minimumMappedTests)
	}
}

// TestEveryRustTestIsMappedOrPending makes dropping a Rust test impossible:
// the map plus the pending list must equal the Rust inventory exactly.
func TestEveryRustTestIsMappedOrPending(t *testing.T) {
	seen := map[string]string{}
	for _, file := range []string{"test_map.txt", "test_map_pending.txt"} {
		for _, line := range readEntries(t, file) {
			rust := strings.Fields(line)[0]
			if previous, dup := seen[rust]; dup {
				t.Errorf("%s is listed in both %s and %s", rust, previous, file)
			}
			seen[rust] = file
		}
	}
	inventory := readEntries(t, "rust_tests.txt")
	for _, rust := range inventory {
		if _, ok := seen[rust]; !ok {
			t.Errorf("%s is neither mapped nor pending", rust)
		}
		delete(seen, rust)
	}
	for rust := range seen {
		t.Errorf("%s is not a Rust test in rust_tests.txt", rust)
	}
}

func declaresTest(t *testing.T, dir, name string) bool {
	t.Helper()
	declaration := regexp.MustCompile(`(?m)^func ` + regexp.QuoteMeta(name) + `\(`)
	files, err := filepath.Glob(filepath.Join(dir, "*_test.go"))
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range files {
		source, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		if declaration.Match(source) {
			return true
		}
	}
	return false
}
