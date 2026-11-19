package main

import (
	"bufio"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/testutil"
)

var binaryPath string

// coverDir is where a `-cover` binary writes its profile; empty builds the
// binary plain. `go test -coverprofile` cannot see an exec'd binary.
var coverDir = os.Getenv("PINGWARD_COVERDIR")

// TestMain builds the binary once for the tests that run it as a process.
func TestMain(m *testing.M) {
	dir, err := os.MkdirTemp("", "pingward-bin")
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	binaryPath = filepath.Join(dir, "pingward")
	buildArgs := []string{"build", "-p", "2", "-o", binaryPath}
	if coverDir != "" {
		buildArgs = append(buildArgs, "-cover")
	}
	build := exec.Command("go", append(buildArgs, ".")...)
	build.Stderr = os.Stderr
	if err := build.Run(); err != nil {
		fmt.Fprintln(os.Stderr, "building the binary:", err)
		os.Exit(1)
	}
	code := m.Run()
	_ = os.RemoveAll(dir)
	os.Exit(code)
}

func freePort(t *testing.T) int {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = listener.Close() }()
	return listener.Addr().(*net.TCPAddr).Port
}

// command runs the binary with exactly env, so nothing leaks in from the
// developer's shell and nothing is written outside the temp dirs.
func command(t *testing.T, env ...string) *exec.Cmd {
	t.Helper()
	home := t.TempDir()
	cmd := exec.Command(binaryPath)
	cmd.Dir = home
	cmd.Env = append([]string{"HOME=" + home, "PATH=" + os.Getenv("PATH")}, env...)
	if coverDir != "" {
		cmd.Env = append(cmd.Env, "GOCOVERDIR="+coverDir)
	}
	return cmd
}

func serveEnv(t *testing.T, port int) []string {
	return []string{
		"DATABASE_URL=sqlite://" + filepath.Join(t.TempDir(), "t.sqlite3") + "?mode=rwc",
		fmt.Sprintf("PINGWARD_BIND=127.0.0.1:%d", port),
		"PINGWARD_SECRET=pingward-test-secret-32-bytes-xx",
		"PINGWARD_LOG_LEVEL=debug",
	}
}

func TestBinaryRefusesRustLogAndPrintsTheMigrationHint(t *testing.T) {
	cmd := command(t, "RUST_LOG=warn")
	output, err := cmd.CombinedOutput()
	if err == nil {
		t.Fatal("must exit non-zero")
	}
	if !strings.Contains(string(output), "RUST_LOG -> PINGWARD_LOG_LEVEL") {
		t.Fatalf("output %q", output)
	}
}

func waitUntilHealthy(t *testing.T, port int) {
	t.Helper()
	deadline := time.Now().Add(30 * time.Second)
	for time.Now().Before(deadline) {
		res, err := http.Get(fmt.Sprintf("http://127.0.0.1:%d/healthz", port))
		if err == nil {
			_ = res.Body.Close()
			if res.StatusCode == http.StatusOK {
				return
			}
		}
		time.Sleep(50 * time.Millisecond)
	}
	t.Fatal("the server did not start")
}

// The setup token must reach the operator's stderr and nothing that gets
// shipped, even at debug level: run the binary and look for it everywhere else.
func TestBinaryPrintsTheSetupTokenOnStderrOnlyAndStopsOnSIGTERM(t *testing.T) {
	port := freePort(t)
	cmd := command(t, serveEnv(t, port)...)
	var stdout testutil.LockedBuffer
	cmd.Stdout = &stdout
	stderrPipe, err := cmd.StderrPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cmd.Process.Kill() })

	const marker = "Setup token (enter it on /setup): "
	token, otherStderr := "", &strings.Builder{}
	scanned := make(chan struct{})
	go func() {
		defer close(scanned)
		scanner := bufio.NewScanner(stderrPipe)
		for scanner.Scan() {
			if _, announced, ok := strings.Cut(scanner.Text(), marker); ok {
				token = strings.TrimSpace(announced)
				continue
			}
			otherStderr.WriteString(scanner.Text() + "\n")
		}
	}()

	waitUntilHealthy(t, port)
	if err := cmd.Process.Signal(syscall.SIGTERM); err != nil {
		t.Fatal(err)
	}
	exited := make(chan error, 1)
	go func() { exited <- cmd.Wait() }()
	select {
	case err := <-exited:
		if err != nil {
			t.Fatalf("exit: %v", err)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("the process did not exit within 10s of SIGTERM")
	}
	<-scanned

	if len(token) != 11 {
		t.Fatalf("token %q", token)
	}
	if strings.Contains(stdout.String(), token) {
		t.Fatal("the token leaked to stdout logs")
	}
	if strings.Contains(otherStderr.String(), token) {
		t.Fatal("the token leaked to a stderr log line")
	}
	if !strings.Contains(stdout.String(), "received SIGTERM") {
		t.Fatalf("stdout %q", stdout.String())
	}
}
