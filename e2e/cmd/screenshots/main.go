// Command screenshots is the re-runnable README screenshot pipeline:
//
//	wipe DB -> boot #1 (migrations) -> POST /setup -> stop
//	-> seed backdated demo history -> boot #2 -> log in -> capture -> stop
//
// Run it from e2e/: `go run ./cmd/screenshots`. It uses bin/pingward, built
// when missing. The server is stopped with a plain kill; SQLite recovers the
// WAL when seeding opens the database.
package main

import (
	"database/sql"
	"fmt"
	"os"
	"path/filepath"
	"time"

	_ "modernc.org/sqlite"

	"github.com/henry40408/pingward/e2e/internal/harness"
	"github.com/henry40408/pingward/e2e/internal/seed"
)

// publicBaseURL is rendered in ping URLs, keeping the random port out of the
// screenshots.
const publicBaseURL = "https://pingward.example.com"

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, "screenshots:", err)
		os.Exit(1)
	}
	fmt.Println("done — PNGs in docs/screenshots")
}

func run() error {
	root := harness.RepoRoot()
	database := filepath.Join(root, "e2e", ".tmp", "screenshots.sqlite3")
	output := filepath.Join(root, "docs", "screenshots")
	if err := prepareDirectories(database, output); err != nil {
		return err
	}
	if err := bootstrapAdmin(database); err != nil {
		return err
	}
	if err := applySeed(database); err != nil {
		return err
	}
	return captureAll(database, output)
}

func prepareDirectories(database, output string) error {
	if err := os.MkdirAll(filepath.Dir(database), 0o755); err != nil {
		return err
	}
	if err := os.MkdirAll(output, 0o755); err != nil {
		return err
	}
	for _, suffix := range []string{"", "-wal", "-shm"} {
		if err := os.Remove(database + suffix); err != nil && !os.IsNotExist(err) {
			return err
		}
	}
	return nil
}

func serverOptions(database string, existing bool) harness.Options {
	return harness.Options{
		ExistingDatabase: existing,
		ExtraEnv: []string{
			"DATABASE_URL=sqlite://" + database + "?mode=rwc",
			"PINGWARD_BASE_URL=" + publicBaseURL,
			"PINGWARD_SECRET=pingward-screenshots-0123456789abcdef",
		},
	}
}

// bootstrapAdmin migrates and creates the admin via /setup, for a real
// password hash.
func bootstrapAdmin(database string) error {
	server, err := harness.Start(serverOptions(database, false))
	if err != nil {
		return err
	}
	defer server.Stop()
	return harness.NewAPI(server).BootstrapAdmin(seed.AdminUsername, seed.AdminPassword)
}

func applySeed(database string) error {
	script, err := seed.SQL(time.Now().UnixMilli())
	if err != nil {
		return err
	}
	db, err := sql.Open("sqlite", database)
	if err != nil {
		return err
	}
	defer db.Close()
	// One connection, so the script's BEGIN and COMMIT share a transaction.
	db.SetMaxOpenConns(1)
	if _, err := db.Exec(script); err != nil {
		return fmt.Errorf("running the seed script: %w", err)
	}
	return nil
}

func captureAll(database, output string) error {
	server, err := harness.Start(serverOptions(database, true))
	if err != nil {
		return err
	}
	defer server.Stop()
	browser, err := harness.Open()
	if err != nil {
		return err
	}
	defer browser.Close()
	if err := signIn(browser, server.BaseURL); err != nil {
		return err
	}
	for _, current := range shots {
		if err := capture(browser, server.BaseURL, output, current); err != nil {
			return fmt.Errorf("%s: %w", current.file, err)
		}
		fmt.Println("captured", current.file)
	}
	return nil
}

// signIn logs in once for all shots; device and scheme overrides are re-issued
// per shot.
func signIn(browser *harness.Browser, baseURL string) error {
	if err := browser.EmulateLocale("UTC", "en-US"); err != nil {
		return err
	}
	if err := browser.Goto(baseURL + "/login"); err != nil {
		return err
	}
	if err := browser.Fill("username-input", seed.AdminUsername); err != nil {
		return err
	}
	if err := browser.Fill("password-input", seed.AdminPassword); err != nil {
		return err
	}
	if err := browser.Submit("login-submit"); err != nil {
		return err
	}
	return browser.ExpectVisible("nav-admin")
}

func capture(browser *harness.Browser, baseURL, output string, current shot) error {
	if err := browser.EmulateDevice(current.device); err != nil {
		return err
	}
	if err := browser.EmulateStillMedia(current.scheme); err != nil {
		return err
	}
	if err := browser.Goto(baseURL + current.goTo); err != nil {
		return err
	}
	if err := settlePage(browser, current.settle); err != nil {
		return err
	}
	if err := freezePage(browser); err != nil {
		return err
	}
	clip, err := frameClip(browser, current)
	if err != nil {
		return err
	}
	png, err := browser.CaptureClip(clip)
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(output, current.file), png, 0o644)
}
