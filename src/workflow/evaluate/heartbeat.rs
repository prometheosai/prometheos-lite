use anyhow::{Result, bail};
use std::path::PathBuf;
use std::time::Duration;

use super::registry::{FenceToken, renew_heartbeat};

// ---------------------------------------------------------------------------
// Heartbeat lifecycle
// ---------------------------------------------------------------------------

/// Owns the heartbeat task and its stop/status channels for a single run.
///
/// The heartbeat renews the registry lease periodically while the pipeline is
/// active, from `Generating` through `Validating`, and stops only when the
/// pipeline reaches a terminal state or `ValidationComplete`. Errors and
/// ownership loss are propagated through the status watch channel.
pub(super) struct HeartbeatSession {
    stop_tx: tokio::sync::watch::Sender<bool>,
    status_rx: tokio::sync::watch::Receiver<Option<String>>,
    handle: tokio::task::JoinHandle<()>,
}

impl HeartbeatSession {
    /// Start a heartbeat task renewing the lease for `identity_key` every
    /// `interval`. `ownership_loss_message` is reported when the entry was
    /// claimed by another worker.
    pub(super) fn start(
        repo: PathBuf,
        identity_key: String,
        fence: FenceToken,
        interval: Duration,
        ownership_loss_message: &'static str,
    ) -> Self {
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let (status_tx, status_rx) = tokio::sync::watch::channel::<Option<String>>(None);
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = stop_rx.changed() => {
                        if *stop_rx.borrow() {
                            break;
                        }
                    }
                }
                if *stop_rx.borrow() {
                    break;
                }
                match renew_heartbeat(&repo, &identity_key, &fence) {
                    Err(e) => {
                        let _ = status_tx.send(Some(format!("{e:#}")));
                        break;
                    }
                    Ok(false) => {
                        let _ = status_tx.send(Some(ownership_loss_message.to_string()));
                        break;
                    }
                    Ok(true) => {}
                }
            }
        });
        Self {
            stop_tx,
            status_rx,
            handle,
        }
    }

    /// A clone of the status receiver, usable for racing generation or for
    /// passing into fenced finalization.
    pub(super) fn status_receiver(&self) -> tokio::sync::watch::Receiver<Option<String>> {
        self.status_rx.clone()
    }

    /// Bail if the heartbeat reported an error or ownership loss. `context`
    /// names the pipeline stage for the error message.
    pub(super) fn check(&self, context: &str) -> Result<()> {
        if let Some(msg) = self.status_rx.borrow().as_ref() {
            bail!("heartbeat failure{context}: {msg}");
        }
        Ok(())
    }

    /// Stop the heartbeat task, await it, and surface any error it reported.
    pub(super) async fn shutdown(self, context: &str) -> Result<()> {
        let Self {
            stop_tx,
            status_rx,
            handle,
        } = self;
        let _ = stop_tx.send(true);
        let _ = handle.await;
        if let Some(msg) = status_rx.borrow().as_ref() {
            bail!("heartbeat failure{context}: {msg}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::workflow::evaluate::registry::{
        LeaseConfig, OwnershipObservation, ProposalState, lookup_entry, try_reserve,
        try_take_ownership, try_take_ownership_cas,
    };
    use crate::workflow::evaluate::registry::{ReserveResult, TakeoverResult};

    fn fresh_entry(repo: &std::path::Path, key: &str, owner: &str) -> FenceToken {
        match try_reserve(repo, key, owner).unwrap() {
            ReserveResult::Owned(fence) => fence,
            ReserveResult::AlreadyExists => panic!("expected fresh reservation"),
        }
    }

    #[tokio::test]
    async fn healthy_owned_entry_shuts_down_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let key = "test-key".to_string();
        let fence = fresh_entry(&repo, &key, "worker-1");

        let session = HeartbeatSession::start(
            repo.clone(),
            key.clone(),
            fence,
            Duration::from_millis(20),
            "ownership lost",
        );
        // A few renewal cycles on a fresh, owned entry must stay healthy.
        tokio::time::sleep(Duration::from_millis(70)).await;
        session.check("test").unwrap();

        session.shutdown("test").await.unwrap();

        // Entry still owned by worker-1 after shutdown.
        let entry = lookup_entry(&repo, &key).unwrap();
        assert_eq!(entry.owner_run_id, "worker-1");
    }

    #[tokio::test]
    async fn ownership_loss_reaches_status_receiver() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let key = "test-key".to_string();
        let fence = fresh_entry(&repo, &key, "worker-1");

        let session = HeartbeatSession::start(
            repo.clone(),
            key.clone(),
            fence,
            Duration::from_millis(1000),
            "ownership lost: registry entry claimed by another worker",
        );

        // Short lease so the fresh Reserved entry becomes stale quickly; the
        // heartbeat interval (1s) is long enough that the entry is stale
        // before the next renewal, letting a takeover succeed.
        let lease = LeaseConfig::with_timeouts(
            Duration::from_millis(50), // stale_reservation_timeout
            Duration::from_millis(50),
            Duration::from_millis(50),
        );
        // Wait past the stale threshold, then take ownership from worker-1.
        tokio::time::sleep(Duration::from_millis(200)).await;
        match try_take_ownership(&repo, &key, "thief", &lease).unwrap() {
            TakeoverResult::Taken(_) => {}
            TakeoverResult::StillLive => panic!("expected takeover"),
            TakeoverResult::LostRace => panic!("expected takeover, lost the race"),
        }

        // The next heartbeat (at ~1s) must observe the ownership change and
        // report failure on the status channel.
        let mut rx = session.status_receiver();
        tokio::select! {
            _ = rx.changed() => {
                let msg = rx.borrow().clone();
                assert!(
                    msg.clone().unwrap().contains("ownership lost"),
                    "unexpected status: {msg:?}"
                );
            }
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                panic!("heartbeat did not report ownership loss");
            }
        }

        session.shutdown("test").await.unwrap_err();
    }

    #[tokio::test]
    async fn long_live_reserved_owner_is_not_reclaimed() {
        // Regression for a live `Reserved` owner surviving a long preflight.
        // The heartbeat starts the moment the reservation is taken, so a slow
        // preflight must never let a contender observe the owner as stale and
        // reclaim it.
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let key = "test-key".to_string();
        let fence = fresh_entry(&repo, &key, "worker-1");

        // Short but SAFE lease: heartbeat * 3 <= reservation timeout.
        let lease = LeaseConfig::with_timeouts(
            Duration::from_secs(1),     // stale_reservation_timeout
            Duration::from_secs(1),     // generation_lease_timeout
            Duration::from_millis(100), // heartbeat_interval
        );
        assert!(lease.validate().is_ok());

        let session = HeartbeatSession::start(
            repo.clone(),
            key.clone(),
            fence.clone(),
            Duration::from_millis(100),
            "ownership lost",
        );

        let observed = OwnershipObservation {
            owner_run_id: "worker-1".to_string(),
            lease_epoch: fence.lease_epoch,
            state: ProposalState::Reserved,
        };

        // Sample across many stale windows. The heartbeat keeps renewing, so a
        // contender that observed the owner must always back off as StillLive.
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let result =
                try_take_ownership_cas(&repo, &key, "thief", &lease, Some(&observed)).unwrap();
            assert!(
                matches!(result, TakeoverResult::StillLive),
                "a live Reserved owner must never be reclaimed: {result:?}"
            );
            let entry = lookup_entry(&repo, &key).unwrap();
            assert_eq!(entry.owner_run_id, "worker-1");
        }

        session.shutdown("test").await.unwrap();
    }
}
