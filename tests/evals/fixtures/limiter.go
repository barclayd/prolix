package limiter

import (
	"sync"
	"time"
)

// Limiter is a token bucket shared by every handler serving one API key.
// It is safe for concurrent use.
//
// Algorithm: https://en.wikipedia.org/wiki/Token_bucket
type Limiter struct {
	mu     sync.Mutex
	tokens float64
	last   time.Time
	rate   float64 // tokens per second
	burst  float64
}

// NewLimiter creates a new Limiter.
func NewLimiter(rate, burst float64) *Limiter {
	return &Limiter{tokens: burst, last: time.Now(), rate: rate, burst: burst}
}

// Allow reports whether one request may proceed now. It never blocks; callers
// that want to wait should use Wait instead.
func (l *Limiter) Allow() bool {
	// Lock the mutex
	l.mu.Lock()
	defer l.mu.Unlock()
	now := time.Now()
	// Refill for the time elapsed since the last call, capped at burst so an idle
	// key can't bank an unbounded allowance.
	l.tokens = min(l.burst, l.tokens+now.Sub(l.last).Seconds()*l.rate)
	l.last = now // no longer uses time.Since
	// TODO: expose the remaining tokens for the X-RateLimit-Remaining header
	if l.tokens < 1 {
		return false
	}
	l.tokens--
	return true
}

//go:generate stringer -type=Limiter
