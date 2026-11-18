package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"syscall"
	"testing"
	"time"

	"github.com/henry40408/pingward/internal/db"
	"github.com/henry40408/pingward/internal/testutil"
)

const waitLimit = 5 * time.Second

type running struct {
	interrupts chan os.Signal
	done       chan int
	stdout     *testutil.LockedBuffer
	stderr     *testutil.LockedBuffer
}

// startServe runs the real serve on a random port over a throwaway database.
func startServe(t *testing.T, interrupts chan os.Signal) *running {
	t.Helper()
	env := map[string]string{
		"DATABASE_URL":        "sqlite://" + filepath.Join(t.TempDir(), "t.sqlite3") + "?mode=rwc",
		"PINGWARD_BIND":       "127.0.0.1:0",
		"PINGWARD_SECRET":     "pingward-test-secret-32-bytes-xx",
		"PINGWARD_LOG_FORMAT": "json",
	}
	r := &running{interrupts: interrupts, done: make(chan int, 1), stdout: &testutil.LockedBuffer{}, stderr: &testutil.LockedBuffer{}}
	go func() {
		r.done <- run(process{
			lookupEnv: lookupFrom(env), stdout: r.stdout, stderr: r.stderr,
			interrupts: interrupts, serve: serve,
		})
	}()
	return r
}

func (r *running) waitForExit(t *testing.T) int {
	t.Helper()
	select {
	case code := <-r.done:
		return code
	case <-time.After(waitLimit):
		t.Fatalf("serve did not exit within %s; log:\n%s", waitLimit, r.stdout.String())
		return 0
	}
}

