//! Regression for dispatch 395: synchronized contenders reach the barrier
//! before acquiring the shared named mutex.
//! Uses the production test-mutex mechanism only; no provisioning or ProgramData writes.
#![cfg(windows)]

use prometheos_lite::work::owner_registry::windows::acquire_test_mutex;
use std::sync::{Arc, Barrier};
use std::time::Duration;

#[test]
fn barrier_before_mutex_completes_within_watchdog() {
    let name = format!("Local\\PR341-DEADLOCK-WITNESS-{}", std::process::id());
    let barrier = Arc::new(Barrier::new(2));
    let (tx, rx) = std::sync::mpsc::channel();

    for _ in 0..2 {
        let name = name.clone();
        let barrier = barrier.clone();
        let tx = tx.clone();
        std::thread::spawn(move || {
            // Both parties synchronize first; only then do they contend on the
            // shared mutex, so no holder waits on a blocked partner.
            barrier.wait();
            let guard = acquire_test_mutex(&name).unwrap();
            drop(guard);
            let _ = tx.send(());
        });
    }
    drop(tx);

    assert!(
        rx.recv_timeout(Duration::from_secs(3)).is_ok(),
        "both contenders must complete within the bounded watchdog"
    );
}
