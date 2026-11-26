//! Regression for dispatch 395/396/397: synchronized contenders reach the
//! barrier before acquiring the shared test mutex, and verification accounts
//! for both contenders without an unbounded join.
#![cfg(windows)]

use prometheos_lite::work::owner_registry::windows::acquire_test_mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

struct ProcessWatchdog {
    armed: Arc<AtomicBool>,
    deadline: Instant,
    join: Option<std::thread::JoinHandle<()>>,
}

impl ProcessWatchdog {
    fn new(timeout: Duration) -> Self {
        let armed = Arc::new(AtomicBool::new(true));
        let flag = armed.clone();
        let deadline = Instant::now() + timeout;
        let join = std::thread::spawn(move || {
            loop {
                if !flag.load(Ordering::SeqCst) {
                    return;
                }
                if Instant::now() >= deadline {
                    eprintln!(
                        "PR341_WATCHDOG_FAIL bounded process watchdog expired after {timeout:?}"
                    );
                    std::process::exit(101);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        Self {
            armed,
            deadline,
            join: Some(join),
        }
    }

    fn remaining(&self) -> Duration {
        self.deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO)
    }
}

impl Drop for ProcessWatchdog {
    fn drop(&mut self) {
        self.armed.store(false, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[test]
fn barrier_before_mutex_completes_within_watchdog() {
    let watchdog = ProcessWatchdog::new(Duration::from_secs(6));
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

    let mut failures = Vec::new();
    for index in 0..2 {
        match rx.recv_timeout(watchdog.remaining()) {
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
    // Join only after complete success. On timeout/refusal this test fails
    // immediately and never blocks on a possibly stuck worker thread.
    if failures.is_empty() {
        for (index, handle) in handles.into_iter().enumerate() {
            if handle.join().is_err() {
                failures.push(format!("contender {index}: thread panicked"));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "both contenders must complete before the watchdog: {}",
        failures.join("; ")
    );
}
