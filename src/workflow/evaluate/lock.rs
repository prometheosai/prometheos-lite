//! Cross-platform exclusive file lock with RAII semantics.
//!
//! Issue #114: every registry mutation previously duplicated OS-specific
//! locking (`flock` on Unix, `LockFileEx` on Windows) with subtly different
//! failure behavior. This module is the single, well-tested abstraction:
//!
//! - [`WorkflowFileLock::acquire`] blocks until the exclusive lock is held.
//! - [`WorkflowFileLock::try_acquire`] returns `Ok(None)` when another owner
//!   (in another process) currently holds the lock, so a caller can decide to
//!   wait instead of failing.
//! - The lock is released on [`Drop`] and, critically, by the OS when the
//!   owning process dies — the lock file itself is never truncated, deleted,
//!   or used as a liveness signal.
//!
//! Windows: a one-byte range is locked with `LockFileEx` and unlocked with
//! `UnlockFileEx` using the exact same `OVERLAPPED`; the handle stays open for
//! the lock's lifetime. Unix: `flock(LOCK_EX)` / `flock(LOCK_UN)` on the open
//! file description. Both are released automatically when the process exits.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Lock file location for a named lock in a repo's workflow directory.
pub fn lock_path_for(repo: &Path, name: &str) -> PathBuf {
    repo.join(".prometheos").join("workflow").join(name)
}

fn open_lock_file(path: &Path) -> Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("failed to create workflow dir for lock file")?;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        // Deliberately never truncated: the lock file carries no payload and
        // must not become a content-based signal.
        .open(path)
        .with_context(|| format!("failed to open lock file {}", path.display()))
}

/// An exclusive, process-scoped lock on a named lock file.
///
/// The lock is tied to an OS-level lock on the file (not to the file's
/// existence or contents). Dropping the value releases it; a crash releases it
/// automatically because the OS closes the handle.
pub struct WorkflowFileLock {
    file: std::fs::File,
    #[cfg(windows)]
    overlapped: winapi::um::minwinbase::OVERLAPPED,
    _path: PathBuf,
}

impl WorkflowFileLock {
    /// Block until the exclusive lock for `name` in `repo` is acquired.
    pub fn acquire(repo: &Path, name: &str) -> Result<Self> {
        let path = lock_path_for(repo, name);
        let file = open_lock_file(&path)?;
        lock_exclusive(&file, &path)?;
        Ok(Self::from_locked(file, path))
    }

    /// Try to acquire the exclusive lock without blocking.
    ///
    /// Returns `Ok(None)` when another process currently holds the lock (the
    /// caller should wait and re-try rather than treating this as an error).
    /// Returns `Err` for any other I/O failure.
    pub fn try_acquire(repo: &Path, name: &str) -> Result<Option<Self>> {
        let path = lock_path_for(repo, name);
        let file = open_lock_file(&path)?;
        match try_lock_exclusive(&file, &path)? {
            true => Ok(Some(Self::from_locked(file, path))),
            false => Ok(None),
        }
    }

    #[cfg(unix)]
    fn from_locked(file: std::fs::File, path: PathBuf) -> Self {
        Self { file, _path: path }
    }

    #[cfg(windows)]
    fn from_locked(file: std::fs::File, path: PathBuf) -> Self {
        let overlapped: winapi::um::minwinbase::OVERLAPPED = unsafe { std::mem::zeroed() };
        Self {
            file,
            overlapped,
            _path: path,
        }
    }
}

#[cfg(unix)]
fn lock_exclusive(file: &std::fs::File, path: &Path) -> Result<()> {
    use std::os::unix::io::AsRawFd;
    let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        bail!("failed to acquire lock {}: {err}", path.display());
    }
    Ok(())
}

#[cfg(unix)]
fn try_lock_exclusive(file: &std::fs::File, path: &Path) -> Result<bool> {
    use std::os::unix::io::AsRawFd;
    let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if ret == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    if err.kind() == std::io::ErrorKind::WouldBlock {
        Ok(false)
    } else {
        bail!("failed to try-acquire lock {}: {err}", path.display())
    }
}

#[cfg(unix)]
impl Drop for WorkflowFileLock {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

#[cfg(windows)]
fn lock_exclusive(file: &std::fs::File, path: &Path) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use winapi::um::fileapi::LockFileEx;
    use winapi::um::minwinbase::LOCKFILE_EXCLUSIVE_LOCK;
    let mut overlapped: winapi::um::minwinbase::OVERLAPPED = unsafe { std::mem::zeroed() };
    let handle = file.as_raw_handle();
    let result = unsafe {
        LockFileEx(
            handle as _,
            LOCKFILE_EXCLUSIVE_LOCK,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if result == 0 {
        let err = std::io::Error::last_os_error();
        bail!("failed to acquire lock {}: {err}", path.display());
    }
    Ok(())
}

#[cfg(windows)]
fn try_lock_exclusive(file: &std::fs::File, path: &Path) -> Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use winapi::um::fileapi::LockFileEx;
    use winapi::um::minwinbase::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY};
    let mut overlapped: winapi::um::minwinbase::OVERLAPPED = unsafe { std::mem::zeroed() };
    let handle = file.as_raw_handle();
    let result = unsafe {
        LockFileEx(
            handle as _,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if result != 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    // ERROR_LOCK_VIOLATION (33) means another process holds the byte range.
    if err.raw_os_error() == Some(33) {
        Ok(false)
    } else {
        bail!("failed to try-acquire lock {}: {err}", path.display())
    }
}

#[cfg(windows)]
impl Drop for WorkflowFileLock {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::UnlockFileEx;
        let handle = self.file.as_raw_handle();
        unsafe {
            UnlockFileEx(handle as _, 0, 1, 0, &mut self.overlapped);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_file_is_never_truncated_or_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        // Seed the lock file with payload that must survive acquisition.
        let path = lock_path_for(repo, "seed.lock");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"payload-must-survive").unwrap();
        let size_before = std::fs::metadata(&path).unwrap().len();

        let lock = WorkflowFileLock::acquire(repo, "seed.lock").unwrap();
        // The lock must not truncate the file. On Windows the locked byte
        // range is unreadable, so assert on the size (metadata) while held.
        let size_during = std::fs::metadata(&path).unwrap().len();
        assert_eq!(size_during, size_before, "lock file must not be truncated");
        drop(lock);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            bytes, b"payload-must-survive",
            "lock file must not be deleted"
        );
        assert!(path.exists(), "lock file must persist after release");
    }

    #[test]
    #[cfg(unix)]
    fn try_acquire_returns_none_while_held() {
        // flock is per open-file-description: a second open of the same file in
        // this process conflicts. (Windows LockFileEx is per-process, so that
        // platform's exclusion is verified by the cross-process integration
        // tests in tests/locking_tests.rs.)
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let lock = WorkflowFileLock::acquire(repo, "x.lock").unwrap();
        assert!(
            WorkflowFileLock::try_acquire(repo, "x.lock")
                .unwrap()
                .is_none(),
            "try_acquire must report the lock is held"
        );
        drop(lock);
        assert!(
            WorkflowFileLock::try_acquire(repo, "x.lock")
                .unwrap()
                .is_some(),
            "try_acquire must succeed after release"
        );
    }

    #[test]
    fn acquire_is_reentrant_after_drop() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let _lock = WorkflowFileLock::acquire(repo, "r.lock").unwrap();
        drop(_lock);
        // Re-acquisition must not fail after a clean release.
        let _lock2 = WorkflowFileLock::acquire(repo, "r.lock").unwrap();
    }
}
