package main

import "testing"

func TestRunSucceedsWithoutConfiguration(t *testing.T) {
	if code := run(process{}); code != 0 {
		t.Fatalf("exit code %d, want 0", code)
	}
}
