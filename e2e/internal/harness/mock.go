package harness

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"sync"
	"time"
)

const deliveryTimeout = 15 * time.Second

// MockWebhook is a receiver for notification scenarios. It answers 200 to
// everything (failed-delivery scenarios point elsewhere). Delivery is
// fire-and-forget, so WaitForPayload polls.
type MockWebhook struct {
	server *httptest.Server

	mu     sync.Mutex
	bodies [][]byte
}

// StartMockWebhook starts a receiver on a loopback port.
func StartMockWebhook() *MockWebhook {
	mock := &MockWebhook{}
	mock.server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		mock.mu.Lock()
		mock.bodies = append(mock.bodies, body)
		mock.mu.Unlock()
		_, _ = w.Write([]byte("ok"))
	}))
	return mock
}

// URL is where the receiver listens.
func (m *MockWebhook) URL() string { return m.server.URL }

// Close stops the receiver.
func (m *MockWebhook) Close() { m.server.Close() }

// Payloads are the JSON bodies received so far, in order; other bodies are
// skipped.
func (m *MockWebhook) Payloads() []map[string]any {
	m.mu.Lock()
	defer m.mu.Unlock()
	var payloads []map[string]any
	for _, body := range m.bodies {
		var payload map[string]any
		if json.Unmarshal(body, &payload) == nil {
			payloads = append(payloads, payload)
		}
	}
	return payloads
}

// WaitForPayload waits for a payload whose event field is event.
func (m *MockWebhook) WaitForPayload(event string) (map[string]any, error) {
	var found map[string]any
	err := EventuallyWithin(deliveryTimeout, fmt.Sprintf("a %q notification", event), func() (bool, error) {
		for _, payload := range m.Payloads() {
			if payload["event"] == event {
				found = payload
				return true, nil
			}
		}
		return false, nil
	})
	if err != nil {
		return nil, fmt.Errorf("%w; received %v", err, m.Payloads())
	}
	return found, nil
}

// ExpectNothing fails when any delivery has arrived.
func (m *MockWebhook) ExpectNothing() error {
	if payloads := m.Payloads(); len(payloads) > 0 {
		return fmt.Errorf("expected no delivery, but %d arrived", len(payloads))
	}
	return nil
}