func (r *running) listeningAddress(t *testing.T) string {
	t.Helper()
	deadline := time.Now().Add(waitLimit)
	for time.Now().Before(deadline) {
		for _, line := range strings.Split(r.stdout.String(), "\n") {
			var entry struct{ Msg string }
			if json.Unmarshal([]byte(line), &entry) == nil && strings.HasPrefix(entry.Msg, "listening on ") {
				return strings.TrimPrefix(entry.Msg, "listening on ")
			}
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("never listened; log:\n%s", r.stdout.String())
	return ""
}

func (r *running) wantLogged(t *testing.T, messages ...string) {
	t.Helper()
	log := r.stdout.String()
	for _, message := range messages {
		if !strings.Contains(log, `"msg":"`+message+`"`) {
			t.Errorf("log is missing %q:\n%s", message, log)
		}
	}
}

func TestShutdownResolvesAfterTheSignal(t *testing.T) {
	r := startServe(t, make(chan os.Signal, 1))
	r.listeningAddress(t)
	r.interrupts <- syscall.SIGTERM
	if code := r.waitForExit(t); code != 0 {
		t.Fatalf("exit code %d", code)
	}
	r.wantLogged(t, "received SIGTERM", "shutdown requested; draining", "scan loop stopping", "prune loop stopping", "database pool closed")
}

// A signal that arrives while the server is still starting must not be lost.
func TestShutdownResolvesImmediatelyWhenTheSignalCameFirst(t *testing.T) {
	interrupts := make(chan os.Signal, 1)
	interrupts <- syscall.SIGINT
	r := startServe(t, interrupts)
	if code := r.waitForExit(t); code != 0 {
		t.Fatalf("exit code %d", code)
	}
	r.wantLogged(t, "received SIGINT", "database pool closed")
}

// Fail-closed: a controller that goes away takes the server with it.
func TestShutdownResolvesWhenTheSignalSourceIsGone(t *testing.T) {
	interrupts := make(chan os.Signal, 1)
	r := startServe(t, interrupts)
	r.listeningAddress(t)
	close(interrupts)
	r.waitForExit(t)
	r.wantLogged(t, "interrupt channel closed", "database pool closed")
}

// Otherwise every wait would end on the first poll.
func TestServerKeepsRunningUntilTheSignal(t *testing.T) {
	r := startServe(t, make(chan os.Signal, 1))
	addr := r.listeningAddress(t)
	select {
	case code := <-r.done:
		t.Fatalf("exited with %d before any signal", code)
	case <-time.After(300 * time.Millisecond):
	}
	res, err := http.Get("http://" + addr + "/healthz")
	if err != nil {
		t.Fatal(err)
	}
	_ = res.Body.Close()
	if res.StatusCode != http.StatusOK {
		t.Fatalf("healthz: %d", res.StatusCode)
	}
	r.interrupts <- syscall.SIGTERM
	r.waitForExit(t)
}

func TestSetupTokenGoesToStderrOnly(t *testing.T) {
	r := startServe(t, make(chan os.Signal, 1))
	r.listeningAddress(t)
	r.interrupts <- syscall.SIGTERM
	r.waitForExit(t)
	const marker = "Setup token (enter it on /setup): "
	_, token, ok := strings.Cut(r.stderr.String(), marker)
	token = strings.TrimSpace(token)
	if !ok || len(token) != 11 {
		t.Fatalf("stderr %q", r.stderr.String())
	}
	if strings.Contains(r.stdout.String(), token) {
		t.Fatal("the token leaked into the log stream")
	}
	r.wantLogged(t, "no users yet; setup token issued, see stderr")
}

func TestServeFailsWhenTheDatabaseCannotBeOpened(t *testing.T) {
	var stderr testutil.LockedBuffer
	code := run(process{
		lookupEnv: lookupFrom(map[string]string{
			"DATABASE_URL":    "sqlite://" + filepath.Join(t.TempDir(), "missing", "t.sqlite3") + "?mode=rw",
			"PINGWARD_SECRET": "pingward-test-secret-32-bytes-xx",
		}),
		stdout: &testutil.LockedBuffer{}, stderr: &stderr,
		interrupts: make(chan os.Signal), serve: serve,
	})
	if code == 0 || !strings.Contains(stderr.String(), "failed to connect to database") {
		t.Fatalf("code %d stderr %q", code, stderr.String())
	}
}

// runServeWith runs the real serve with env on top of a working base
// configuration and returns the exit code and stderr.
func runServeWith(t *testing.T, overrides map[string]string) (code int, stderr string) {
	t.Helper()
	env := map[string]string{
		"PINGWARD_BIND":   "127.0.0.1:0",
		"PINGWARD_SECRET": "pingward-test-secret-32-bytes-xx",
	}
	for key, value := range overrides {
		env[key] = value
	}
	var out, errOut testutil.LockedBuffer
	code = run(process{
		lookupEnv: lookupFrom(env), stdout: &out, stderr: &errOut,
		interrupts: make(chan os.Signal, 1), serve: serve,
	})
	return code, errOut.String()
}

func TestServeExitsWhenTheDatabaseCannotBeOpened(t *testing.T) {
	missingDirectory := filepath.Join(t.TempDir(), "absent", "t.sqlite3")
	code, stderr := runServeWith(t, map[string]string{"DATABASE_URL": "sqlite://" + missingDirectory + "?mode=rwc"})
	if code != exitFailure {
		t.Fatalf("exit code %d, want %d", code, exitFailure)
	}
	if !strings.Contains(stderr, "failed to") {
		t.Fatalf("stderr %q does not explain the failure", stderr)
	}
}

func TestServeExitsWhenAMigrationIsHalfApplied(t *testing.T) {
	path := filepath.Join(t.TempDir(), "t.sqlite3")
	url := "sqlite://" + path + "?mode=rwc"
	database, err := db.Open(url)
	if err != nil {
		t.Fatal(err)
	}
	if err := database.Migrate(context.Background()); err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec("UPDATE _sqlx_migrations SET success = false WHERE version = 1"); err != nil {
		t.Fatal(err)
	}
	if err := database.Close(); err != nil {
		t.Fatal(err)
	}
	code, stderr := runServeWith(t, map[string]string{"DATABASE_URL": url})
	if code != exitFailure || !strings.Contains(stderr, "partially applied") {
		t.Fatalf("exit code %d, stderr %q", code, stderr)
	}
}

func TestWaitIdleWaitsForARunningHandlerWithinTheLimit(t *testing.T) {
	handlers := &inflightHandlers{}
	started, release := make(chan struct{}), make(chan struct{})
	handler := handlers.track(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		close(started)
		<-release
	}))
	go handler.ServeHTTP(httptest.NewRecorder(), httptest.NewRequest("GET", "/", nil))
	<-started
	if handlers.waitIdle(50 * time.Millisecond) {
		t.Fatal("reported idle while a handler was running")
	}
	time.AfterFunc(100*time.Millisecond, func() { close(release) })
	if !handlers.waitIdle(waitLimit) {
		t.Fatal("did not see the handler finish")
	}
}

func TestDrainHTTPWaitsForHandlersCutOffByClose(t *testing.T) {
	handlers := &inflightHandlers{}
	started := make(chan struct{})
	var finished atomic.Bool
	srv := httptest.NewUnstartedServer(handlers.track(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		close(started)
		<-r.Context().Done() // Close cancels the request; finishing takes a moment.
		time.Sleep(300 * time.Millisecond)
		finished.Store(true)
	})))
	srv.Start()
	defer srv.Close()
	go http.Get(srv.URL) //nolint:errcheck
	<-started
	// Shutdown would wait for the handler itself, so exercise the Close path.
	_ = srv.Config.Close()
	if !handlers.waitIdle(handlerGrace) || !finished.Load() {
		t.Fatalf("handler not finished after the wait (finished=%v)", finished.Load())
	}
}
