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
use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind, Read};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use winapi::shared::minwindef::{DWORD, TRUE};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::fileapi::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_DISPOSITION_INFO, GetFileInformationByHandle,
    GetFinalPathNameByHandleW, OPEN_EXISTING, SetFileInformationByHandle,
};
use winapi::um::handleapi::CloseHandle;
use winapi::um::minwinbase::FileDispositionInfo;
use winapi::um::winbase::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, VOLUME_NAME_DOS,
};
use winapi::um::winnt::{
    DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, HANDLE,
};

const SID_A: &str = "S-1-5-21-1111111111-2222222222-3333333333-1001";
const SID_B: &str = "S-1-5-21-1111111111-2222222222-3333333333-1002";

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

struct VerifiedHandle(HANDLE);

impl Drop for VerifiedHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Kernel object identity from GetFileInformationByHandle, not a timestamp or
/// content digest. Volume serial + file index + creation time + size bind the
/// deletion to one exact filesystem object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KernelIdentity {
    volume: u32,
    index_high: u32,
    index_low: u32,
    size_high: u32,
    size_low: u32,
    created_high: u32,
    created_low: u32,
}

impl KernelIdentity {
    fn from_info(info: &BY_HANDLE_FILE_INFORMATION) -> Self {
        Self {
            volume: info.dwVolumeSerialNumber,
            index_high: info.nFileIndexHigh,
            index_low: info.nFileIndexLow,
            size_high: info.nFileSizeHigh,
            size_low: info.nFileSizeLow,
            created_high: info.ftCreationTime.dwHighDateTime,
            created_low: info.ftCreationTime.dwLowDateTime,
        }
    }
}

fn open_object(path: &Path, directory: bool, for_delete: bool) -> Result<VerifiedHandle, String> {
    let mut access = FILE_READ_ATTRIBUTES;
    if for_delete {
        access |= DELETE;
    }
    let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
    if directory {
        flags |= FILE_FLAG_BACKUP_SEMANTICS;
    }
    let handle = unsafe {
        CreateFileW(
            wide(path.as_os_str()).as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            flags,
            std::ptr::null_mut(),
        )
    };
    if handle.is_null() {
        return Err(format!("CreateFileW {:?} failed: {}", path, unsafe {
            GetLastError()
        }));
    }
    Ok(VerifiedHandle(handle))
}

fn inspect_object(handle: HANDLE) -> Result<(KernelIdentity, bool, bool), String> {
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(handle, &mut info) } != TRUE {
        return Err(format!("GetFileInformationByHandle failed: {}", unsafe {
            GetLastError()
        }));
    }
    let reparse = info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    let directory = info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    Ok((KernelIdentity::from_info(&info), reparse, directory))
}

/// Final path resolved by the kernel for this handle, so an open handle cannot
/// be silently redirected by a later namespace substitution.
fn handle_final_path(handle: HANDLE) -> Result<PathBuf, String> {
    let mut buffer = vec![0u16; 4096];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            handle,
            buffer.as_mut_ptr(),
            buffer.len() as DWORD,
            VOLUME_NAME_DOS,
        )
    };
    if length == 0 || length as usize >= buffer.len() {
        return Err(format!("GetFinalPathNameByHandleW failed: {}", unsafe {
            GetLastError()
        }));
    }
    Ok(PathBuf::from(OsString::from_wide(
        &buffer[..length as usize],
    )))
}

fn expected_final_path(path: &Path) -> Result<PathBuf, String> {
    // Resolve the parent only: resolving the leaf would follow a reparse point.
    let parent = path
        .parent()
        .ok_or_else(|| format!("no parent for {path:?}"))?;
    let canonical_parent = std::fs::canonicalize(parent)
        .map_err(|error| format!("canonicalize parent {:?}: {error}", parent))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("no file name for {path:?}"))?;
    Ok(canonical_parent.join(name))
}

