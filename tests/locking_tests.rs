//! Cross-process file-lock tests (issue #114).
//!
//! [`WorkflowFileLock`] must exclude OTHER PROCESSES on every supported
//! platform. In-process unit tests can only prove exclusion on Unix (flock is
//! per open file description); Windows `LockFileEx` is per-process, so real
//! exclusion is verified here by spawning child processes that own separate
//! handles. Each child runs the same test binary via `--exact`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use prometheos_lite::workflow::evaluate::WorkflowFileLock;

const CHILD_ENV: &str = "PROMETHEOS_LOCK_CHILD";
const MODE_ENV: &str = "PROMETHEOS_LOCK_CHILD_MODE";
const REPO_ENV: &str = "PROMETHEOS_LOCK_REPO";
const LOCK_NAME_ENV: &str = "PROMETHEOS_LOCK_NAME";
const READY_ENV: &str = "PROMETHEOS_LOCK_READY";
const RELEASE_ENV: &str = "PROMETHEOS_LOCK_RELEASE";

fn child_command(repo: &Path, ready: &Path, release: Option<&Path>, mode: &str) -> Command {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.arg("--exact").arg("child_lock_routine");
    cmd.arg("--test-threads=1").arg("--nocapture");
    cmd.env(CHILD_ENV, "1");
    cmd.env(MODE_ENV, mode);
    cmd.env(REPO_ENV, repo);
    cmd.env(LOCK_NAME_ENV, "shared.lock");
    cmd.env(READY_ENV, ready);
    if let Some(r) = release {
        cmd.env(RELEASE_ENV, r);
    }
    cmd
}

fn wait_for(path: &Path, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while !path.exists() {
        if start.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

#[test]
fn child_process_cannot_acquire_lock_held_by_parent() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let ready = dir.path().join("ready");
    let _parent_lock = WorkflowFileLock::acquire(repo, "shared.lock").unwrap();

    let mut child = child_command(repo, &ready, None, "try").spawn().unwrap();
    assert!(
        wait_for(&ready, Duration::from_secs(60)),
        "child never signalled readiness"
    );
    let status = child.wait().unwrap();
    assert!(status.success(), "child must exit cleanly");

    let result = std::fs::read_to_string(&ready).unwrap();
    assert_eq!(
        result, "unlocked",
        "child must NOT acquire a lock held by the parent process"
    );
}

#[test]
fn lock_is_released_when_child_process_dies() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let ready = dir.path().join("ready");
    let release = dir.path().join("release");

    // Child acquires the lock and holds it until killed.
    let mut child = child_command(repo, &ready, Some(&release), "hold")
        .spawn()
        .unwrap();
    assert!(
        wait_for(&ready, Duration::from_secs(60)),
        "child never acquired the lock"
    );
    assert_eq!(std::fs::read_to_string(&ready).unwrap(), "locked");

    // The parent must observe the lock as held.
    assert!(
        WorkflowFileLock::try_acquire(repo, "shared.lock")
            .unwrap()
            .is_none(),
        "lock must be held by the child process"
    );

    // Killing the child must release the lock (OS semantics: handles close).
    child.kill().unwrap();
    child.wait().unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        if WorkflowFileLock::try_acquire(repo, "shared.lock")
            .unwrap()
            .is_some()
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "lock never released after child death"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn child_process_lock_released_on_clean_exit() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let ready = dir.path().join("ready");
    let release = dir.path().join("release");

    let mut child = child_command(repo, &ready, Some(&release), "hold")
        .spawn()
        .unwrap();
    assert!(
        wait_for(&ready, Duration::from_secs(60)),
        "child never acquired the lock"
    );
    assert!(
        WorkflowFileLock::try_acquire(repo, "shared.lock")
            .unwrap()
            .is_none(),
        "lock must be held by the child process"
    );

    // Signal the child to exit cleanly; the lock must be released.
    std::fs::write(&release, "go").unwrap();
    child.wait().unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        if WorkflowFileLock::try_acquire(repo, "shared.lock")
            .unwrap()
            .is_some()
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "lock never released after child exit"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Child routine executed by the spawned test binary. When the env var is not
/// set (a normal suite run) this is a no-op pass.
#[test]
fn child_lock_routine() {
    if std::env::var(CHILD_ENV)
        .map(|v| v.is_empty())
        .unwrap_or(true)
    {
        return;
    }
    let repo = PathBuf::from(std::env::var(REPO_ENV).unwrap());
    let ready = PathBuf::from(std::env::var(READY_ENV).unwrap());
    let lock_name = std::env::var(LOCK_NAME_ENV).unwrap_or_else(|_| "shared.lock".into());
    let mode = std::env::var(MODE_ENV).unwrap_or_else(|_| "hold".into());

    if mode == "try" {
        let acquired = WorkflowFileLock::try_acquire(&repo, &lock_name).unwrap();
        let out = if acquired.is_some() {
            "locked"
        } else {
            "unlocked"
        };
        std::fs::write(&ready, out).unwrap();
        return;
    }

    let _lock = WorkflowFileLock::acquire(&repo, &lock_name).unwrap();
    std::fs::write(&ready, "locked").unwrap();
    let release = PathBuf::from(std::env::var(RELEASE_ENV).unwrap());
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    while !release.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "child timed out waiting for release signal"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
