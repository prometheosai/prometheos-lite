//! Clock abstraction so rate limiting is deterministic under test.

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

/// Monotonic millisecond clock.
pub trait Clock {
    fn now_millis(&self) -> u64;
}

/// Deterministic clock for tests and controlled evaluation workspaces.
pub struct ManualClock {
    now: Cell<u64>,
}

impl ManualClock {
    pub fn new() -> Self {
        Self { now: Cell::new(0) }
    }

    pub fn with_start(start_millis: u64) -> Self {
        Self { now: Cell::new(start_millis) }
    }

    pub fn advance(&self, delta_millis: u64) {
        self.now.set(self.now.get().saturating_add(delta_millis));
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn now_millis(&self) -> u64 {
        self.now.get()
    }
}

/// Wall-clock milliseconds since the Unix epoch.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}