fn same_path(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

/// Delete only through an already verified kernel handle. There is no second
/// path lookup between verification and deletion, so substitution cannot change
/// which object is removed.
fn delete_verified_path(
    path: &Path,
    expected_identity: Option<&KernelIdentity>,
    expected_reparse: Option<bool>,
    expect_directory: bool,
) -> Result<(), String> {
    let handle = open_object(path, expect_directory, true)?;
    let (identity, reparse, directory) = inspect_object(handle.0)?;
    if expected_identity.is_some_and(|expected| identity != *expected) {
        return Err(format!("kernel identity changed for {path:?}"));
    }
    if expected_reparse.is_some_and(|expected| reparse != expected) {
        return Err(format!("reparse state changed for {path:?}"));
    }
    if directory != expect_directory {
        return Err(format!("object kind changed for {path:?}"));
    }
    if !same_path(&handle_final_path(handle.0)?, &expected_final_path(path)?) {
        return Err(format!("final path changed for {path:?}"));
    }
    let mut disposition = FILE_DISPOSITION_INFO { DeleteFile: 1 };
    if unsafe {
        SetFileInformationByHandle(
            handle.0,
            FileDispositionInfo,
            &mut disposition as *mut _ as *mut winapi::ctypes::c_void,
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as DWORD,
        )
    } != TRUE
    {
        return Err(format!(
            "SetFileInformationByHandle delete failed for {path:?}: {}",
            unsafe { GetLastError() }
        ));
    }
    Ok(())
}

fn ancestry_errors(path: &Path, allow_absent: bool) -> Vec<String> {
    let mut errors = Vec::new();
    let mut cursor = path.parent();
    while let Some(dir) = cursor {
        match std::fs::symlink_metadata(dir) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    errors.push(format!("ancestor is not a plain directory: {dir:?}"));
                }
                if let Err(error) = reject_reparse(dir) {
                    errors.push(format!("ancestor reparse check failed {dir:?}: {error:?}"));
                }
                match open_object(dir, true, false).and_then(|handle| {
                    let (_, reparse, directory) = inspect_object(handle.0)?;
                    if reparse || !directory {
                        return Err(format!("handle describes a substituted ancestor {dir:?}"));
                    }
                    let final_path = handle_final_path(handle.0)?;
                    if !same_path(&final_path, dir) {
                        return Err(format!("ancestor final path changed {dir:?}"));
                    }
                    Ok(())
                }) {
                    Ok(()) => {}
                    Err(error) => errors.push(error),
                }
            }
            Err(error) if allow_absent && error.kind() == ErrorKind::NotFound => {}
            Err(error) => errors.push(format!(
                "unexpected ancestor metadata error {dir:?}: {error}"
            )),
        }
        cursor = dir.parent();
    }
    errors
}

/// Identity of the exact leaf object this run created, plus its expected bytes
/// as content evidence. Deletion uses the kernel identity, never the bytes.
struct LeafIdentity {
    kernel: KernelIdentity,
    bytes: Vec<u8>,
}

struct OwnedRegistry {
    path: PathBuf,
    absent_dirs: Vec<PathBuf>,
    identity: Option<LeafIdentity>,
    dir_identities: Vec<(PathBuf, KernelIdentity)>,
    cleanup_attempted: bool,
    cleanup_errors: Vec<String>,
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
        match std::fs::symlink_metadata(&path) {
            Ok(_) => panic!(
                "pre-existing owner registry (including dangling reparse) is not test-owned: {path:?}"
            ),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => panic!("cannot inspect owner registry {path:?}: {error}"),
        }

