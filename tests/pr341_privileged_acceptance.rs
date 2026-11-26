//! PR341 Stage 1 real privileged acceptance target.
//! Windows guest only: executes genuine production owner_registry APIs against
//! the guest's OS-managed ProgramData root. No simulated SIDs or JSON-only
//! positive provisioning.

#![cfg(windows)]

use prometheos_lite::work::owner_registry::windows::{
    acquire_test_mutex, load_registry, provision_registry, reject_reparse,
    require_provisioning_authority, resolve_owner, test_mutex_name, validate_file_security,
    validate_registry_location,
};
use prometheos_lite::work::owner_registry::{
    OWNER_REGISTRY_SCHEMA, OwnerBinding, OwnerRegistry, canonical_json, default_registry_path,
    enforce_monotonic_update, is_well_formed_sid, parse_registry_bytes, programdata_root,
};
use std::path::{Path, PathBuf};

const SID_A: &str = "S-1-5-21-1111111111-2222222222-3333333333-1001";
const SID_B: &str = "S-1-5-21-1111111111-2222222222-3333333333-1002";

struct OwnedRegistry {
    path: PathBuf,
    absent_dirs: Vec<PathBuf>,
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
        assert!(
            !path.exists(),
            "pre-existing owner registry is not test-owned"
        );

        let mut absent_dirs = Vec::new();
        let mut cursor = path.parent();
        while let Some(dir) = cursor {
            if !dir.exists() {
                absent_dirs.push(dir.to_path_buf());
            }
            cursor = dir.parent();
        }
        Self { path, absent_dirs }
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
}

impl Drop for OwnedRegistry {
    fn drop(&mut self) {
        // Remove only the leaf this test created, then only ancestor directories
        // that were demonstrably absent before provisioning.
        if self.path.exists() {
            let _ = std::fs::remove_file(&self.path);
        }
        for dir in self.absent_dirs.iter().rev() {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

#[test]
fn elevated_first_install_exact_bytes_and_trust() {
    let owned = OwnedRegistry::fresh();
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
    // Cleanup guard removes the test-owned leaf after this assertion.
}

#[test]
fn monotonic_update_and_permanent_revocation() {
    let owned = OwnedRegistry::fresh();
    let first = owned.registry(
        1,
        vec![
            OwnedRegistry::binding(SID_A, "owner-a", false, 1),
            OwnedRegistry::binding(SID_B, "owner-b", false, 1),
        ],
    );
    provision_registry(&owned.path, &first).unwrap();

    let second = owned.registry(
        2,
        vec![
            OwnedRegistry::binding(SID_A, "owner-a", false, 1),
            OwnedRegistry::binding(SID_B, "owner-b", true, 2),
        ],
    );
    provision_registry(&owned.path, &second).unwrap();
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

    // Reparse/junction substitution on a guest-local disposable path.
    let base = programdata_root()
        .unwrap()
        .join("PrometheOS-Lite-PR341-Negative");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let link = base.join("junction");
    let target = base.join("target");
    std::fs::create_dir(&target).unwrap();
    std::os::windows::fs::symlink_dir(&target, &link).unwrap();
    assert!(reject_reparse(&link).is_err(), "junction must be refused");
    let _ = std::fs::remove_dir(&link);
    let _ = std::fs::remove_dir(&target);
    let _ = std::fs::remove_dir(&base);
}

#[test]
fn concurrent_provisioners_serialize_and_cleanup_is_verified() {
    use std::sync::Arc;
    let owned = OwnedRegistry::fresh();
    let name = test_mutex_name(&owned.path);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let counters = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let name_a = name.clone();
    let name_b = name.clone();
    let barrier_a = barrier.clone();
    let barrier_b = barrier.clone();
    let counters_a = counters.clone();
    let counters_b = counters.clone();
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let contender = move |name: String,
                          barrier: Arc<std::sync::Barrier>,
                          counters: Arc<std::sync::atomic::AtomicUsize>,
                          tx: std::sync::mpsc::Sender<Result<(), String>>| {
        // Both contenders must reach the barrier BEFORE either takes the named
        // mutex. Holding the mutex while waiting for the partner deadlocks:
        // the partner cannot reach the barrier until the mutex is released.
        barrier.wait();
        let guard = match acquire_test_mutex(&name) {
            Ok(guard) => guard,
            Err(error) => {
                let _ = tx.send(Err(format!("mutex acquisition refused: {error:?}")));
                return;
            }
        };
        counters.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(25));
        let observed = counters.load(std::sync::atomic::Ordering::SeqCst);
        drop(guard);
        let _ = tx.send(if observed == 1 {
            Ok(())
        } else {
            Err(format!(
                "two contenders entered the critical section: {observed}"
            ))
        });
    };
    let tx_a = tx.clone();
    let a = std::thread::spawn(move || contender(name_a, barrier_a, counters_a, tx_a));
    let tx_b = tx.clone();
    let b = std::thread::spawn(move || contender(name_b, barrier_b, counters_b, tx_b));
    drop(tx);
    for _ in 0..2 {
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("bounded watchdog: contender did not finish")
            .expect("contender failed");
    }
    a.join().expect("contender thread panicked");
    b.join().expect("contender thread panicked");
    assert!(
        !owned.path.exists(),
        "test-owned registry must be removed on drop"
    );
}
