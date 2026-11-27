//! PR341 Stage 1 real privileged acceptance target.
//! Windows guest only: genuine production owner_registry APIs against the
//! guest's OS-managed ProgramData root. No simulated SIDs or JSON-only
//! positive provisioning.

#![cfg(windows)]

use prometheos_lite::work::owner_registry::windows::{
    acquire_test_mutex, last_native_io_diagnostic, load_registry, provision_registry,
    reject_reparse, require_provisioning_authority, resolve_owner, test_mutex_name,
    validate_file_security, validate_registry_leaf, validate_registry_location,
};
use prometheos_lite::work::owner_registry::{
    OWNER_REGISTRY_SCHEMA, OwnerBinding, OwnerRegistry, canonical_json, default_registry_path,
    enforce_monotonic_update, is_well_formed_sid, parse_registry_bytes, programdata_root,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use winapi::shared::minwindef::{DWORD, FALSE, TRUE};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::fileapi::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_DISPOSITION_INFO, GetFileInformationByHandle,
    GetFinalPathNameByHandleW, OPEN_EXISTING, SetFileInformationByHandle,
};
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::minwinbase::FileDispositionInfo;
use winapi::um::synchapi::{CreateEventW, SetEvent, WaitForSingleObject};
use winapi::um::winbase::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, VOLUME_NAME_DOS, WAIT_OBJECT_0,
};
use winapi::um::winnt::{
    DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, HANDLE,
};

const SID_A: &str = "S-1-5-21-1111111111-2222222222-3333333333-1001";
const SID_B: &str = "S-1-5-21-1111111111-2222222222-3333333333-1002";
const WORKER_ACCEPTED: i32 = 0;
const WORKER_REFUSED: i32 = 10;
const WORKER_EXECUTION_FAILURE: i32 = 20;

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock after epoch")
        .as_nanos()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn millis(deadline: Instant) -> DWORD {
    deadline
        .saturating_duration_since(Instant::now())
        .as_millis()
        .min(DWORD::MAX as u128) as DWORD
}

struct VerifiedHandle(HANDLE);

impl Drop for VerifiedHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

struct EventHandle(HANDLE);

