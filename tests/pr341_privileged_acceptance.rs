//! PR341 Stage 1 real privileged acceptance target.
//! Windows guest only: executes genuine production owner_registry APIs against
//! the guest's OS-managed ProgramData root. No simulated SIDs or JSON-only
//! positive provisioning.

#![cfg(windows)]

use prometheos_lite::work::owner_registry::windows::{
    acquire_test_mutex, load_registry, provision_registry, reject_reparse,
    require_provisioning_authority, resolve_owner, test_mutex_name, validate_file_security,
    validate_registry_leaf, validate_registry_location,
};
use prometheos_lite::work::owner_registry::{
    OWNER_REGISTRY_SCHEMA, OwnerBinding, OwnerRegistry, canonical_json, default_registry_path,
    enforce_monotonic_update, is_well_formed_sid, parse_registry_bytes, programdata_root,
};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SID_A: &str = "S-1-5-21-1111111111-2222222222-3333333333-1001";
const SID_B: &str = "S-1-5-21-1111111111-2222222222-3333333333-1002";

/// Identity of the exact leaf object this run created. Every field is checked
/// again before deletion; a substituted or unexplained object is preserved.
struct LeafIdentity {
    created: SystemTime,
    bytes: Vec<u8>,
}

struct OwnedRegistry {
    path: PathBuf,
    absent_dirs: Vec<PathBuf>,
    identity: Option<LeafIdentity>,
    cleanup_attempted: bool,
}

impl OwnedRegistry {
    fn fresh() -> Self {
        let root = programdata_root().expect("OS Known Folder ProgramData must resolve");
        assert!(
            root.to_string_lossy()
                .eq_ignore_ascii_case("C:\\ProgramData"),
            "unexpected guest ProgramData root: {root:?}"
        );
        let path = default_registry_path().expect("protected registry path must resolve");
        assert!(
            path.starts_with(&root),
            "registry path escapes guest ProgramData"
        );
        require_provisioning_authority().expect("elevated Windows authority required");
        // `exists()` follows reparse points; a dangling one would look absent.
        assert!(
            std::fs::symlink_metadata(&path).is_err(),
            "pre-existing owner registry (including dangling reparse) is not test-owned: {path:?}"
        );

        let mut absent_dirs = Vec::new();
        let mut cursor = path.parent();
        while let Some(dir) = cursor {
            match std::fs::symlink_metadata(dir) {
                Ok(metadata) => {
                    assert!(
                        metadata.is_dir() && !metadata.file_type().is_symlink(),
                        "protected ancestor is not a plain directory: {dir:?}"
                    );
                    reject_reparse(dir).expect("trusted ancestor without reparse substitution");
                    break;
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    absent_dirs.push(dir.to_path_buf());
                }
                Err(error) => panic!("cannot inspect protected ancestor {dir:?}: {error}"),
            }
            cursor = dir.parent();
        }
        Self {
            path,
            absent_dirs,
            identity: None,
            cleanup_attempted: false,
        }
    }

    fn registry(&self, generation: u64, bindings: Vec<OwnerBinding>) -> OwnerRegistry {
        OwnerRegistry {
            schema_version: OWNER_REGISTRY_SCHEMA.to_string(),
            generation,
            bindings,
        }
    }

    fn binding(sid: &str, owner: &str, revoked: bool, revision: u64) -> OwnerBinding {
        OwnerBinding {
            sid: sid.to_string(),
            owner: owner.to_string(),
            revoked,
            revision,
        }
    }

    /// Bind cleanup to the object this run actually installed: strict trusted
    /// security, no reparse, exact canonical bytes, and Windows file identity.
    fn observe(&mut self, expected: &OwnerRegistry) {
        reject_reparse(&self.path).expect("test-installed leaf must not be a reparse point");
        validate_registry_leaf(&self.path).expect("test-installed leaf must be trusted");
        let metadata = std::fs::symlink_metadata(&self.path).expect("test-installed leaf metadata");
        assert!(metadata.is_file(), "leaf is no longer a regular file");
        assert!(
            !metadata.file_type().is_symlink(),
            "leaf was substituted by a reparse point"
        );
        let bytes = std::fs::read(&self.path).expect("test-installed leaf bytes");
        assert_eq!(
            bytes,
            canonical_json(expected).expect("canonical expected bytes"),
            "leaf content differs from the registry this run installed"
        );
        self.identity = Some(LeafIdentity {
            created: metadata.created().expect("leaf creation time"),
            bytes,
        });
    }

