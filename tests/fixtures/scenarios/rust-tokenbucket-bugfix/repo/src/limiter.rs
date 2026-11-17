//! Keyed rate limiter built on per-key token buckets.

use std::collections::HashMap;

use crate::bucket::TokenBucket;
use crate::clock::Clock;

/// Per-key token-bucket rate limiter.
///
/// Each key gets a bucket that refills `requests_per_window` tokens per
/// `window_millis`, up to a burst capacity of `burst` tokens. The bucket
/// starts full. Refill accrues from elapsed wall-clock time measured by
/// the shared clock, so partial windows contribute towards the next
/// refill.
pub struct KeyedRateLimiter<C: Clock> {
    clock: C,
    capacity: u64,
    refill_per_tick: u64,
    window_millis: u64,
    buckets: HashMap<String, TokenBucket>,
}

impl<C: Clock> KeyedRateLimiter<C> {
    pub fn new(clock: C, requests_per_window: u64, window_millis: u64, burst: u64) -> Self {
        assert!(requests_per_window > 0, "requests_per_window must be positive");
        assert!(window_millis > 0, "window_millis must be positive");
        assert!(burst > 0, "burst must be positive");
        Self {
            clock,
            capacity: burst,
            refill_per_tick: requests_per_window,
            window_millis,
            buckets: HashMap::new(),
        }
    }

    /// Attempt to admit one request for `key` at the current time.
    pub fn allow(&mut self, key: &str) -> bool {
        let now = self.clock.now_millis();
        self.buckets
            .entry(key.to_string())
            .or_insert_with(|| {
                TokenBucket::new(self.capacity, self.refill_per_tick, self.window_millis, now)
            })
            .try_acquire(now, 1)
    }

    pub fn tracked_keys(&self) -> usize {
        self.buckets.len()
    }

    pub fn clock(&self) -> &C {
        &self.clock
    }
}
