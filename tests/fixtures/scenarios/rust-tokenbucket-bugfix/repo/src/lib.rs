//! Pinned #136 Slice D evaluation fixture (synthetic OSS-style crate).
//!
//! A small keyed rate limiter built on token buckets. The crate is a
//! pinned evaluation artifact: do not "fix" it in place - the content
//! digest is pinned by the scenario descriptor.

pub mod bucket;
pub mod clock;
pub mod limiter;

pub use bucket::TokenBucket;
pub use clock::{Clock, ManualClock, SystemClock};
pub use limiter::KeyedRateLimiter;
