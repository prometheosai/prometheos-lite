//! Cross-process workspace ownership tests (issue #171, FIX 2).
//!
//! `GitWorktreeWorkspace` ownership must be exclusive across PROCESSES, not
//! just threads: an OS-backed lock (`flock` / `LockFileEx`) guards the claim,
//! so two racing processes recovering a stale (crashed-owner) workspace get
//! exactly one winner. The lock file is never deleted or interpreted, so a
//! dropped handle can never remove a successor's claim.
//!
//! Each child runs the same test binary via `--exact`, following the pattern
//! of `tests/locking_tests.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use prometheos_lite::workflow::workspace::{
    AdapterKind, GitWorktreeWorkspace, WorkspaceAdapter, WorkspaceManifestV1, WorkspaceMode,
};

const CHILD_ENV: &str = "PROMETHEOS_WS_OWN_CHILD";
const MODE_ENV: &str = "PROMETHEOS_WS_OWN_MODE";
const PARENT_ENV: &str = "PROMETHEOS_WS_OWN_PARENT";
const SOURCE_ENV: &str = "PROMETHEOS_WS_OWN_SOURCE";
const MANIFEST_ENV: &str = "PROMETHEOS_WS_OWN_MANIFEST";
const OUT_ENV: &str = "PROMETHEOS_WS_OWN_OUT";
const RELEASE_ENV: &str = "PROMETHEOS_WS_OWN_RELEASE";

const TIMEOUT: Duration = Duration::from_secs(120);

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Fixture {
    repo: PathBuf,
    parent: PathBuf,
    manifest: WorkspaceManifestV1,
    manifest_path: PathBuf,
    rev: String,
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!(
        "lite-owns-{}-{}-{}",
        tag,
        std::process::id(),
        unique_suffix()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.email", "t@t"]);
    git(&dir, &["config", "user.name", "T"]);
    git(&dir, &["config", "core.autocrlf", "false"]);
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "c1"]);
    let rev_out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let rev = String::from_utf8_lossy(&rev_out.stdout).trim().to_string();
    let parent = dir.join("_ws");
    let manifest = WorkspaceManifestV1 {
        schema_version: prometheos_lite::workflow::workspace::WORKSPACE_SCHEMA_VERSION.into(),
        workspace_id: "ws-own".into(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: prometheos_lite::workflow::workspace::ADAPTER_REVISION.into(),
        repo_identity: dir.to_string_lossy().to_string(),
        base_revision: rev.clone(),
        branch: None,
        mode: WorkspaceMode::Writable,
        writable_scopes: vec!["work://x".into()],
        resource_lock_id: "lock-own".into(),
        created_at: "2026-08-25T00:00:00Z".into(),
        content_digest: None,
    }
    .sealed();
    let manifest_path = dir.join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();
    Fixture {
        repo: dir,
        parent,
        manifest,
        manifest_path,
        rev,
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.repo);
    }
}

fn unique_suffix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn child_command(fix: &Fixture, mode: &str, out: &Path, release: &Path) -> Command {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.arg("--exact").arg("child_workspace_routine");
    cmd.arg("--test-threads=1").arg("--nocapture");
    cmd.env(CHILD_ENV, "1");
    cmd.env(MODE_ENV, mode);
    cmd.env(PARENT_ENV, &fix.parent);
    cmd.env(SOURCE_ENV, &fix.repo);
    cmd.env(MANIFEST_ENV, &fix.manifest_path);
    cmd.env(OUT_ENV, out);
    cmd.env(RELEASE_ENV, release);
    cmd
}

