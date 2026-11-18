// Package events fans check ids out to live-tail subscribers.
package events

import "sync"

// subscriberBuffer holds pending ids per subscriber. Every frame means "this
// check changed", so a dropped id merges into a later one.
const subscriberBuffer = 256

// Broker delivers check ids to subscribers without ever blocking a sender.
type Broker struct {
	mu          sync.Mutex
	subscribers map[chan int64]struct{}
}

func NewBroker() *Broker { return &Broker{subscribers: make(map[chan int64]struct{})} }

// Subscribe returns a channel of changed check ids and a cancel that must be
// called once the subscriber is gone.
func (b *Broker) Subscribe() (<-chan int64, func()) {
	events := make(chan int64, subscriberBuffer)
	b.mu.Lock()
	b.subscribers[events] = struct{}{}
	b.mu.Unlock()
	cancel := func() {
		b.mu.Lock()
		defer b.mu.Unlock()
		delete(b.subscribers, events)
	}
	return events, cancel
}

// Subscribers lets a sender skip the work when nobody is watching.
func (b *Broker) Subscribers() int {
	b.mu.Lock()
	defer b.mu.Unlock()
	return len(b.subscribers)
}

// Publish never blocks: a subscriber whose buffer is full misses the id.
func (b *Broker) Publish(checkID int64) {
	b.mu.Lock()
	defer b.mu.Unlock()
	for subscriber := range b.subscribers {
		select {
		case subscriber <- checkID:
		default:
		}
	}
}