    fn identity_still_matches(&self, identity: &LeafIdentity) -> Result<(), String> {
        reject_reparse(&self.path).map_err(|e| format!("reparse check: {e:?}"))?;
        validate_registry_leaf(&self.path).map_err(|e| format!("leaf security: {e:?}"))?;
        let metadata =
            std::fs::symlink_metadata(&self.path).map_err(|e| format!("metadata: {e}"))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("leaf is no longer the test-created regular file".to_string());
        }
        if metadata
            .created()
            .map_err(|e| format!("created time: {e}"))?
            != identity.created
        {
            return Err("creation time changed: substituted object".to_string());
        }
        if metadata.len() != identity.bytes.len() as u64
            || std::fs::read(&self.path).map_err(|e| format!("read: {e}"))? != identity.bytes
        {
            return Err("leaf bytes changed: substituted object".to_string());
        }
        Ok(())
    }

    fn cleanup(&mut self) {
        if self.cleanup_attempted {
            return;
        }
        self.cleanup_attempted = true;

        match self.identity.take() {
            Some(identity) => match std::fs::symlink_metadata(&self.path) {
                Ok(_) => match self.identity_still_matches(&identity) {
                    Ok(()) => {
                        if let Err(error) = std::fs::remove_file(&self.path) {
                            eprintln!("PR341_CLEANUP_FAILED leaf={:?} error={error}", self.path);
                        }
                    }
                    Err(reason) => {
                        eprintln!(
                            "PR341_CLEANUP_REFUSED leaf={:?} reason={reason}; object preserved",
                            self.path
                        );
                    }
                },
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => eprintln!(
                    "PR341_CLEANUP_FAILED leaf metadata {:?}: {error}",
                    self.path
                ),
            },
            None => eprintln!(
                "PR341_CLEANUP_NO_IDENTITY leaf={:?}; no deletion attempted",
                self.path
            ),
        }

        for dir in self.absent_dirs.iter().rev() {
            match std::fs::symlink_metadata(dir) {
                Ok(metadata) => {
                    if !metadata.is_dir()
                        || metadata.file_type().is_symlink()
                        || reject_reparse(dir).is_err()
                    {
                        eprintln!(
                            "PR341_CLEANUP_REFUSED ancestor={dir:?} substituted; object preserved"
                        );
                        continue;
                    }
                    // remove_dir is non-recursive: an unexplained child blocks it.
                    if let Err(error) = std::fs::remove_dir(dir) {
                        eprintln!("PR341_CLEANUP_FAILED ancestor={dir:?} error={error}");
                    }
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => eprintln!("PR341_CLEANUP_FAILED ancestor metadata {dir:?}: {error}"),
            }
        }
    }

    fn residual(&self) -> Vec<PathBuf> {
        std::iter::once(&self.path)
            .chain(self.absent_dirs.iter())
            .filter(|path| std::fs::symlink_metadata(path).is_ok())
            .cloned()
            .collect()
    }

    /// Explicit cleanup verification. Evidence is printed before any failure,
    /// so it is captured in the guest log before checkpoint recovery.
    fn finish(mut self) {
        self.cleanup();
        let residual = self.residual();
        if !residual.is_empty() {
            eprintln!("PR341_RESIDUAL_STATE preserved_objects={residual:?}");
            panic!("test-owned cleanup incomplete; residual state preserved for evidence");
        }
    }
}

impl Drop for OwnedRegistry {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// Independent, cumulative-free measurements of the shared critical section.
#[derive(Default)]
struct CriticalSectionProbe {
    active: AtomicUsize,
    maximum: AtomicUsize,
    acquisitions: AtomicUsize,
}

impl CriticalSectionProbe {
    fn enter(&self) {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        self.acquisitions.fetch_add(1, Ordering::SeqCst);
    }

