#![cfg(windows)]

//! Stage 2 privileged-acceptance evidence for PR #340/#341.
//!
//! OPT-IN ONLY. These tests write to the machine-local ProgramData registry
//! path used by the trusted owner registry. They must never run on the
//! owner's physical machine. Enable with `PR341_STAGE2_ACCEPTANCE=1` on a
//! fresh, disposable, GitHub-hosted `windows-2022` runner only.
//!
//! A missing opt-in is reported as `STAGE2_NOT_ENABLED`, never as a pass.
//! This target is not invoked by the merged Stage 1 workflow; enabling it in
//! CI requires separate workflow review, a review-pin update, policy
//! regression coverage and explicit Principal authorization.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use prometheos_lite::work::owner_registry as registry;
use registry::windows::{
    acquire_test_mutex, dacl_grants_full_control_to, load_registry, provision_registry,
    reject_reparse, require_provisioning_authority, resolve_owner, test_mutex_name,
    validate_file_security, validate_registry_location,
};
use registry::{OwnerBinding, OwnerRegistry, Refusal, SID_ADMINISTRATORS, SID_EVERYONE};

const OPT_IN: &str = "PR341_STAGE2_ACCEPTANCE";

fn enabled() -> bool {
    std::env::var_os(OPT_IN).as_deref() == Some(OsStr::new("1"))
}

macro_rules! require_stage2 {
    () => {
        if !enabled() {
            eprintln!("STAGE2_NOT_ENABLED: set {OPT_IN}=1 on a disposable GitHub-hosted runner");
            return;
        }
    };
}

fn binding(sid: &str, owner: &str, revoked: bool, revision: u64) -> OwnerBinding {
    OwnerBinding {
        sid: sid.to_owned(),
        owner: owner.to_owned(),
        revoked,
        revision,
    }
}

fn document(generation: u64, bindings: Vec<OwnerBinding>) -> OwnerRegistry {
    OwnerRegistry {
        schema_version: registry::OWNER_REGISTRY_SCHEMA.to_owned(),
        generation,
        bindings,
    }
}

/// Fail closed unless this really is a standard GitHub-hosted runner with an
/// OS-resolved, non-reparse ProgramData root. No host-mounted protected state.
fn preflight() -> PathBuf {
    assert_eq!(
        std::env::var("GITHUB_ACTIONS").as_deref(),
        Ok("true"),
        "Stage 2 requires a GitHub Actions runner"
    );
    let environment = std::env::var("RUNNER_ENVIRONMENT").unwrap_or_default();
    assert!(
        environment.eq_ignore_ascii_case("github-hosted"),
        "Stage 2 requires a standard GitHub-hosted runner, found {environment:?}"
    );
    let root = registry::programdata_root().expect("ProgramData Known Folder must resolve");
    assert!(root.is_absolute(), "ProgramData root must be absolute");
    assert!(
        reject_reparse(&root).is_ok(),
        "ProgramData root must not be a reparse point"
    );
    let path = registry::default_registry_path().expect("trusted registry path must resolve");
    assert!(
        path.starts_with(&root),
        "registry path must be under ProgramData"
    );
    assert_eq!(
        path.file_name().and_then(OsStr::to_str),
        Some(registry::REGISTRY_FILE_NAME),
        "registry leaf name must be pinned"
    );
    path
}

/// Deletes only objects this test created: the registry leaf and ancestors
/// that did not exist before provisioning. Never touches pre-existing state.
struct OwnedRegistry {
    path: PathBuf,
    leaf_existed: bool,
    new_dirs: Vec<PathBuf>,
}

impl OwnedRegistry {
    fn new(path: &Path) -> Self {
        let leaf_existed = path.exists();
        let root = registry::programdata_root().expect("ProgramData root");
        let mut new_dirs = Vec::new();
        let mut current = path.parent().expect("registry parent");
        while current != root {
            if !current.exists() {
                new_dirs.push(current.to_path_buf());
            }
            match current.parent() {
                Some(parent) => current = parent,
                None => break,
            }
        }
        Self {
            path: path.to_path_buf(),
            leaf_existed,
            new_dirs,
        }
    }
}

impl Drop for OwnedRegistry {
    fn drop(&mut self) {
        if !self.leaf_existed {
            let _ = fs::remove_file(&self.path);
        }
        for dir in self.new_dirs.iter().rev() {
            let _ = fs::remove_dir(dir);
        }
    }
}

fn grant_everyone_full_control(path: &Path) {
    let sid = format!("*{SID_EVERYONE}:(OI)(CI)F");
    let output = Command::new("icacls")
        .arg(path)
        .arg("/grant")
        .arg(sid)
        .output()
        .expect("icacls must be available on the hosted Windows runner");
    assert!(
        output.status.success(),
        "ACL setup must fail the test, never skip: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn stage2_preflight_proves_elevated_token_and_isolated_programdata() {
    require_stage2!();
    let path = preflight();
    let authority = require_provisioning_authority()
        .expect("Stage 2 first-install requires a real elevated provisioning token");
    assert!(
        authority.contains(SID_ADMINISTRATORS) || authority.contains("S-1-5-18"),
        "provisioning authority must be Administrators or SYSTEM: {authority}"
    );
    assert_eq!(
        path.file_name().unwrap(),
        OsStr::new(registry::REGISTRY_FILE_NAME)
    );
}

#[test]
fn stage2_first_install_installs_protected_owner_and_dacl() {
    require_stage2!();
    let path = preflight();
    let _owned = OwnedRegistry::new(&path);
    let sid = SID_ADMINISTRATORS;
    let proposed = document(1, vec![binding(sid, "stage2-owner", false, 1)]);
    provision_registry(&path, &proposed)
        .expect("elevated first install must succeed on a disposable hosted runner");

    let installed = load_registry(&path).expect("installed registry must load");
    assert_eq!(
        installed, proposed,
        "installed bytes must be exactly the proposal"
    );
    assert!(
        validate_registry_location(&path).is_ok(),
        "installed location and leaf must pass protected-location validation"
    );
    assert_eq!(
        dacl_grants_full_control_to(&path, sid),
        Ok(true),
        "trusted owner must hold full control on the installed registry"
    );
    assert_eq!(
        dacl_grants_full_control_to(&path, SID_EVERYONE),
        Ok(false),
        "Everyone must not hold full control on a protected registry"
    );
    let entries: Vec<_> = fs::read_dir(path.parent().unwrap())
        .expect("parent directory")
        .filter_map(Result::ok)
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "atomic publication must leave no temp files behind"
    );
}