        let mut absent_dirs = Vec::new();
        let mut cursor = path.parent();
        while let Some(dir) = cursor {
            match std::fs::symlink_metadata(dir) {
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    absent_dirs.push(dir.to_path_buf());
                }
                Err(error) => panic!("cannot inspect protected ancestor {dir:?}: {error}"),
            }
            cursor = dir.parent();
        }
        let errors = ancestry_errors(&path, true);
        assert!(
            errors.is_empty(),
            "unsafe ancestry before provisioning: {errors:?}"
        );
        Self {
            path,
            absent_dirs,
            identity: None,
            dir_identities: Vec::new(),
            cleanup_attempted: false,
            cleanup_errors: Vec::new(),
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

    /// Capture verified kernel object identities after every provisioning step.
    fn observe(&mut self, expected: &OwnerRegistry) {
        let errors = ancestry_errors(&self.path, false);
        assert!(
            errors.is_empty(),
            "unsafe ancestry after provisioning: {errors:?}"
        );
        reject_reparse(&self.path).expect("test-installed leaf must not be a reparse point");
        validate_registry_leaf(&self.path).expect("test-installed leaf must be trusted");
        let handle = open_object(&self.path, false, true).expect("open installed leaf");
        let (kernel, reparse, directory) = inspect_object(handle.0).expect("leaf kernel identity");
        assert!(
            !reparse && !directory,
            "leaf is not the installed regular file"
        );
        let final_path = handle_final_path(handle.0).expect("leaf final path");
        assert!(
            same_path(
                &final_path,
                &expected_final_path(&self.path).expect("expected final path")
            ),
            "leaf handle resolves to an unexpected final path"
        );
        let bytes = std::fs::read(&self.path).expect("test-installed leaf bytes");
        assert_eq!(
            bytes,
            canonical_json(expected).expect("canonical expected bytes"),
            "leaf content differs from the registry this run installed"
        );
        self.identity = Some(LeafIdentity { kernel, bytes });

        let mut dir_identities = Vec::new();
        for dir in &self.absent_dirs {
            let handle = open_object(dir, true, true).expect("open test-created ancestor");
            let (kernel, reparse, directory) =
                inspect_object(handle.0).expect("ancestor kernel identity");
            assert!(!reparse && directory, "ancestor is not a plain directory");
            let final_path = handle_final_path(handle.0).expect("ancestor final path");
            assert!(
                same_path(
                    &final_path,
                    &expected_final_path(dir).expect("expected ancestor path")
                ),
                "ancestor handle resolves to an unexpected final path"
            );
            dir_identities.push((dir.clone(), kernel));
        }
        self.dir_identities = dir_identities;
    }

    fn cleanup(&mut self) {
        if self.cleanup_attempted {
            return;
        }
        self.cleanup_attempted = true;

        let ancestry = ancestry_errors(&self.path, false);
        if !ancestry.is_empty() {
            self.cleanup_errors.extend(ancestry);
            // Do not delete any object beneath an unverified ancestor tree.
            return;
        }

        match self.identity.take() {
            Some(identity) => {
                if let Err(error) =
                    delete_verified_path(&self.path, Some(&identity.kernel), Some(false), false)
                {
                    self.cleanup_errors.push(format!(
                        "leaf deletion refused {:?}: {error}; object preserved",
                        self.path
                    ));
                }
                // Content is evidence only; a substituted file stays in place.
                if std::fs::read(&self.path)
                    .map(|current| current != identity.bytes)
                    .unwrap_or(false)
                {
                    self.cleanup_errors.push(format!(
                        "unexpected leaf bytes remain at {:?}; object preserved",
                        self.path
                    ));
                }
            }
            None => self.cleanup_errors.push(format!(
                "no verified identity for {:?}; no deletion attempted",
                self.path
            )),
        }

        for (dir, identity) in self.dir_identities.iter().rev() {
            if let Err(error) = delete_verified_path(dir, Some(identity), Some(false), true) {
                self.cleanup_errors
                    .push(format!("ancestor deletion refused {dir:?}: {error}"));
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

    /// Explicit cleanup verification. Evidence is printed before failure so it
    /// is captured in the guest log before any checkpoint recovery.
    fn finish(mut self) {
        self.cleanup();
        let residual = self.residual();
        for error in &self.cleanup_errors {
            eprintln!("PR341_CLEANUP_FAIL {error}");
        }
        if !residual.is_empty() {
            eprintln!("PR341_RESIDUAL_STATE preserved_objects={residual:?}");
        }
        assert!(
            self.cleanup_errors.is_empty() && residual.is_empty(),
            "cleanup failed closed: {} errors, {} residual objects; evidence preserved",
            self.cleanup_errors.len(),
            residual.len()
        );
    }
}

impl Drop for OwnedRegistry {
    fn drop(&mut self) {
        self.cleanup();
        for error in &self.cleanup_errors {
            eprintln!("PR341_CLEANUP_FAIL {error}");
        }
    }
}

/// Independent, cumulative-free measurements of the test-only Local mutex.
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

/// Process-level watchdog. A stuck worker cannot turn a bounded test into an
/// infinite wait: on expiry an explicit FAIL is printed and the process exits.
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

fn remove_negative_fixture(base: &Path, link: &Path, target: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    if let Err(error) = delete_verified_path(link, None, Some(true), true) {
        errors.push(format!("junction deletion refused {link:?}: {error}"));
    }
    for dir in [target, base] {
        if let Err(error) = delete_verified_path(dir, None, Some(false), true) {
            errors.push(format!(
                "negative fixture dir deletion refused {dir:?}: {error}"
            ));
        }
    }
    for path in [link, target, base] {
        match std::fs::symlink_metadata(path) {
            Ok(_) => errors.push(format!("PR341_RESIDUAL_STATE negative object {path:?}")),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => errors.push(format!(
                "PR341_CLEANUP_FAIL negative metadata {path:?}: {error}"
            )),
        }
    }
    for error in &errors {
        eprintln!("{error}");
    }
    errors
}

fn provision_worker_executable() -> PathBuf {
    let current = std::env::current_exe().expect("current test executable");
    let directory = current.parent().expect("test executable directory");
    let staged = directory.join("pr397_provision_worker.exe");
    if staged.is_file() {
        return staged;
    }
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory).expect("read test executable directory") {
        let entry = entry.expect("directory entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("pr397_provision_worker-") && name.ends_with(".exe") {
            found.push(entry.path());
        }
    }
    assert_eq!(
        found.len(),
        1,
        "expected exactly one distinct provision worker process: {found:?}"
    );
    found.remove(0)
}

/// Spawn a separate guest process and bound it with try_wait/kill. No blocking
/// wait after the deadline, and each child's stdout is independent evidence.
fn run_provision_worker(worker: &Path, registry: &Path, generation: u64) -> (bool, String) {
    let mut child = Command::new(worker)
        .env("PR341_WORKER_REGISTRY", registry)
        .env("PR341_WORKER_GENERATION", generation.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn distinct provision worker process");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().expect("poll provision worker") {
            let mut output = String::new();
            if let Some(mut stdout) = child.stdout.take() {
                let _ = stdout.read_to_string(&mut output);
            }
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.read_to_string(&mut output);
            }
            println!("PR341_PROVISION_WORKER gen={generation} exit={status}: {output}");
            return (status.success(), output);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("provision worker watchdog expired for generation {generation}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
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

    assert!(
        provision_registry(Path::new("registry.json"), &OwnerRegistry::empty(1)).is_err(),
        "relative registry path must be refused"
    );
    assert!(parse_registry_bytes(b"not json").is_err());
    assert!(parse_registry_bytes(&vec![b'x'; 1_048_577]).is_err());
    assert!(
        !is_well_formed_sid("S-1-5-21-not-a-number"),
        "non-numeric SID component must be refused"
    );

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock after epoch")
        .as_nanos();
    let base = programdata_root().unwrap().join(format!(
        "PrometheOS-Lite-PR341-Negative-{}-{nonce}",
        std::process::id()
    ));
    match std::fs::symlink_metadata(&base) {
        Ok(_) => panic!("pre-existing negative-control fixture is not test-owned: {base:?}"),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => panic!("cannot inspect negative-control fixture {base:?}: {error}"),
    }
    std::fs::create_dir(&base).expect("exclusive creation of unique negative fixture");
    let link = base.join("junction");
    let target = base.join("target");
    assert!(
        match std::fs::symlink_metadata(&link) {
            Err(error) => error.kind() == ErrorKind::NotFound,
            Ok(_) => false,
        } && match std::fs::symlink_metadata(&target) {
            Err(error) => error.kind() == ErrorKind::NotFound,
            Ok(_) => false,
        },
        "pre-existing negative-control child is not test-owned"
    );
    std::fs::create_dir(&target).expect("test-owned target directory");
    std::os::windows::fs::symlink_dir(&target, &link).expect("test-owned junction");
    let reparse_refused = reject_reparse(&link).is_err();
    let cleanup_errors = remove_negative_fixture(&base, &link, &target);

    assert!(reparse_refused, "junction must be refused");
    assert!(
        cleanup_errors.is_empty(),
        "negative-control cleanup failed closed: {cleanup_errors:?}"
    );
}

/// LIMITED SCOPE regression: this is the test-only Local mutex mechanism and
/// is NOT concurrent provisioning acceptance. Production Global-mutex
/// provisioning is verified by concurrent_provisioning_serializes_production_updates.
#[test]
fn test_mutex_local_regression_serializes_and_cleanup_is_verified() {
    let _watchdog = ProcessWatchdog::new(Duration::from_secs(20));
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
        match rx.recv_timeout(_watchdog.remaining()) {
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
    // Join only after complete success. A timeout is FAIL with no blocking join.
    if failures.is_empty() {
        for (index, handle) in handles.into_iter().enumerate() {
            if handle.join().is_err() {
                failures.push(format!("contender {index}: thread panicked"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "test-only Local mutex regression failed: {}",
        failures.join("; ")
    );
    assert_eq!(probe.acquisitions.load(Ordering::SeqCst), 2);
    assert_eq!(probe.maximum.load(Ordering::SeqCst), 1);
    assert_eq!(probe.active.load(Ordering::SeqCst), 0);
    owned.finish();
}

/// REAL acceptance: distinct guest processes contend through production
/// provision_registry and its Global mutex. Version/conflict outcomes are
/// checked after every round.
#[test]
fn concurrent_provisioning_serializes_production_updates() {
    let _watchdog = ProcessWatchdog::new(Duration::from_secs(45));
    let mut owned = OwnedRegistry::fresh();
    let worker = provision_worker_executable();

    // Two competing first installs at the same generation: serialization plus
    // monotonic compare must yield exactly one winner and one refusal.
    let first_round = [
        run_provision_worker(&worker, &owned.path, 1),
        run_provision_worker(&worker, &owned.path, 1),
    ];
    let successes = first_round.iter().filter(|result| result.0).count();
    let refusals = first_round.iter().filter(|result| !result.0).count();
    assert_eq!(
        (successes, refusals),
        (1, 1),
        "competing first installs must serialize: {first_round:?}"
    );
    let installed = load_registry(&owned.path).expect("installed registry must load");
    assert_eq!(
        installed.generation, 1,
        "winner generation must be installed"
    );

    // Two competing updates at generations 2 and 3: the higher generation wins
    // if it acquires the lock second; otherwise generation 2 wins and the
    // stale generation 3 update is refused. Exactly one succeeds either way.
    let second_round = [
        run_provision_worker(&worker, &owned.path, 2),
        run_provision_worker(&worker, &owned.path, 3),
    ];
    let successes = second_round.iter().filter(|result| result.0).count();
    let refusals = second_round.iter().filter(|result| !result.0).count();
    assert_eq!(
        (successes, refusals),
        (1, 1),
        "competing updates must serialize: {second_round:?}"
    );
    let winner_generation = if second_round[0].0 { 2 } else { 3 };
    let final_registry = load_registry(&owned.path).expect("final registry must load");
    assert_eq!(
        final_registry.generation, winner_generation,
        "final version must equal the winning provisioner's generation"
    );
    let expected = owned.registry(
        winner_generation,
        vec![OwnedRegistry::binding(SID_A, "pr341-worker", false, 1)],
    );
    assert_eq!(
        canonical_json(&final_registry).expect("final bytes"),
        canonical_json(&expected).expect("expected bytes"),
        "final registry bytes must equal the winning provisioner's document"
    );
    owned.observe(&expected);
    owned.finish();
}
