package poll

import (
	"context"
	"errors"
	"testing"
	"time"
)

func TestUntilReturnsOnceDone(t *testing.T) {
	calls := 0
	err := Until(context.Background(), time.Millisecond, func(context.Context) (bool, error) {
		calls++
		return calls == 3, nil
	})
	if err != nil || calls != 3 {
		t.Fatalf("err = %v, calls = %d; want nil, 3", err, calls)
	}
}

func TestUntilStopsOnError(t *testing.T) {
	boom := errors.New("boom")
	err := Until(context.Background(), time.Millisecond, func(context.Context) (bool, error) {
		return false, boom
	})
	if !errors.Is(err, boom) {
		t.Fatalf("err = %v, want %v", err, boom)
	}
}

func TestUntilStopsAtDeadline(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	err := Until(ctx, time.Millisecond, func(context.Context) (bool, error) {
		return false, nil
	})
	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("err = %v, want %v", err, context.DeadlineExceeded)
	}
}
