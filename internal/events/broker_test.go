package events

import (
	"testing"
	"time"
)

func TestSubscribersReceiveAPublishedID(t *testing.T) {
	broker := NewBroker()
	first, cancelFirst := broker.Subscribe()
	second, cancelSecond := broker.Subscribe()
	defer cancelFirst()
	defer cancelSecond()
	broker.Publish(7)
	for _, subscriber := range []<-chan int64{first, second} {
		if got := <-subscriber; got != 7 {
			t.Fatalf("got %d, want 7", got)
		}
	}
}

func TestSubscribersCountsLiveSubscribersOnly(t *testing.T) {
	broker := NewBroker()
	if broker.Subscribers() != 0 {
		t.Fatal("a new broker has no subscribers")
	}
	_, cancel := broker.Subscribe()
	if broker.Subscribers() != 1 {
		t.Fatal("want one subscriber")
	}
	cancel()
	if broker.Subscribers() != 0 {
		t.Fatal("cancel must remove the subscriber")
	}
}

// A slow subscriber must not stall the sender (a ping handler); it drops ids.
func TestAFullSubscriberDropsInsteadOfBlocking(t *testing.T) {
	broker := NewBroker()
	slow, cancel := broker.Subscribe()
	defer cancel()
	done := make(chan struct{})
	go func() {
		for id := range subscriberBuffer + 50 {
			broker.Publish(int64(id))
		}
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("Publish blocked on a full subscriber")
	}
	if len(slow) != subscriberBuffer {
		t.Fatalf("buffered = %d, want %d", len(slow), subscriberBuffer)
	}
}

func TestPublishWithoutSubscribersIsANoOp(t *testing.T) {
	NewBroker().Publish(1)
}