    fn leave(&self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Remove only the unique fixture objects this run created. Never recurse and
/// never remove an object whose type changed since creation.
fn remove_negative_fixture(base: &Path, link: &Path, target: &Path) -> bool {
    match std::fs::symlink_metadata(link) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            if let Err(error) = std::fs::remove_dir(link) {
                eprintln!("PR341_CLEANUP_FAILED negative junction {link:?}: {error}");
            }
        }
        Ok(_) => eprintln!("PR341_CLEANUP_REFUSED substituted negative junction {link:?}"),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            eprintln!("PR341_CLEANUP_FAILED negative junction metadata {link:?}: {error}")
        }
    }
    for dir in [target, base] {
        match std::fs::symlink_metadata(dir) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                if let Err(error) = std::fs::remove_dir(dir) {
                    eprintln!("PR341_CLEANUP_FAILED negative fixture dir {dir:?}: {error}");
                }
            }
            Ok(_) => eprintln!("PR341_CLEANUP_REFUSED substituted negative dir {dir:?}"),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => eprintln!("PR341_CLEANUP_FAILED negative dir metadata {dir:?}: {error}"),
        }
    }
    let residual: Vec<&Path> = [link, target, base]
        .into_iter()
        .filter(|path| std::fs::symlink_metadata(path).is_ok())
        .collect();
    if !residual.is_empty() {
        eprintln!("PR341_RESIDUAL_STATE negative_fixture_objects={residual:?}");
    }
    residual.is_empty()
}

#[test]
fn elevated_first_install_exact_bytes_and_trust() {
    let mut owned = OwnedRegistry::fresh();
    let expected = owned.registry(
        1,
        vec![OwnedRegistry::binding(
            SID_A,
            "pr341-first-install",
            false,
            1,
        )],
    );
    provision_registry(&owned.path, &expected).expect("elevated first install must succeed");
    owned.observe(&expected);

    let installed = load_registry(&owned.path).expect("installed registry must load");
    assert_eq!(installed, expected);
    assert_eq!(
        canonical_json(&installed).expect("canonical bytes"),
        canonical_json(&expected).expect("canonical bytes")
    );
    validate_registry_location(&owned.path).expect("trusted owner and DACL");
    validate_file_security(
        owned
            .path
            .parent()
            .expect("registry has a protected parent"),
    )
    .expect("trusted parent ACL");
    reject_reparse(&owned.path).expect("no reparse substitution");
    assert_eq!(
        resolve_owner(&owned.path, SID_A).expect("owner lookup"),
        Some("pr341-first-install".to_string())
    );
    owned.finish();
}

#[test]
fn monotonic_update_and_permanent_revocation() {
    let mut owned = OwnedRegistry::fresh();
    let first = owned.registry(
        1,
        vec![
            OwnedRegistry::binding(SID_A, "owner-a", false, 1),
            OwnedRegistry::binding(SID_B, "owner-b", false, 1),
        ],
    );
    provision_registry(&owned.path, &first).unwrap();
    owned.observe(&first);

    let second = owned.registry(
        2,
        vec![
            OwnedRegistry::binding(SID_A, "owner-a", false, 1),
            OwnedRegistry::binding(SID_B, "owner-b", true, 2),
        ],
    );
    provision_registry(&owned.path, &second).unwrap();
    owned.observe(&second);
    assert_eq!(
        resolve_owner(&owned.path, SID_B).unwrap(),
        None,
        "revoked SID must be a permanent miss"
    );
    assert_eq!(
        resolve_owner(&owned.path, SID_A).unwrap(),
        Some("owner-a".into())
    );

    assert!(
        enforce_monotonic_update(Some(&second), &first).is_err(),
        "generation rollback"
    );
    let reversal = owned.registry(
        3,
        vec![
            OwnedRegistry::binding(SID_A, "owner-a", false, 1),
            OwnedRegistry::binding(SID_B, "owner-b", false, 3),
        ],
    );
    assert!(
        enforce_monotonic_update(Some(&second), &reversal).is_err(),
        "revocation reversal"
    );
    let substituted = owned.registry(
        3,
        vec![
            OwnedRegistry::binding(SID_A, "attacker", false, 2),
            OwnedRegistry::binding(SID_B, "owner-b", true, 2),
        ],
    );
    assert!(
        enforce_monotonic_update(Some(&second), &substituted).is_err(),
        "owner substitution"
    );
    let omitted_tombstone =
        owned.registry(3, vec![OwnedRegistry::binding(SID_A, "owner-a", false, 1)]);
    assert!(
        enforce_monotonic_update(Some(&second), &omitted_tombstone).is_err(),
        "tombstone drop"
    );
    owned.finish();
}