#[test]
fn stage2_monotonic_update_and_permanent_revocation() {
    require_stage2!();
    let path = preflight();
    let _owned = OwnedRegistry::new(&path);
    let sid = SID_ADMINISTRATORS;
    let first = document(1, vec![binding(sid, "stage2-owner", false, 1)]);
    provision_registry(&path, &first).expect("first install");

    let revoked = document(2, vec![binding(sid, "stage2-owner", true, 2)]);
    provision_registry(&path, &revoked).expect("monotonic revocation update");
    assert_eq!(
        resolve_owner(&path, sid),
        Ok(None),
        "a revoked binding must be a permanent miss"
    );

    assert_eq!(
        provision_registry(&path, &first),
        Err(Refusal::NonMonotonicUpdate),
        "a stale generation must be refused"
    );
    let unrevoked = document(3, vec![binding(sid, "stage2-owner", false, 3)]);
    assert_eq!(
        provision_registry(&path, &unrevoked),
        Err(Refusal::NonMonotonicUpdate),
        "revocation must not be reversible by a later proposal"
    );
}

#[test]
fn stage2_untrusted_parent_and_junction_are_refused() {
    require_stage2!();
    let _ = preflight();
    let base = PathBuf::from(std::env::var("RUNNER_TEMP").expect("RUNNER_TEMP"));
    let dir = base.join(format!("pr341-stage2-acl-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create disposable ACL fixture");
    grant_everyone_full_control(&dir);
    assert!(
        validate_file_security(&dir).is_err(),
        "a world-writable ancestor must be refused with real OS ACLs"
    );

    let target = dir.join("target");
    let junction = dir.join("junction");
    fs::create_dir_all(&target).expect("create junction target");
    std::os::windows::fs::symlink_dir(&target, &junction)
        .expect("create junction on hosted runner");
    assert!(
        reject_reparse(&junction.join(registry::REGISTRY_FILE_NAME)).is_err(),
        "a junction substitution must be refused"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn stage2_concurrent_provisioners_share_one_lock() {
    require_stage2!();
    let path = preflight();
    let name = test_mutex_name(&path);
    let first_name = name.clone();
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let holder = thread::spawn(move || {
        let _guard = acquire_test_mutex(&first_name).expect("first lock");
        held_tx.send(()).expect("signal held");
        let _ = release_rx.recv();
    });
    held_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("first lock acquired");

    let (contender_tx, contender_rx) = mpsc::channel();
    let second_name = name;
    let contender = thread::spawn(move || {
        let _guard = acquire_test_mutex(&second_name).expect("second lock");
        contender_tx.send(()).expect("signal contender acquired");
    });
    assert!(
        contender_rx
            .recv_timeout(Duration::from_millis(300))
            .is_err(),
        "second provisioner must not enter while the first holds the lock"
    );
    release_tx.send(()).expect("release first lock");
    contender_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("second lock acquired after release");
    holder.join().expect("holder thread");
    contender.join().expect("contender thread");
}

#[test]
fn stage2_no_cross_owner_authorization() {
    require_stage2!();
    let path = preflight();
    let _owned = OwnedRegistry::new(&path);
    let owner_sid = SID_ADMINISTRATORS;
    let proposed = document(1, vec![binding(owner_sid, "stage2-owner", false, 1)]);
    provision_registry(&path, &proposed).expect("first install");
    assert_eq!(
        resolve_owner(&path, owner_sid),
        Ok(Some("stage2-owner".to_owned()))
    );
    assert_eq!(
        resolve_owner(&path, SID_EVERYONE),
        Ok(None),
        "an unmapped SID must never inherit another owner's binding"
    );
    assert!(
        resolve_owner(&path, "S-1-1-999999999999999999999999").is_err(),
        "a malformed SID must not resolve"
    );
}

#[test]
fn stage2_cleanup_removes_only_test_owned_resources() {
    require_stage2!();
    let path = preflight();
    let sentinel_dir = PathBuf::from(std::env::var("RUNNER_TEMP").expect("RUNNER_TEMP"));
    let sentinel = sentinel_dir.join("pr341-stage2-unowned-sentinel");
    fs::write(&sentinel, b"do not delete").expect("create unowned sentinel");

    {
        let _owned = OwnedRegistry::new(&path);
        let proposed = document(
            1,
            vec![binding(SID_ADMINISTRATORS, "stage2-owner", false, 1)],
        );
        provision_registry(&path, &proposed).expect("first install");
        assert!(path.exists(), "registry installed before cleanup");
    }

    assert!(
        !path.exists(),
        "owned registry leaf must be removed on cleanup"
    );
    assert!(
        sentinel.exists(),
        "cleanup must not delete unowned resources"
    );
    let _ = fs::remove_file(&sentinel);
}
