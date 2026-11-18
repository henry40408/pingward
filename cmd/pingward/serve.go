package main

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"net/http"
	"os"
	"sync"
	"syscall"
	"time"

	"github.com/henry40408/pingward/internal/db"
	"github.com/henry40408/pingward/internal/prune"
	"github.com/henry40408/pingward/internal/scheduler"
	"github.com/henry40408/pingward/internal/server"
	"github.com/henry40408/pingward/internal/store"
	"github.com/henry40408/pingward/internal/web"
)

const (
	httpDrainTimeout = 3 * time.Second
	dbCloseTimeout   = 5 * time.Second
	// readHeaderTimeout guards against slow-header clients; it does not limit
	// bodies or the long-lived event streams.
	readHeaderTimeout = 10 * time.Second
)

// serve runs the app until a signal or a serve error, then drains.
func serve(p process, s started) int {
	database, err := openDatabase(s)
	if err != nil {
		fmt.Fprintf(p.stderr, "pingward: %v\n", err)
		return exitFailure
	}
	st := store.New(database)
	st.Logger = s.logger
	stopping := make(chan struct{})
	deps := web.NewDeps(st, &s.cfg, s.logger, time.Now)
	deps.Stopping = stopping
	if err := announceSetupToken(p, s.logger, deps); err != nil {
		fmt.Fprintf(p.stderr, "pingward: %v\n", err)
		closeDatabase(s.logger, database)
		return exitFailure
	}
	listener, err := net.Listen("tcp", s.cfg.Bind)
	if err != nil {
		fmt.Fprintf(p.stderr, "pingward: failed to bind %s: %v\n", s.cfg.Bind, err)
		closeDatabase(s.logger, database)
		return exitFailure
	}
	srv := &http.Server{
		Handler:           server.New(deps, web.Routes(deps), server.Mounts(deps)),
		ReadHeaderTimeout: readHeaderTimeout,
	}
	loops := startLoops(s, deps)
	s.logger.Info("listening on " + listener.Addr().String())
	serveErr := make(chan error, 1)
	go func() { serveErr <- srv.Serve(listener) }()

	code := 0
	select {
	case sig, ok := <-p.interrupts:
		// A closed channel means the controller is gone: stop, fail-closed.
		s.logger.Info(interruptReason(sig, ok))
	case err := <-serveErr:
		s.logger.Error("http server stopped", "error", err)
		code = exitFailure
	}
	s.logger.Info("shutdown requested; draining")
	close(stopping)
	loops.stop()
	drainHTTP(s.logger, srv)
	loops.wait()
	closeDatabase(s.logger, database)
	return code
}

func openDatabase(s started) (*db.DB, error) {
	database, err := db.Open(s.cfg.DatabaseURL)
	if err != nil {
		return nil, fmt.Errorf("failed to connect to database: %w", err)
	}
	if err := database.Migrate(context.Background()); err != nil {
		_ = database.Close()
		return nil, fmt.Errorf("failed to run migrations: %w", err)
	}
	return database, nil
}

// announceSetupToken prints the token on stderr only: stdout is the log
// stream and may be shipped.
func announceSetupToken(p process, logger *slog.Logger, deps *web.Deps) error {
	token, err := deps.ArmSetupTokenIfUnclaimed(context.Background())
	if err != nil {
		return fmt.Errorf("failed to check for existing users: %w", err)
	}
	if token == "" {
		return nil
	}
	fmt.Fprintf(p.stderr, "pingward: no users yet. Setup token (enter it on /setup): %s\n", token)
	logger.Warn("no users yet; setup token issued, see stderr")
	return nil
}

func interruptReason(sig os.Signal, open bool) string {
	switch {
	case !open:
		return "interrupt channel closed"
	case sig == syscall.SIGTERM:
		return "received SIGTERM"
	}
	return "received SIGINT"
}

// backgroundLoops are the scan and prune loops. They observe cancellation
// only while sleeping, so a pass in flight finishes before the database closes.
type backgroundLoops struct {
	cancel context.CancelFunc
	group  sync.WaitGroup
}

func startLoops(s started, deps *web.Deps) *backgroundLoops {
	ctx, cancel := context.WithCancel(context.Background())
	loops := &backgroundLoops{cancel: cancel}
	scan := scheduler.Deps{
		Store: deps.Store, Logger: s.logger, BaseURL: s.cfg.BaseURL,
		EnvDefaultScanSecs: s.cfg.ScanIntervalSecs, Events: deps.Events,
		Deliver: server.NewDeliverer(deps).DeliverEvent, Now: deps.Now,
	}
	pruning := prune.Loop{Store: deps.Store, Logger: s.logger, IntervalSecs: s.cfg.PruneIntervalSecs, Now: deps.Now}
	loops.group.Add(2)
	go func() { defer loops.group.Done(); scan.RunScanLoop(ctx) }()
	go func() { defer loops.group.Done(); pruning.Run(ctx) }()
	return loops
}

func (l *backgroundLoops) stop() { l.cancel() }
func (l *backgroundLoops) wait() { l.group.Wait() }

// drainHTTP gives connections 3 seconds, then closes them: event streams never
// end by themselves.
func drainHTTP(logger *slog.Logger, srv *http.Server) {
	ctx, cancel := context.WithTimeout(context.Background(), httpDrainTimeout)
	defer cancel()
	err := srv.Shutdown(ctx)
	if err == nil {
		return
	}
	if !errors.Is(err, context.DeadlineExceeded) {
		logger.Error("http shutdown failed", "error", err)
	}
	logger.Warn("http connections still open after 3s; closing them")
	_ = srv.Close()
}

// closeDatabase bounds the close at 5 seconds so a stuck connection cannot
// keep the process alive. SQLite checkpoints the WAL first so the file left
// behind is self-contained.
func closeDatabase(logger *slog.Logger, database *db.DB) {
	closed := make(chan struct{})
	go func() {
		defer close(closed)
		if database.Dialect() == db.SQLite {
			if _, err := database.ExecContext(context.Background(), "PRAGMA wal_checkpoint(TRUNCATE)"); err != nil {
				logger.Warn("wal checkpoint failed", "error", err)
			}
		}
		if err := database.Close(); err != nil {
			logger.Warn("database close failed", "error", err)
		}
	}()
	select {
	case <-closed:
		logger.Info("database pool closed")
	case <-time.After(dbCloseTimeout):
		logger.Warn("database pool did not close within 5s; exiting anyway")
	}
}