#[test]
fn negative_controls_fail_closed() {
    let _authority = require_provisioning_authority().expect("elevated authority");

    // Unsafe location and malformed/oversized persisted data.
    assert!(
        provision_registry(Path::new("registry.json"), &OwnerRegistry::empty(1)).is_err(),
        "relative registry path must be refused"
    );
    assert!(parse_registry_bytes(b"not json").is_err());
    assert!(parse_registry_bytes(&vec![b'x'; 1_048_577]).is_err());
    assert!(is_well_formed_sid("S-1-5-21-not-a-number"));

    // Reparse/junction substitution on a fixture created exclusively by this
    // run. A pre-existing or substituted path is preserved, never reused.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock after epoch")
        .as_nanos();
    let base = programdata_root().unwrap().join(format!(
        "PrometheOS-Lite-PR341-Negative-{}-{nonce}",
        std::process::id()
    ));
    assert!(
        std::fs::symlink_metadata(&base).is_err(),
        "pre-existing negative-control fixture is not test-owned: {base:?}"
    );
    std::fs::create_dir(&base).expect("exclusive creation of unique negative fixture");
    let link = base.join("junction");
    let target = base.join("target");
    assert!(
        std::fs::symlink_metadata(&link).is_err() && std::fs::symlink_metadata(&target).is_err(),
        "pre-existing negative-control child is not test-owned"
    );
    std::fs::create_dir(&target).expect("test-owned target directory");
    std::os::windows::fs::symlink_dir(&target, &link).expect("test-owned junction");
    let reparse_refused = reject_reparse(&link).is_err();
    let residual_clean = remove_negative_fixture(&base, &link, &target);

    assert!(reparse_refused, "junction must be refused");
    assert!(
        residual_clean,
        "residual negative-control fixture preserved for evidence"
    );
}

#[test]
fn concurrent_provisioners_serialize_and_cleanup_is_verified() {
    let owned = OwnedRegistry::fresh();
    let name = test_mutex_name(&owned.path);
    let barrier = Arc::new(Barrier::new(2));
    let probe = Arc::new(CriticalSectionProbe::default());
    let (tx, rx) = mpsc::channel::<Result<(), String>>();

    let mut handles = Vec::new();
    for _ in 0..2 {
        let name = name.clone();
        let barrier = barrier.clone();
        let probe = probe.clone();
        let tx = tx.clone();
        handles.push(std::thread::spawn(move || {
            // Both contenders reach the barrier BEFORE either takes the named
            // mutex. Holding the mutex while waiting for the partner deadlocks.
            barrier.wait();
            let guard = match acquire_test_mutex(&name) {
                Ok(guard) => guard,
                Err(error) => {
                    let _ = tx.send(Err(format!("mutex acquisition refused: {error:?}")));
                    return;
                }
            };
            probe.enter();
            std::thread::sleep(Duration::from_millis(25));
            probe.leave();
            drop(guard);
            let _ = tx.send(Ok(()));
        }));
    }
    drop(tx);

    let mut failures = Vec::new();
    for index in 0..2 {
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failures.push(format!("contender {index}: {error}")),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                failures.push(format!("contender {index}: watchdog timeout"))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                failures.push(format!("contender {index}: no completion acknowledgment"))
            }
        }
    }
    for (index, handle) in handles.into_iter().enumerate() {
        if handle.join().is_err() {
            failures.push(format!("contender {index}: thread panicked"));
        }
    }
    assert!(
        failures.is_empty(),
        "contender failures: {}",
        failures.join("; ")
    );
    assert_eq!(
        probe.acquisitions.load(Ordering::SeqCst),
        2,
        "both contenders must acquire the mutex exactly once"
    );
    assert_eq!(
        probe.maximum.load(Ordering::SeqCst),
        1,
        "at most one critical-section holder at a time"
    );
    assert_eq!(
        probe.active.load(Ordering::SeqCst),
        0,
        "no critical-section holder may remain after completion"
    );
    owned.finish();
}
