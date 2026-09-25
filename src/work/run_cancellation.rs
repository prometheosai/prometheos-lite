//! RunCancelRegistry — per-context cooperative cancellation registry for
//! WorkContext runs (#222).
//!
//! The API server builds a fresh `WorkOrchestrator` per request, so a run in
//! flight holds no handle any other request can reach. This registry closes
//! that gap: `POST /work-contexts/:id/run` registers a
//! [`CancellationToken`] for the duration of the run (RAII via
//! [`RunGuard`]), and `POST /work-contexts/:id/cancel` fires every token
//! registered for the context after the durable status flip commits.
//!
//! Semantics:
//! - **Identity**: each `register` returns a guard owning exactly one token.
//!   Dropping the guard removes that token and no other, so concurrent runs
//!   of the same context each keep their own signal (no clobbering).
//! - **Idempotency**: `fire` on an id with no registered tokens is a no-op
//!   (returns 0); firing an already-fired token is a no-op on the token
//!   (see `CancellationToken::cancel`).
//! - **Concurrent runs**: `Vec` per id — `fire` wakes every registered run.
//!   Rejecting concurrent runs of one context would be a behavior change
//!   deliberately left out of this slice (see the #222 investigation
//!   report).
//!
//! Cross-process honesty: this registry is per-process memory. A cancel
//! issued from another process (or a second server instance) is observed
//! through the durable `Cancelled` status at the run loop's cancellation
//! checkpoints — graceful polling after control returns to the loop, never
//! an immediate mid-step wake. See `WorkOrchestrator::run_until_blocked_or_complete_with_token`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::workflow::evaluate::CancellationToken;

/// Registry of live per-context run tokens.
#[derive(Default)]
pub struct RunCancelRegistry {
    inner: Mutex<HashMap<String, Vec<(u64, CancellationToken)>>>,
    next_guard_id: AtomicU64,
}

impl RunCancelRegistry {
    /// Register a cancellation token for `context_id`. The returned guard
    /// removes exactly this token on drop.
    pub fn register(self: &Arc<Self>, context_id: &str) -> RunGuard {
        let guard_id = self.next_guard_id.fetch_add(1, Ordering::SeqCst);
        let token = CancellationToken::new();
        let mut map = self.inner.lock().expect("RunCancelRegistry mutex poisoned");
        map.entry(context_id.to_string())
            .or_default()
            .push((guard_id, token.clone()));
        RunGuard {
            registry: Arc::clone(self),
            context_id: context_id.to_string(),
            guard_id,
            token,
        }
    }

    /// Fire every token registered for `context_id`. Returns how many runs
    /// were signalled. No registered runs => 0 (idempotent no-op).
    pub fn fire(&self, context_id: &str) -> usize {
        let map = self.inner.lock().expect("RunCancelRegistry mutex poisoned");
        match map.get(context_id) {
            Some(tokens) => {
                for (_, token) in tokens {
                    token.cancel();
                }
                tokens.len()
            }
            None => 0,
        }
    }
}

/// RAII registration handle. Dropping removes exactly the token this guard
/// registered; other concurrent runs' tokens are untouched.
pub struct RunGuard {
    registry: Arc<RunCancelRegistry>,
    context_id: String,
    guard_id: u64,
    token: CancellationToken,
}

impl RunGuard {
    /// The token registered for this run. Cloning shares the same signal.
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        let mut map = self
            .registry
            .inner
            .lock()
            .expect("RunCancelRegistry mutex poisoned");
        if let Some(tokens) = map.get_mut(&self.context_id) {
            tokens.retain(|(id, _)| *id != self.guard_id);
            if tokens.is_empty() {
                map.remove(&self.context_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_removes_only_its_own_token() {
        let registry = Arc::new(RunCancelRegistry::default());
        let guard1 = registry.register("ctx-1");
        let guard2 = registry.register("ctx-1");
        let t1 = guard1.token();
        let t2 = guard2.token();

        drop(guard1);
        // Only guard2's token remains: firing must reach t2 and not t1.
        assert_eq!(registry.fire("ctx-1"), 1);
        assert!(!t1.is_cancelled());
        assert!(t2.is_cancelled());
    }

    #[test]
    fn fire_is_idempotent_and_reaches_all_concurrent_runs() {
        let registry = Arc::new(RunCancelRegistry::default());
        let guard1 = registry.register("ctx-1");
        let guard2 = registry.register("ctx-1");

        assert_eq!(registry.fire("ctx-1"), 2);
        assert!(guard1.token().is_cancelled());
        assert!(guard2.token().is_cancelled());

        // Second fire: tokens are already cancelled; still reports how many
        // are registered, and cancelling again is a token-level no-op.
        assert_eq!(registry.fire("ctx-1"), 2);
        assert!(guard1.token().is_cancelled());
    }

    #[test]
    fn fire_with_no_registered_run_is_noop() {
        let registry = Arc::new(RunCancelRegistry::default());
        assert_eq!(registry.fire("no-such-context"), 0);
    }

    #[test]
    fn drop_then_register_again_gets_fresh_token() {
        let registry = Arc::new(RunCancelRegistry::default());
        let old = {
            let guard = registry.register("ctx-1");
            let token = guard.token();
            drop(guard);
            token
        };
        let guard = registry.register("ctx-1");
        let fresh = guard.token();

        assert_eq!(registry.fire("ctx-1"), 1);
        assert!(!old.is_cancelled());
        assert!(fresh.is_cancelled());
    }
}
