// Package harness drives a pingward server and a browser for the e2e suite.
package harness

import (
	"bufio"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"
)

const startupTimeout = 30 * time.Second

const setupTokenMarker = "Setup token (enter it on /setup): "

// BinaryEnv names the variable that pins the server binary under test.
const BinaryEnv = "PINGWARD_BIN"

// Server is a pingward server with its own database. Every scenario starts
// one, since POST /setup works only once.
type Server struct {
	// BaseURL is where the browser goes.
	BaseURL string
	// SetupToken is the first-run token the server printed to stderr.
	SetupToken string

	cmd     *exec.Cmd
	dataDir string
}

// Start runs the pingward binary, building it first when it is missing.
func Start(options Options) (*Server, error) {
	binary, err := ensureBinary()
	if err != nil {
		return nil, err
	}
	port, err := freePort()
	if err != nil {
		return nil, err
	}
	dataDir, err := os.MkdirTemp("", "pingward-e2e-")
	if err != nil {
		return nil, err
	}
	server := &Server{BaseURL: "http://127.0.0.1:" + strconv.Itoa(port), dataDir: dataDir}
	if err := server.launch(binary, port, options); err != nil {
		server.Stop()
		return nil, err
	}
	return server, nil
}

func (s *Server) launch(binary string, port int, options Options) error {
	cmd := exec.Command(binary)
	cmd.Dir = RepoRoot()
	cmd.Env = append(os.Environ(),
		"DATABASE_URL=sqlite://"+filepath.Join(s.dataDir, "test.sqlite3")+"?mode=rwc",
		"PINGWARD_BIND=127.0.0.1:"+strconv.Itoa(port),
		"PINGWARD_BASE_URL="+s.BaseURL,
		"PINGWARD_LOG_LEVEL=warn",
		// Pinned to silence the random-secret startup warning.
		"PINGWARD_SECRET=pingward-e2e-secret-0123456789abcdef",
	)
	if options.ScanIntervalSecs > 0 {
		cmd.Env = append(cmd.Env, "PINGWARD_SCAN_INTERVAL="+strconv.Itoa(options.ScanIntervalSecs))
	}
	cmd.Env = append(cmd.Env, options.ExtraEnv...)
	// Teardown kills the server mid-request; its stdout would bury real
	// failures.
	cmd.Stdout = nil
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return err
	}
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("spawning %s: %w", binary, err)
	}
	s.cmd = cmd
	token := make(chan string, 1)
	go readSetupToken(stderr, token)
	if options.ExistingDatabase {
		return s.waitUntilHealthy()
	}
	select {
	case s.SetupToken = <-token:
	case <-time.After(startupTimeout):
		return fmt.Errorf("the server did not print a setup token in time")
	}
	return s.waitUntilHealthy()
}

// readSetupToken delivers the token the server prints, then keeps draining
// stderr so the server never blocks on a full pipe.
func readSetupToken(stderr io.Reader, token chan<- string) {
	lines := bufio.NewScanner(stderr)
	for lines.Scan() {
		_, found, ok := strings.Cut(lines.Text(), setupTokenMarker)
		if !ok {
			continue
		}
		select {
		case token <- strings.TrimSpace(found):
		default:
		}
	}
	_, _ = io.Copy(io.Discard, stderr)
}

// Stop kills the server and removes its data. Safe to call twice.
func (s *Server) Stop() {
	if s.cmd != nil {
		_ = s.cmd.Process.Kill()
		_ = s.cmd.Wait()
		s.cmd = nil
	}
	_ = os.RemoveAll(s.dataDir)
}

func (s *Server) waitUntilHealthy() error {
	deadline := time.Now().Add(startupTimeout)
	for time.Now().Before(deadline) {
		response, err := http.Get(s.BaseURL + "/healthz")
		if err == nil {
			_ = response.Body.Close()
			if response.StatusCode == http.StatusOK {
				return nil
			}
		}
		time.Sleep(50 * time.Millisecond)
	}
	return fmt.Errorf("pingward did not answer %s/healthz within %v", s.BaseURL, startupTimeout)
}

func freePort() (int, error) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return 0, fmt.Errorf("probing for a free port: %w", err)
	}
	defer listener.Close()
	return listener.Addr().(*net.TCPAddr).Port, nil
}

// ensureBinary returns the server binary: PINGWARD_BIN, else bin/pingward,
// built first when it is not there.
//
// An existing binary is never rebuilt, so rebuild by hand after changing
// server code: the views are compiled in, a stale binary shows a stale UI.
func ensureBinary() (string, error) {
	if pinned := os.Getenv(BinaryEnv); pinned != "" {
		if !isFile(pinned) {
			return "", fmt.Errorf("%s=%s is not a file", BinaryEnv, pinned)
		}
		return filepath.Abs(pinned)
	}
	binary := filepath.Join(RepoRoot(), "bin", "pingward")
	if isFile(binary) {
		return binary, nil
	}
	fmt.Fprintf(os.Stderr, "e2e: %s is missing, building it\n", binary)
	build := exec.Command("go", "build", "-mod=vendor", "-o", "bin/pingward", "./cmd/pingward")
	build.Dir = RepoRoot()
	build.Stdout = os.Stdout
	build.Stderr = os.Stderr
	if err := build.Run(); err != nil {
		return "", fmt.Errorf("running go build: %w", err)
	}
	return binary, nil
}

func isFile(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.Mode().IsRegular()
}

// RepoRoot is the repository root, three levels above e2e/internal/harness. It
// comes from this file's compile-time path, so the suite works from any
// working directory.
func RepoRoot() string {
	_, file, _, _ := runtime.Caller(0)
	return filepath.Join(filepath.Dir(file), "..", "..", "..")
}
