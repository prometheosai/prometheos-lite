//! Regression for dispatch 395/396: synchronized contenders reach the barrier
//! before acquiring the shared named mutex, and verification accounts for both
//! contenders. Uses the production test-mutex mechanism only; no provisioning
//! or ProgramData writes.
#![cfg(windows)]

use prometheos_lite::work::owner_registry::windows::acquire_test_mutex;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

#[test]
fn barrier_before_mutex_completes_within_watchdog() {
    let name = format!("Local\\PR341-DEADLOCK-WITNESS-{}", std::process::id());
    let barrier = Arc::new(Barrier::new(2));
    let (tx, rx) = mpsc::channel::<Result<(), String>>();

    let mut handles = Vec::new();
    for _ in 0..2 {
        let name = name.clone();
        let barrier = barrier.clone();
        let tx = tx.clone();
        handles.push(std::thread::spawn(move || {
            // Both parties synchronize first; only then do they contend on the
            // shared mutex, so no holder waits on a blocked partner.
            barrier.wait();
            let outcome = match acquire_test_mutex(&name) {
                Ok(guard) => {
                    drop(guard);
                    Ok(())
                }
                Err(error) => Err(format!("mutex acquisition refused: {error:?}")),
            };
            let _ = tx.send(outcome);
        }));
    }
    drop(tx);

    // Bounded watchdog shared by both acknowledgments. A timeout, a refusal or
    // a panic is collected as FAIL instead of ending the test early.
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut failures = Vec::new();
    for index in 0..2 {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);
        match rx.recv_timeout(remaining) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failures.push(format!("contender {index}: {error}")),
            Err(RecvTimeoutError::Timeout) => {
                failures.push(format!("contender {index}: bounded watchdog timeout"))
            }
            Err(RecvTimeoutError::Disconnected) => {
                failures.push(format!("contender {index}: no completion acknowledgment"))
            }
        }
    }
    // Successful verification never leaves a background thread unchecked.
    for (index, handle) in handles.into_iter().enumerate() {
        if handle.join().is_err() {
            failures.push(format!("contender {index}: thread panicked"));
        }
    }

    assert!(
        failures.is_empty(),
        "both contenders must complete before the watchdog: {}",
        failures.join("; ")
    );
}
