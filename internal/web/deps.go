package web

import (
	"log/slog"
	"sync"
	"time"

	"github.com/henry40408/pingward/internal/config"
	"github.com/henry40408/pingward/internal/store"
)

// Deps is everything the web handlers share. Nothing is global.
type Deps struct {
	Store      *store.Store
	Config     *config.Config
	Logger     *slog.Logger
	Now        func() time.Time
	SetupToken *SetupToken
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
