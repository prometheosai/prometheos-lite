//! Token bucket with time-accumulating refill.

/// A token bucket that refills `refill_per_tick` tokens every `tick_millis`.
///
/// The bucket starts full. Refill is computed from the time that has
/// elapsed since the last applied refill, in whole ticks, and any
/// sub-tick remainder of elapsed time is carried forward to the next
/// refill.
pub struct TokenBucket {
    capacity: u64,
    tokens: u64,
    refill_per_tick: u64,
    tick_millis: u64,
    last_refill: u64,
}

impl TokenBucket {
    pub fn new(capacity: u64, refill_per_tick: u64, tick_millis: u64, now_millis: u64) -> Self {
        assert!(capacity > 0, "capacity must be positive");
        assert!(tick_millis > 0, "tick must be positive");
        Self {
            capacity,
            tokens: capacity,
            refill_per_tick,
            tick_millis,
            last_refill: now_millis,
        }
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    pub fn available(&self) -> u64 {
        self.tokens
    }

    fn refill(&mut self, now_millis: u64) {
        let elapsed = now_millis.saturating_sub(self.last_refill);
        let ticks = elapsed / self.tick_millis;
        if ticks == 0 {
            return;
        }
        let gained = ticks.saturating_mul(self.refill_per_tick);
        self.tokens = self.capacity.min(self.tokens.saturating_add(gained));
        self.last_refill = now_millis;
    }

    /// Attempt to spend `cost` tokens at `now_millis`.
    pub fn try_acquire(&mut self, now_millis: u64, cost: u64) -> bool {
        if cost == 0 {
            return true;
        }
        self.refill(now_millis);
        if self.tokens >= cost {
            self.tokens -= cost;
            true
        } else {
            false
        }
    }
}
