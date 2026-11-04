// Package poll waits for chain state to catch up, such as a submitted
// transaction showing up in the indexer.
package poll

import (
	"context"
	"time"
)

// Until calls ok every interval until it returns true or an error, or ctx is
// done. It checks once immediately. Bound the wait by giving ctx a deadline.
func Until(ctx context.Context, interval time.Duration, ok func(context.Context) (bool, error)) error {
	ticker := time.NewTicker(interval)
	defer ticker.Stop()
	for {
		done, err := ok(ctx)
		if err != nil || done {
			return err
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
		}
	}
}