fn wait_for(path: &Path) -> bool {
    let start = std::time::Instant::now();
    while !path.exists() {
        if start.elapsed() > TIMEOUT {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

fn adapter(fix: &Fixture) -> GitWorktreeWorkspace {
    GitWorktreeWorkspace {
        parent_dir: fix.parent.clone(),
        source_repo: fix.repo.clone(),
    }
}

/// FIX 2 regression: several processes race to recover a stale workspace
/// (owner crashed: lock file + worktree remain, no OS lock held). Exactly
/// one must win; the rest must be refused. No stale-lock deletion happens.
#[test]
fn stale_workspace_race_exactly_one_winner() {
    let fix = fixture("race");
    let adapter = adapter(&fix);
    // Create the workspace, then abandon it like a crashed owner would.
    let first = adapter
        .acquire(&fix.manifest)
        .expect("setup acquire must succeed");
    let root = first.root.clone();
    drop(first);
    let lock_file = fix.parent.join("ws-own").join("owner.lock");
    assert!(
        lock_file.exists(),
        "stale lock file must remain after crash"
    );
    assert!(root.exists(), "stale worktree must remain after crash");

    let release = fix.repo.join("release");
    let mut children = Vec::new();
    let mut outs = Vec::new();
    for i in 0..4 {
        let out = fix.repo.join(format!("out-{i}"));
        let child = child_command(&fix, "hold", &out, &release).spawn().unwrap();
        children.push(child);
        outs.push(out);
    }

    let mut acquired = 0;
    let mut refused = 0;
    for out in &outs {
        assert!(wait_for(out), "child never reported a result");
        let content = std::fs::read_to_string(out).unwrap();
        match content.as_str() {
            "acquired" => acquired += 1,
            s if s.starts_with("refused") => refused += 1,
            other => panic!("unexpected child result: {other}"),
        }
    }
    assert_eq!(acquired, 1, "exactly one process may own the workspace");
    assert_eq!(refused, 3, "all other racers must be refused");

    // The winner's workspace is intact and the lock file was never deleted.
    assert!(root.exists());
    assert!(lock_file.exists(), "lock file must never be deleted");

    std::fs::write(&release, "go").unwrap();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }

    // After release, a fresh acquisition succeeds: the lock stays exclusive
    // across drop/reacquire, and the lock file persists.
    let recovered = adapter
        .acquire(&fix.manifest)
        .expect("acquisition after release must succeed");
    assert_eq!(recovered.revision, fix.rev);
    adapter
        .cleanup(
            recovered,
            &prometheos_lite::workflow::workspace::PreservationSet::default(),
            &fix.repo.join("_ev"),
        )
        .expect("cleanup");
    assert!(lock_file.exists(), "cleanup must not delete the lock file");
}

/// FIX 1/FIX 2: cross-process release + reacquire stays exclusive, and a
/// dropped handle cannot remove a successor's claim.
#[test]
fn cross_process_release_reacquire_exclusive_drop_identity() {
    let fix = fixture("hold");
    let adapter = adapter(&fix);
    let out1 = fix.repo.join("out1");
    let release1 = fix.repo.join("release1");
    let mut holder = child_command(&fix, "hold", &out1, &release1)
        .spawn()
        .unwrap();
    assert!(wait_for(&out1), "holder child never acquired");
    assert_eq!(std::fs::read_to_string(&out1).unwrap(), "acquired");

    // Parent process must be refused while the child holds the workspace.
    let err = adapter.acquire(&fix.manifest).unwrap_err();
    assert!(
        err.to_string().contains("concurrent"),
        "parent must refuse while child holds, got: {err}"
    );

    // Child releases; parent now acquires the stale workspace.
    std::fs::write(&release1, "go").unwrap();
    assert!(holder.wait().unwrap().success());
    let parent_handle = adapter
        .acquire(&fix.manifest)
        .expect("parent must acquire after child release");
    assert_eq!(parent_handle.revision, fix.rev);

    // While the parent holds it, a second child must be refused.
    let out2 = fix.repo.join("out2");
    let release2 = fix.repo.join("release2");
    let mut second = child_command(&fix, "hold", &out2, &release2)
        .spawn()
        .unwrap();
    assert!(wait_for(&out2), "second child never reported");
    assert!(
        std::fs::read_to_string(&out2)
            .unwrap()
            .starts_with("refused"),
        "child must refuse while parent holds"
    );
    assert!(second.wait().unwrap().success());

    // Parent drops its handle. The lock file still exists, and the dropped
    // handle must not have removed the successor's claim: only NOW does a
    // fresh child acquire.
    let lock_file = fix.parent.join("ws-own").join("owner.lock");
    drop(parent_handle);
    assert!(lock_file.exists(), "drop must not delete the lock file");

    let out3 = fix.repo.join("out3");
    let release3 = fix.repo.join("release3");
    let mut third = child_command(&fix, "hold", &out3, &release3)
        .spawn()
        .unwrap();
    assert!(wait_for(&out3), "third child never acquired");
    assert_eq!(std::fs::read_to_string(&out3).unwrap(), "acquired");

    // While the successor (third child) holds, the parent must be refused:
    // the first drop did not and cannot remove the successor's claim.
    let err = adapter.acquire(&fix.manifest).unwrap_err();
    assert!(
        err.to_string().contains("concurrent"),
        "parent must refuse while successor holds, got: {err}"
    );

    std::fs::write(&release3, "go").unwrap();
    assert!(third.wait().unwrap().success());
    assert!(
        lock_file.exists(),
        "lock file must persist for the next owner"
    );
}

/// Child routine executed by spawned test binaries. No-op on a normal run.
#[test]
fn child_workspace_routine() {
    if std::env::var(CHILD_ENV)
        .map(|v| v.is_empty())
        .unwrap_or(true)
    {
        return;
    }
    let parent = PathBuf::from(std::env::var(PARENT_ENV).unwrap());
    let source = PathBuf::from(std::env::var(SOURCE_ENV).unwrap());
    let manifest_path = PathBuf::from(std::env::var(MANIFEST_ENV).unwrap());
    let out = PathBuf::from(std::env::var(OUT_ENV).unwrap());
    let release = PathBuf::from(std::env::var(RELEASE_ENV).unwrap());
    let mode = std::env::var(MODE_ENV).unwrap_or_else(|_| "hold".into());

    let manifest =
        WorkspaceManifestV1::parse_json(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let adapter = GitWorktreeWorkspace {
        parent_dir: parent,
        source_repo: source,
    };
    match adapter.acquire(&manifest) {
        Ok(_handle) => {
            std::fs::write(&out, "acquired").unwrap();
            if mode == "hold" {
                let start = std::time::Instant::now();
                while !release.exists() {
                    assert!(
                        start.elapsed() < TIMEOUT,
                        "child timed out waiting for release"
                    );
                    std::thread::sleep(Duration::from_millis(20));
                }
                std::fs::write(&out, "released").unwrap();
            }
        }
        Err(e) => {
            std::fs::write(&out, format!("refused: {e}")).unwrap();
        }
    }
}
