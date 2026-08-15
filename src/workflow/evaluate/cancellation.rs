//! First-class cooperative cancellation for the evaluation pipeline.
//!
//! Issue #114: cancellation is a distinct control-flow signal, NOT a failure.
//! A cancelled run stops at the next safe point, durably records where it
//! stopped (a same-state journal event classified `"cancelled"`), stops its
//! heartbeat cleanly, fences further writes, and never deletes the proposal,
//! evidence, checkpoint, or any PortableWorkState. A later run resumes from the
//! authoritative journal position.
//!
//! If cancellation races generation (the provider is in flight), the outcome
//! is *uncertain*: the pipeline records the cancellation at `Generating` and
//! recovery subsequently reports [`crate::workflow::evaluate::recovery::RecoveryDisposition::GenerationOutcomeUnknown`]
//! rather than re-invoking the provider.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Barrier;

/// A shareable, awaitable cancellation signal.
///
/// Cloning shares the same signal; calling [`cancel`](Self::cancel) on any
/// clone notifies every waiter.
#[derive(Clone, Default)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
    // Optional test-only rendezvous barrier. When present, the pipeline parks at
    // each safe point until the test releases it, making cancellation
    // deterministic in tests. Always `None` in production.
    park: Option<Arc<Barrier>>,
}

impl CancellationToken {
    /// A token that is never cancelled (the default for plain `evaluate`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Test-only: create a token that parks the run at each safe point until the
    /// test releases the barrier. This removes the race between a test observing
    /// durable progress and cancelling a run that might otherwise race ahead and
    /// publish a terminal event before the cancel arrives.
    pub fn with_park_barrier(barrier: Arc<Barrier>) -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(tokio::sync::Notify::new()),
            park: Some(barrier),
        }
    }

    /// Cancel the operation. Idempotent; wakes all awaiters.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// True once cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Await until cancellation is requested.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        // Register interest before re-checking the flag so a cancel between
        // the check and the wait is not missed.
        let notified = self.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }

    /// Park the run at a safe point if a test barrier is installed. No-op in
    /// production (no barrier).
    pub async fn park_at_safe_point(&self) {
        if let Some(b) = &self.park {
            b.wait().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_is_idempotent_and_shared() {
        let token = CancellationToken::new();
        let clone = token.clone();
        assert!(!token.is_cancelled());
        clone.cancel();
        clone.cancel();
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn cancelled_future_wakes_immediately() {
        let token = CancellationToken::new();
        token.cancel();
        // Already cancelled: the future must complete immediately.
        tokio::time::timeout(std::time::Duration::from_secs(5), token.cancelled())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cancel_wakes_pending_waiter() {
        let token = CancellationToken::new();
        let waiter = token.clone();
        let handle = tokio::spawn(async move {
            waiter.cancelled().await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        token.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .unwrap()
            .unwrap();
    }
}
