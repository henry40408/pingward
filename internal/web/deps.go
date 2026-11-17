package web

import (
	"log/slog"
	"net/netip"
	"sync"
	"time"

	"github.com/henry40408/pingward/internal/config"
	"github.com/henry40408/pingward/internal/elevate"
	"github.com/henry40408/pingward/internal/ratelimit"
	"github.com/henry40408/pingward/internal/store"
)

// Deps is everything the web handlers share. Nothing is global.
type Deps struct {
	Store      *store.Store
	Config     *config.Config
	Logger     *slog.Logger
	Now        func() time.Time
	SetupToken *SetupToken

	LoginLimiter   *ratelimit.Limiter[netip.Addr]
	SetupLimiter   *ratelimit.Limiter[netip.Addr]
	AccountLimiter *ratelimit.Limiter[string]
	Elevations     *elevate.Elevations
}

// NewDeps wires the limiters and elevation grants a server needs. now is read
// once for the limiters' overflow windows and then per request.
func NewDeps(store *store.Store, cfg *config.Config, logger *slog.Logger, now func() time.Time) *Deps {
	start := now()
	return &Deps{
		Store: store, Config: cfg, Logger: logger, Now: now, SetupToken: &SetupToken{},
		LoginLimiter:   ratelimit.New[netip.Addr](ratelimit.LoginMaxAttempts, ratelimit.LoginWindow, start),
		SetupLimiter:   ratelimit.New[netip.Addr](ratelimit.SetupMaxAttempts, ratelimit.SetupWindow, start),
		AccountLimiter: ratelimit.New[string](ratelimit.AccountMaxAttempts, ratelimit.AccountWindow, start),
		Elevations:     elevate.New(elevate.TTL),
	}
}

// SetupToken is the one-time token printed at startup while no user exists.
type SetupToken struct {
	mu    sync.Mutex
	token string
}

func (s *SetupToken) Arm(token string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.token = token
}

// Get returns "" when no token is armed.
func (s *SetupToken) Get() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.token
}

func (s *SetupToken) Clear() { s.Arm("") }