impl Drop for EventHandle {
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
    // CreateFileW failure is INVALID_HANDLE_VALUE (-1), never null.
    if handle == INVALID_HANDLE_VALUE {
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
    // A drive root (C:\) has no parent or file name. Its kernel-resolved final
    // path is simply its own canonical form; do not fabricate a parent.
    if path.parent().is_none() || path.file_name().is_none() {
        return std::fs::canonicalize(path)
            .map_err(|error| format!("canonicalize drive root {path:?}: {error}"));
    }
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

/// Normalize only unambiguous local drive paths: `C:\...` and its legitimate
/// extended-length form `\\?\C:\...`. UNC, device, volume GUID, relative and
/// drive-relative paths are deliberately not normalized and never compare equal.
fn normalized_local_path(path: &Path) -> Option<String> {
    let text = path.as_os_str().to_str()?;
    let stripped = text.strip_prefix("\\\\?\\").unwrap_or(text);
    // After the single allowed extended prefix, another UNC/device prefix is
    // not a local drive path.
    if stripped.starts_with("\\\\") || stripped.starts_with("//") {
        return None;
    }
    let local = stripped.replace('/', "\\");
    let bytes = local.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return None;
    }
    Some(local.to_ascii_lowercase())
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (normalized_local_path(left), normalized_local_path(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn capture_identity(
    path: &Path,
    expect_directory: bool,
    expect_reparse: bool,
) -> Result<KernelIdentity, String> {
    let handle = open_object(path, expect_directory, false)?;
    let (identity, reparse, directory) = inspect_object(handle.0)?;
    if reparse != expect_reparse || directory != expect_directory {
        return Err(format!("unexpected object kind/reparse state for {path:?}"));
    }
    if !same_path(&handle_final_path(handle.0)?, &expected_final_path(path)?) {
        return Err(format!("final path changed for {path:?}"));
    }
    Ok(identity)
}

/// Delete only through an already verified kernel handle, with an identity
/// captured before any destructive operation. No path lookup is repeated
/// between verification and deletion.
fn delete_verified_path(
    path: &Path,
    expected_identity: &KernelIdentity,
    expected_reparse: bool,
    expect_directory: bool,
) -> Result<(), String> {
    let handle = open_object(path, expect_directory, true)?;
    let (identity, reparse, directory) = inspect_object(handle.0)?;
    if identity != *expected_identity {
        return Err(format!("kernel identity changed for {path:?}"));
    }
    if reparse != expected_reparse || directory != expect_directory {
        return Err(format!("object kind/reparse state changed for {path:?}"));
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

/// Validate the OS-managed ProgramData boundary itself. This is the trusted
/// root: it must be the Known Folder value, a plain non-reparse directory whose
/// kernel-resolved final path is itself, and never the drive root.
fn validate_programdata_boundary(root: &Path, errors: &mut Vec<String>) {
    if !root
        .to_string_lossy()
        .eq_ignore_ascii_case("C:\\ProgramData")
    {
        errors.push(format!("untrusted ProgramData boundary: {root:?}"));
        return;
    }
    if root.parent().is_none() || root.file_name().is_none() {
        errors.push(format!("ProgramData boundary is a drive root: {root:?}"));
        return;
    }
    match std::fs::canonicalize(root) {
        Ok(canonical) if same_path(&canonical, root) => {}
        Ok(canonical) => errors.push(format!(
            "ProgramData boundary resolves elsewhere: {root:?} -> {canonical:?}"
        )),
        Err(error) => errors.push(format!("cannot canonicalize ProgramData {root:?}: {error}")),
    }
    match std::fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => errors.push(format!(
            "ProgramData boundary is not a plain directory: {root:?}"
        )),
        Err(error) => errors.push(format!("cannot inspect ProgramData {root:?}: {error}")),
    }
    if let Err(error) = reject_reparse(root) {
        errors.push(format!(
            "ProgramData boundary reparse check failed: {error:?}"
        ));
    }
    if let Err(error) = capture_identity(root, true, false) {
        errors.push(format!(
            "ProgramData boundary identity check failed: {error}"
        ));
    }
}

/// Walk only the protected ancestors beneath ProgramData. The OS-managed
/// ProgramData root is validated separately and terminates traversal; the
/// drive root is never visited by test-owned cleanup.
fn ancestry_errors(path: &Path, allow_absent: bool) -> Vec<String> {
    let mut errors = Vec::new();
    let root = match programdata_root() {
        Ok(root) => root,
        Err(error) => {
            errors.push(format!("ProgramData root unavailable: {error}"));
            return errors;
        }
    };
    if !path.starts_with(&root) {
        errors.push(format!(
            "protected path escapes ProgramData boundary: {path:?}"
        ));
        return errors;
    }
    validate_programdata_boundary(&root, &mut errors);

    let mut reached_boundary = false;
    let mut cursor = path.parent();
    while let Some(dir) = cursor {
        if same_path(dir, &root) {
            reached_boundary = true;
            break;
        }
        match std::fs::symlink_metadata(dir) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    errors.push(format!("ancestor is not a plain directory: {dir:?}"));
                }
                if let Err(error) = reject_reparse(dir) {
                    errors.push(format!("ancestor reparse check failed {dir:?}: {error:?}"));
                }
                match capture_identity(dir, true, false) {
                    Ok(_) => {}
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
    if !reached_boundary {
        errors.push(format!(
            "ProgramData ancestry boundary was not reached from {path:?}"
        ));
    }
    errors
}

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
            if same_path(dir, &root) {
                break;
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

    fn observe(&mut self, expected: &OwnerRegistry) {
        let errors = ancestry_errors(&self.path, false);
        assert!(
            errors.is_empty(),
            "unsafe ancestry after provisioning: {errors:?}"
        );
        reject_reparse(&self.path).expect("test-installed leaf must not be a reparse point");
        validate_registry_leaf(&self.path).expect("test-installed leaf must be trusted");
        let kernel = capture_identity(&self.path, false, false).expect("leaf kernel identity");
        let bytes = std::fs::read(&self.path).expect("test-installed leaf bytes");
        assert_eq!(
            bytes,
            canonical_json(expected).expect("canonical expected bytes"),
            "leaf content differs from the registry this run installed"
        );
        self.identity = Some(LeafIdentity { kernel, bytes });

        let mut dir_identities = Vec::new();
        for dir in &self.absent_dirs {
            let identity = capture_identity(dir, true, false).expect("ancestor kernel identity");
            dir_identities.push((dir.clone(), identity));
        }
        self.dir_identities = dir_identities;
    }

    fn cleanup(&mut self) {
        if self.cleanup_attempted {
            return;
        }
        self.cleanup_attempted = true;

        // State-aware cleanup: a refused provisioning leaves the initially
        // absent protected directories absent. That is the expected no-op case,
        // not a security failure. Any object that appears there is unexplained
        // and must be preserved and reported.
        let no_test_objects = self.identity.is_none() && self.dir_identities.is_empty();
        let ancestry = ancestry_errors(&self.path, no_test_objects);
        if no_test_objects {
            // The leaf itself must also be proven absent after a refused install.
            match std::fs::symlink_metadata(&self.path) {
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Ok(_) => self
                    .cleanup_errors
                    .push(format!("unexplained leaf at {:?}; preserved", self.path)),
                Err(error) => self.cleanup_errors.push(format!(
                    "unexpected leaf metadata at {:?}: {error}",
                    self.path
                )),
            }
            for dir in &self.absent_dirs {
                match std::fs::symlink_metadata(dir) {
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Ok(_) => self.cleanup_errors.push(format!(
                        "unexplained object at initially absent {dir:?}; preserved"
                    )),
                    Err(error) => self.cleanup_errors.push(format!(
                        "unexpected metadata at initially absent {dir:?}: {error}"
                    )),
                }
            }
        }
        if !ancestry.is_empty() {
            self.cleanup_errors.extend(ancestry);
            return;
        }
        if no_test_objects {
            // Nothing was created by this run: no deletion is authorized.
            return;
        }
        match self.identity.take() {
            Some(identity) => {
                if let Err(error) = delete_verified_path(&self.path, &identity.kernel, false, false)
                {
                    self.cleanup_errors.push(format!(
                        "leaf deletion refused {:?}: {error}; object preserved",
                        self.path
                    ));
                }
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
        // absent_dirs and dir_identities are recorded deepest-first. Removing
        // in that order never asks Windows to delete a still-nonempty parent.
        for (dir, identity) in self.dir_identities.iter() {
            if let Err(error) = delete_verified_path(dir, identity, false, true) {
                self.cleanup_errors
                    .push(format!("ancestor deletion refused {dir:?}: {error}"));
            }
        }
    }

    /// Report both surviving objects and paths whose metadata could not be
    /// inspected. `symlink_metadata(...).is_ok()` alone would silently treat a
    /// permission or I/O failure as absence, which is not fail-closed.
    fn residual(&self) -> (Vec<PathBuf>, Vec<String>) {
        let mut objects = Vec::new();
        let mut errors = Vec::new();
        for path in std::iter::once(&self.path).chain(self.absent_dirs.iter()) {
            match std::fs::symlink_metadata(path) {
                Ok(_) => objects.push(path.clone()),
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    errors.push(format!("residual inspection failed for {path:?}: {error}"))
                }
            }
        }
        (objects, errors)
    }

    fn finish(mut self) {
        self.cleanup();
        let (residual, inspection_errors) = self.residual();
        self.cleanup_errors.extend(inspection_errors);
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

/// Test-only Local mutex probe. NOT provisioning acceptance.
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

/// Process-level watchdog. On expiry it records that cleanup was not verified
/// and requires controlled guest inspection before any checkpoint recovery.
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
                        "PR341_WATCHDOG_FAIL timeout={timeout:?} cleanup=NOT_VERIFIED checkpoint_recovery=REQUIRED_BEFORE_RETRY"
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

/// Negative fixture with an identity captured for every object before any
/// deletion is authorized. No destructive call receives an unbound identity.
struct NegativeFixture {
    base: PathBuf,
    link: PathBuf,
    target: PathBuf,
    base_identity: Option<KernelIdentity>,
    link_identity: Option<KernelIdentity>,
    target_identity: Option<KernelIdentity>,
    errors: Vec<String>,
}

impl NegativeFixture {
    fn new() -> Self {
        let root = programdata_root().expect("ProgramData root");
        let base = root.join(format!("PrometheOS-Lite-PR341-Negative-{}", nonce()));
        match std::fs::symlink_metadata(&base) {
            Ok(_) => panic!("pre-existing negative fixture is not test-owned: {base:?}"),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => panic!("cannot inspect negative fixture {base:?}: {error}"),
        }
        std::fs::create_dir(&base).expect("exclusive negative fixture creation");
        let target = base.join("target");
        let link = base.join("junction");
        let mut fixture = Self {
            base: base.clone(),
            link,
            target,
            base_identity: None,
            link_identity: None,
            target_identity: None,
            errors: Vec::new(),
        };
        fixture.base_identity = match capture_identity(&base, true, false) {
            Ok(identity) => Some(identity),
            Err(error) => {
                fixture
                    .errors
                    .push(format!("base identity capture failed: {error}"));
                None
            }
        };
        for child in [&fixture.target, &fixture.link] {
            match std::fs::symlink_metadata(child) {
                Ok(_) => panic!("pre-existing negative child is not test-owned: {child:?}"),
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => panic!("cannot inspect negative child {child:?}: {error}"),
            }
        }
        std::fs::create_dir(&fixture.target).expect("test-owned target directory");
        fixture.target_identity = match capture_identity(&fixture.target, true, false) {
            Ok(identity) => Some(identity),
            Err(error) => {
                fixture
                    .errors
                    .push(format!("target identity capture failed: {error}"));
                None
            }
        };
        std::os::windows::fs::symlink_dir(&fixture.target, &fixture.link)
            .expect("test-owned junction");
        fixture.link_identity = match capture_identity(&fixture.link, true, true) {
            Ok(identity) => Some(identity),
            Err(error) => {
                fixture
                    .errors
                    .push(format!("junction identity capture failed: {error}"));
                None
            }
        };
        fixture
    }

    fn finish(mut self) {
        for (path, identity, reparse) in [
            (&self.link, self.link_identity, true),
            (&self.target, self.target_identity, false),
            (&self.base, self.base_identity, false),
        ] {
            match identity {
                Some(identity) => {
                    if let Err(error) = delete_verified_path(path, &identity, reparse, true) {
                        self.errors
                            .push(format!("deletion refused {path:?}: {error}"));
                    }
                }
                None => self.errors.push(format!(
                    "no captured kernel identity for {path:?}; object preserved"
                )),
            }
        }
        for path in [&self.link, &self.target, &self.base] {
            match std::fs::symlink_metadata(path) {
                Ok(_) => self.errors.push(format!("PR341_RESIDUAL_STATE {path:?}")),
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => self
                    .errors
                    .push(format!("PR341_CLEANUP_FAIL metadata {path:?}: {error}")),
            }
        }
        for error in &self.errors {
            eprintln!("{error}");
        }
        assert!(
            self.errors.is_empty(),
            "negative fixture cleanup failed closed: {:?}",
            self.errors
        );
    }
}

#[derive(Clone)]
struct WorkerPlan {
    label: String,
    generation: u64,
    owner: String,
}

#[derive(Debug)]
enum WorkerOutcome {
    Accepted { owner: String, digest: String },
    Refused { owner: String, refusal: String },
    ExecutionFailure(String),
    Crash(String),
    Timeout,
}

#[derive(Debug)]
struct WorkerResult {
    label: String,
    outcome: WorkerOutcome,
    /// Bounded native I/O diagnostic carried from the worker outcome file into
    /// the final captured acceptance log. No protected bytes or secrets.
    diagnostic: String,
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
        "exactly one provision worker binary: {found:?}"
    );
    found.remove(0)
}

fn create_event(name: &str, manual_reset: bool) -> Result<EventHandle, String> {
    let handle = unsafe {
        CreateEventW(
            std::ptr::null_mut(),
            if manual_reset { TRUE } else { FALSE },
            FALSE,
            wide(OsStr::new(name)).as_ptr(),
        )
    };
    if handle.is_null() {
        return Err(format!("CreateEventW {name} failed: {}", unsafe {
            GetLastError()
        }));
    }
    Ok(EventHandle(handle))
}

fn classify_worker(
    label: &str,
    status: Option<ExitStatus>,
    timed_out: bool,
    outcome_path: &Path,
) -> WorkerResult {
    let outcome = if timed_out {
        WorkerOutcome::Timeout
    } else {
        let code = status.and_then(|status| status.code());
        let parsed: Value = match std::fs::metadata(outcome_path) {
            Ok(metadata) if metadata.len() <= 16 * 1024 => std::fs::read_to_string(outcome_path)
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .unwrap_or(Value::Null),
            _ => Value::Null,
        };
        let kind = parsed.get("kind").and_then(Value::as_str).unwrap_or("");
        let owner = parsed
            .get("owner")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        match (code, kind) {
            (Some(WORKER_ACCEPTED), "accepted") => WorkerOutcome::Accepted {
                owner,
                digest: parsed
                    .get("registry_sha256")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            },
            (Some(WORKER_REFUSED), "refused") => WorkerOutcome::Refused {
                owner,
                refusal: parsed
                    .get("refusal")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            },
            (Some(WORKER_EXECUTION_FAILURE), "execution_failure") => {
                WorkerOutcome::ExecutionFailure(format!("{parsed}"))
            }
            _ => WorkerOutcome::Crash(format!("exit={code:?} outcome={parsed}")),
        }
    };
    let diagnostic = parsed_field(outcome_path, "io_diagnostic", timed_out, status);
    WorkerResult {
        label: label.to_string(),
        outcome,
        diagnostic,
    }
}

fn parsed_field(
    outcome_path: &Path,
    field: &str,
    timed_out: bool,
    status: Option<ExitStatus>,
) -> String {
    if timed_out || status.and_then(|s| s.code()) != Some(WORKER_REFUSED) {
        return String::new();
    }
    match std::fs::read_to_string(outcome_path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(value) => value
                .get(field)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            Err(_) => String::new(),
        },
        Err(_) => String::new(),
    }
}

/// Spawn both child processes before waiting for either, synchronize their
/// start with named Win32 events, and monitor them with bounded polling.
/// Child stdout/stderr are null so no inherited pipe can block output reads.
fn run_worker_pair(registry: &Path, plans: &[WorkerPlan]) -> Vec<WorkerResult> {
    assert_eq!(plans.len(), 2, "a competing pair is required");
    let worker = provision_worker_executable();
    let run_nonce = nonce();
    let sync_dir = std::env::temp_dir().join(format!("pr341-sync-{run_nonce}"));
    std::fs::create_dir(&sync_dir).expect("exclusive sync directory");
    let go_name = format!("Local\\PR341-SYNC-{run_nonce}-go");
    let ready_names: Vec<String> = (0..2)
        .map(|index| format!("Local\\PR341-SYNC-{run_nonce}-ready-{index}"))
        .collect();
    let go = create_event(&go_name, true).expect("go event");
    let ready_events: Vec<EventHandle> = ready_names
        .iter()
        .map(|name| create_event(name, false).expect("ready event"))
        .collect();

    let mut children = Vec::new();
    let mut outcome_paths = Vec::new();
    for (index, plan) in plans.iter().enumerate() {
        let outcome_path = sync_dir.join(format!("outcome-{index}.json"));
        outcome_paths.push(outcome_path.clone());
        let child = Command::new(&worker)
            .env("PR341_WORKER_REGISTRY", registry)
            .env("PR341_WORKER_GENERATION", plan.generation.to_string())
            .env("PR341_WORKER_OWNER", &plan.owner)
            .env("PR341_WORKER_OUTCOME", &outcome_path)
            .env("PR341_WORKER_READY_EVENT", &ready_names[index])
            .env("PR341_WORKER_GO_EVENT", &go_name)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn distinct provision worker process");
        children.push(child);
    }

    let ready_deadline = Instant::now() + Duration::from_secs(10);
    let mut ready_ok = true;
    for event in &ready_events {
        let wait = unsafe { WaitForSingleObject(event.0, millis(ready_deadline)) };
        if wait != WAIT_OBJECT_0 {
            eprintln!("PR341_WORKER_FAIL ready synchronization wait={wait}");
            ready_ok = false;
            break;
        }
    }
    if ready_ok {
        assert_ne!(unsafe { SetEvent(go.0) }, 0, "release both contenders");
    } else {
        for child in children.iter_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    let collect_deadline = Instant::now() + Duration::from_secs(20);
    let mut statuses: Vec<Option<ExitStatus>> = vec![None; children.len()];
    let mut timed_out = vec![!ready_ok; children.len()];
    loop {
        let mut complete = true;
        for (index, child) in children.iter_mut().enumerate() {
            if statuses[index].is_none() && !timed_out[index] {
                match child.try_wait().expect("poll provision worker") {
                    Some(status) => statuses[index] = Some(status),
                    None => complete = false,
                }
            }
        }
        if complete {
            break;
        }
        if Instant::now() >= collect_deadline {
            for (index, child) in children.iter_mut().enumerate() {
                if statuses[index].is_none() {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out[index] = true;
                }
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    let results: Vec<WorkerResult> = plans
        .iter()
        .enumerate()
        .map(|(index, plan)| {
            let result = classify_worker(
                &plan.label,
                statuses[index],
                timed_out[index],
                &outcome_paths[index],
            );
            println!(
                "PR341_WORKER_RESULT {}: {:?} io={}",
                plan.label, result.outcome, result.diagnostic
            );
            result
        })
        .collect();

    for path in &outcome_paths {
        if std::fs::symlink_metadata(path).is_ok() {
            let _ = std::fs::remove_file(path);
        }
    }
    let _ = std::fs::remove_dir(&sync_dir);
    assert!(
        std::fs::symlink_metadata(&sync_dir).is_err(),
        "temporary worker synchronization state remains: {sync_dir:?}"
    );
    results
}

fn describe_results(results: &[WorkerResult]) -> String {
    results
        .iter()
        .map(|result| match &result.outcome {
            WorkerOutcome::Accepted { owner, digest } => {
                format!("{}:accepted(owner={owner},digest={digest})", result.label)
            }
            WorkerOutcome::Refused { owner, refusal } => {
                format!(
                    "{}:refused(owner={owner},refusal={refusal},io={})",
                    result.label, result.diagnostic
                )
            }
            WorkerOutcome::ExecutionFailure(detail) => {
                format!("{}:execution_failure({detail})", result.label)
            }
            WorkerOutcome::Crash(detail) => format!("{}:crash({detail})", result.label),
            WorkerOutcome::Timeout => format!("{}:timeout", result.label),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn assert_competing_pair(
    results: &[WorkerResult],
    generation: u64,
    compatible_owner: Option<&str>,
) -> (String, String) {
    let accepted: Vec<&WorkerResult> = results
        .iter()
        .filter(|result| matches!(result.outcome, WorkerOutcome::Accepted { .. }))
        .collect();
    let refused: Vec<&WorkerResult> = results
        .iter()
        .filter(|result| matches!(result.outcome, WorkerOutcome::Refused { .. }))
        .collect();
    assert_eq!(
        accepted.len(),
        1,
        "exactly one competing generation {generation} proposal must be accepted: {}",
        describe_results(results)
    );
    assert_eq!(
        refused.len(),
        1,
        "exactly one competing generation {generation} proposal must be refused: {}",
        describe_results(results)
    );
    let refusal = match &refused[0].outcome {
        WorkerOutcome::Refused { refusal, .. } => refusal,
        other => unreachable!("classified refusal: {other:?}"),
    };
    assert!(
        refusal.contains("NonMonotonicUpdate") || refusal.contains("ConcurrentUpdate"),
        "unexpected refusal must be a monotonic/concurrency conflict: {refusal}"
    );
    let (owner, digest) = match &accepted[0].outcome {
        WorkerOutcome::Accepted { owner, digest } => (owner.clone(), digest.clone()),
        other => unreachable!("classified acceptance: {other:?}"),
    };
    if let Some(expected) = compatible_owner {
        assert_eq!(
            owner, expected,
            "accepted owner must match the prior document; substitution is refused"
        );
    }
    (owner, digest)
}

fn verify_final_document(owned: &OwnedRegistry, generation: u64, owner: &str, digest: &str) {
    let installed =
        load_registry(&owned.path).expect("final registry must load via production API");
    assert_eq!(
        installed.generation, generation,
        "final generation mismatch"
    );
    let expected = owned.registry(
        generation,
        vec![OwnedRegistry::binding(SID_A, owner, false, 1)],
    );
    let bytes = canonical_json(&expected).expect("expected canonical bytes");
    assert_eq!(
        canonical_json(&installed).expect("installed canonical bytes"),
        bytes,
        "final canonical bytes must equal the accepted proposal"
    );
    assert_eq!(hex(Sha256::digest(&bytes).as_slice()), digest);
}

/// Local, nonprivileged regression for R2: test-owned nested directories must
/// be removed deepest-first, and unowned contents must block removal.
#[test]
fn owned_nested_directories_are_removed_deepest_first() {
    let base = std::env::temp_dir().join(format!("pr403-cleanup-{}", nonce()));
    std::fs::create_dir(&base).expect("test-owned base");
    let nested = base.join("a").join("b").join("c");
    std::fs::create_dir_all(&nested).expect("test-owned nested directories");
    let dirs = [nested.clone(), base.join("a").join("b"), base.join("a")];
    let identities: Vec<_> = dirs
        .iter()
        .map(|dir| capture_identity(dir, true, false).expect("directory identity"))
        .collect();
    let leaf = nested.join("leaf.json");
    std::fs::write(&leaf, b"owned").expect("test-owned leaf");
    let leaf_identity = capture_identity(&leaf, false, false).expect("leaf identity");
    delete_verified_path(&leaf, &leaf_identity, false, false).expect("leaf deleted");
    for (dir, identity) in dirs.iter().zip(identities.iter()) {
        delete_verified_path(dir, identity, false, true).expect("deepest-first deletion");
    }
    assert!(!base.join("a").exists(), "all test-owned ancestors removed");
    assert!(base.exists(), "pre-existing base survives");
    std::fs::remove_dir(&base).expect("remove test-owned base");

    // Unowned content in an owned ancestor must block its removal and survive.
    std::fs::create_dir(&base).expect("second test-owned base");
    std::fs::create_dir_all(&nested).expect("second nested directories");
    let second_identities: Vec<_> = dirs
        .iter()
        .map(|dir| capture_identity(dir, true, false).expect("directory identity"))
        .collect();
    std::fs::write(&leaf, b"owned").expect("second owned leaf");
    let second_leaf = capture_identity(&leaf, false, false).expect("second leaf identity");
    delete_verified_path(&leaf, &second_leaf, false, false).expect("second leaf deleted");
    let unowned = dirs[1].join("unowned.txt");
    std::fs::write(&unowned, b"not ours").expect("unowned content");
    delete_verified_path(&dirs[0], &second_identities[0], false, true).expect("deepest removed");
    assert!(
        delete_verified_path(&dirs[1], &second_identities[1], false, true).is_err(),
        "nonempty parent must not be deleted"
    );
    assert!(unowned.exists(), "unowned content must be preserved");
    assert!(
        dirs[1].exists(),
        "parent holding unowned content must survive"
    );
    let _ = std::fs::remove_file(&unowned);
    let _ = std::fs::remove_dir(&dirs[1]);
    let _ = std::fs::remove_dir(&dirs[2]);
    let _ = std::fs::remove_dir(&base);
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
    provision_registry(&owned.path, &expected).unwrap_or_else(|error| {
        panic!(
            "elevated first install failed: {error:?} io={:?}",
            last_native_io_diagnostic()
        )
    });
    owned.observe(&expected);
    let installed = load_registry(&owned.path).expect("installed registry must load");
    assert_eq!(installed, expected);
    assert_eq!(
        canonical_json(&installed).expect("canonical bytes"),
        canonical_json(&expected).expect("canonical bytes")
    );
    validate_registry_location(&owned.path).expect("trusted owner and DACL");
    validate_file_security(owned.path.parent().expect("protected parent"))
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
    provision_registry(&owned.path, &first).unwrap_or_else(|error| {
        panic!(
            "monotonic first install failed: {error:?} io={:?}",
            last_native_io_diagnostic()
        )
    });
    owned.observe(&first);
    let second = owned.registry(
        2,
        vec![
            OwnedRegistry::binding(SID_A, "owner-a", false, 1),
            OwnedRegistry::binding(SID_B, "owner-b", true, 2),
        ],
    );
    provision_registry(&owned.path, &second).unwrap_or_else(|error| {
        panic!(
            "monotonic update failed: {error:?} io={:?}",
            last_native_io_diagnostic()
        )
    });
    owned.observe(&second);
    assert_eq!(resolve_owner(&owned.path, SID_B).unwrap(), None);
    assert_eq!(
        resolve_owner(&owned.path, SID_A).unwrap(),
        Some("owner-a".into())
    );
    assert!(enforce_monotonic_update(Some(&second), &first).is_err());
    let reversal = owned.registry(
        3,
        vec![
            OwnedRegistry::binding(SID_A, "owner-a", false, 1),
            OwnedRegistry::binding(SID_B, "owner-b", false, 3),
        ],
    );
    assert!(enforce_monotonic_update(Some(&second), &reversal).is_err());
    let substituted = owned.registry(
        3,
        vec![
            OwnedRegistry::binding(SID_A, "attacker", false, 2),
            OwnedRegistry::binding(SID_B, "owner-b", true, 2),
        ],
    );
    assert!(enforce_monotonic_update(Some(&second), &substituted).is_err());
    let omitted_tombstone =
        owned.registry(3, vec![OwnedRegistry::binding(SID_A, "owner-a", false, 1)]);
    assert!(enforce_monotonic_update(Some(&second), &omitted_tombstone).is_err());
    owned.finish();
}

#[test]
fn negative_controls_fail_closed() {
    let _authority = require_provisioning_authority().expect("elevated authority");
    assert!(provision_registry(Path::new("registry.json"), &OwnerRegistry::empty(1)).is_err());
    assert!(parse_registry_bytes(b"not json").is_err());
    assert!(parse_registry_bytes(&vec![b'x'; 1_048_577]).is_err());
    assert!(!is_well_formed_sid("S-1-5-21-not-a-number"));
    let fixture = NegativeFixture::new();
    let reparse_refused = reject_reparse(&fixture.link).is_err();
    fixture.finish();
    assert!(reparse_refused, "junction must be refused");
}

/// LIMITED SCOPE: test-only Local mutex regression, not provisioning acceptance.
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
                failures.push(format!("contender {index}: no acknowledgment"))
            }
        }
    }
    if failures.is_empty() {
        for (index, handle) in handles.into_iter().enumerate() {
            if handle.join().is_err() {
                failures.push(format!("contender {index}: thread panicked"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Local mutex regression failed: {failures:?}"
    );
    assert_eq!(probe.acquisitions.load(Ordering::SeqCst), 2);
    assert_eq!(probe.maximum.load(Ordering::SeqCst), 1);
    assert_eq!(probe.active.load(Ordering::SeqCst), 0);
    owned.finish();
}

#[test]
fn sequential_generation_progression_control() {
    let mut owned = OwnedRegistry::fresh();
    for generation in 1..=3 {
        let proposed = owned.registry(
            generation,
            vec![OwnedRegistry::binding(SID_A, "pr341-sequential", false, 1)],
        );
        provision_registry(&owned.path, &proposed).unwrap_or_else(|error| {
            panic!(
                "sequential generation {generation} failed: {error:?} io={:?}",
                last_native_io_diagnostic()
            )
        });
        owned.observe(&proposed);
    }
    let final_registry = load_registry(&owned.path).expect("final registry");
    assert_eq!(final_registry.generation, 3);
    assert_eq!(
        resolve_owner(&owned.path, SID_A).unwrap(),
        Some("pr341-sequential".to_string())
    );
    owned.finish();
}

#[test]
fn concurrent_provisioning_serializes_production_updates() {
    let _watchdog = ProcessWatchdog::new(Duration::from_secs(60));
    let mut owned = OwnedRegistry::fresh();
    let round_one = run_worker_pair(
        &owned.path,
        &[
            WorkerPlan {
                label: "gen1-A".to_string(),
                generation: 1,
                owner: "pr341-worker-A".to_string(),
            },
            WorkerPlan {
                label: "gen1-B".to_string(),
                generation: 1,
                owner: "pr341-worker-B".to_string(),
            },
        ],
    );
    let (accepted_owner, digest) = assert_competing_pair(&round_one, 1, None);
    verify_final_document(&owned, 1, &accepted_owner, &digest);

    let other_owner = if accepted_owner == "pr341-worker-A" {
        "pr341-worker-B"
    } else {
        "pr341-worker-A"
    };
    let round_two = run_worker_pair(
        &owned.path,
        &[
            WorkerPlan {
                label: "gen2-compatible".to_string(),
                generation: 2,
                owner: accepted_owner.clone(),
            },
            WorkerPlan {
                label: "gen2-substitution".to_string(),
                generation: 2,
                owner: other_owner.to_string(),
            },
        ],
    );
    let (final_owner, digest) = assert_competing_pair(&round_two, 2, Some(&accepted_owner));
    assert_eq!(
        final_owner, accepted_owner,
        "untrusted owner substitution refused"
    );
    verify_final_document(&owned, 2, &final_owner, &digest);

    let expected = owned.registry(
        2,
        vec![OwnedRegistry::binding(SID_A, &final_owner, false, 1)],
    );
    owned.observe(&expected);
    owned.finish();
}
