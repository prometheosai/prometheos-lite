//! Pinned #136 Slice D evaluation task: the keyed rate limiter's refill
//! accounting does not carry sub-window remainder time forward.
//!
//! `cargo test --offline` currently fails on
//! `refill_accumulates_sub_window_remainder`. Fix the crate so the full
//! test suite passes without changing the public API or the tests.

use fixture_tokenbucket::{Clock, KeyedRateLimiter, ManualClock};

#[test]
fn burst_is_granted_up_to_capacity_then_denied() {
    let clock = ManualClock::new();
    let mut limiter = KeyedRateLimiter::new(clock, 2, 1_000, 3);
    assert!(limiter.allow("a"));
    assert!(limiter.allow("a"));
    assert!(limiter.allow("a"));
    assert!(!limiter.allow("a"));
}

#[test]
fn full_window_elapsed_refills_quota() {
    let clock = ManualClock::new();
    let mut limiter = KeyedRateLimiter::new(clock, 2, 1_000, 2);
    assert!(limiter.allow("a"));
    assert!(limiter.allow("a"));
    assert!(!limiter.allow("a"));
    limiter.clock().advance(1_000);
    assert!(limiter.allow("a"));
    assert!(limiter.allow("a"));
    assert!(!limiter.allow("a"));
}

#[test]
fn refill_accumulates_sub_window_remainder() {
    let clock = ManualClock::new();
    let mut limiter = KeyedRateLimiter::new(clock, 1, 1_000, 1);
    assert!(limiter.allow("a"));
    assert!(!limiter.allow("a"));
    // The limiter should drain at most one token per 1000 ms of elapsed
    // time per key.
    limiter.clock().advance(1_500);
    assert!(limiter.allow("a"));
    limiter.clock().advance(600);
    assert!(limiter.allow("a"));
    assert!(!limiter.allow("a"));
}

#[test]
fn keys_are_accounted_independently() {
    let clock = ManualClock::new();
    let mut limiter = KeyedRateLimiter::new(clock, 1, 1_000, 1);
    assert!(limiter.allow("a"));
    assert!(!limiter.allow("a"));
    assert!(limiter.allow("b"));
}

#[test]
fn zero_cost_spend_is_always_allowed() {
    let clock = ManualClock::new();
    let mut limiter = KeyedRateLimiter::new(clock, 1, 1_000, 1);
    assert!(limiter.allow("a"));
    let now = limiter.clock().now_millis();
    let mut bucket = fixture_tokenbucket::TokenBucket::new(1, 1, 1_000, now);
    assert!(bucket.try_acquire(now, 0));
}
