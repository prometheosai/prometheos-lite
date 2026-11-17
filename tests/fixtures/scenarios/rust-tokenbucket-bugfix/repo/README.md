# fixture-tokenbucket

A small keyed rate limiter built on token buckets. Synthetic
OSS-style crate; a pinned evaluation fixture, never published.

Each key gets an independent token bucket that refills
`requests_per_window` tokens per `window_millis`, up to a burst
capacity. Buckets start full. Elapsed time is measured by a `Clock`
implementation (`ManualClock` under test, `SystemClock` in production).

## Example

```rust
use fixture_tokenbucket::{KeyedRateLimiter, SystemClock};

let mut limiter = KeyedRateLimiter::new(SystemClock, 10, 60_000, 20);
assert!(limiter.allow("user-42"));
```
