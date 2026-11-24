#![cfg(windows)]

//! Stage 2 privileged-acceptance evidence for PR #340/#341.
//!
//! LAYERS (kept separate by design):
//! 1. Compile-time: Windows-only target using the reviewed public API.
//! 2. Static safety preflight: exact path, no reparse, verifiable ACL — no mutation.
//! 3. Runtime isolation attestation: an independently supplied, digest-pinned
//!    artifact. No attestation is currently approved, so mutating tests fail
//!    closed. Environment variables are NEVER sufficient.
//! 4. Authorized privileged acceptance: only after (3) and explicit Principal
//!    approval on a disposable GitHub-hosted runner.
//!
//! Opt-out is reported as `NOT_RUN — STAGE2_DISABLED`, never as a PASS.
//! This target is not invoked by the merged Stage 1 workflow.

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use prometheos_lite::work::owner_registry as registry;
use registry::windows::{
    acquire_test_mutex, dacl_grants_full_control_to, load_registry, provision_registry,
    reject_reparse, require_provisioning_authority, resolve_owner, test_mutex_name,
    validate_file_security, validate_registry_location,
};
use registry::{OwnerBinding, OwnerRegistry, Refusal, SID_ADMINISTRATORS, SID_EVERYONE};
use sha2::{Digest, Sha256};
use winapi::shared::minwindef::TRUE;
use winapi::um::fileapi::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, GetFileInformationByHandle, OPEN_EXISTING,
};
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::winbase::FILE_FLAG_BACKUP_SEMANTICS;
use winapi::um::winnt::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GENERIC_READ,
};

const OPT_IN: &str = "PR341_STAGE2_ACCEPTANCE";
const ATTESTATION_PATH_VAR: &str = "PR341_STAGE2_ATTESTATION";
const LOCK_CHILD_VAR: &str = "PR341_STAGE2_LOCK_CHILD";
const LOCK_MARKER_VAR: &str = "PR341_STAGE2_LOCK_MARKER";
/// No attestation artifact is approved. A Principal-reviewed PR must move this
/// pin before any privileged Stage 2 effect is possible.
const APPROVED_ATTESTATION_SHA256: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";
static FIXTURE_COUNTER: AtomicUsize = AtomicUsize::new(0);
fn temp_base() -> PathBuf {
    std::env::var_os("RUNNER_TEMP")
        .or_else(|| std::env::var_os("TEMP"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

macro_rules! require_stage2 {
    () => {
        if std::env::var_os(OPT_IN).as_deref() != Some(OsStr::new("1")) {
            eprintln!("NOT_RUN — STAGE2_DISABLED: privileged acceptance not executed");
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

fn unique_suffix(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let counter = FIXTURE_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("{prefix}-{}-{nanos}-{counter}", std::process::id())
}

/// Layer 2 only: exact location and verifiable OS security metadata. Performs
/// no mutation and deliberately makes no claim about runner isolation.
fn static_preflight() -> Result<PathBuf, String> {
    let path = registry::default_registry_path().map_err(|e| format!("default path: {e}"))?;
    let root = registry::programdata_root().map_err(|e| format!("ProgramData: {e}"))?;
    if !path.is_absolute() || !path.starts_with(&root) {
        return Err("registry path is not under the OS ProgramData root".into());
    }
    if path.file_name().and_then(OsStr::to_str) != Some(registry::REGISTRY_FILE_NAME) {
        return Err("registry leaf name is not pinned".into());
    }
    if reject_reparse(&root).is_err() {
        return Err("ProgramData root is a reparse point".into());
    }
    if dacl_grants_full_control_to(&root, SID_ADMINISTRATORS).is_err() {
        return Err("ProgramData ACL cannot be verified".into());
    }
    Ok(path)
}

/// Layer 3: independent runtime attestation. Env vars are ignored entirely.
fn runtime_isolation_attestation(path: Option<&Path>) -> Result<(), String> {
    let supplied = path.ok_or("no independently supplied isolation attestation")?;
    if fs::symlink_metadata(supplied)
        .map_err(|e| format!("attestation unreadable: {e}"))?
        .file_type()
        .is_symlink()
    {
        return Err("attestation must be a regular file".into());
    }
    let bytes = fs::read(supplied).map_err(|e| format!("attestation unreadable: {e}"))?;
    let digest = hex_digest(&bytes);
    if digest != APPROVED_ATTESTATION_SHA256 {
        return Err(format!(
            "attestation digest {digest} is not Principal-approved {}",
            APPROVED_ATTESTATION_SHA256
        ));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("attestation malformed: {e}"))?;
    let hosted = value.get("githubHosted").and_then(|v| v.as_bool());
    let disposable = value.get("disposable").and_then(|v| v.as_bool());
    let isolated = value.get("programDataIsolated").and_then(|v| v.as_bool());
    if hosted != Some(true) || disposable != Some(true) || isolated != Some(true) {
        return Err("attestation does not assert hosted, disposable and isolated state".into());
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Kernel identity of the actual filesystem object, not merely its path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObjectIdentity {
    volume: u32,
    index: u64,
}

fn object_identity(path: &Path) -> Result<ObjectIdentity, String> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(once(0u16)).collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err("object handle could not be opened for identity verification".into());
    }
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let queried = unsafe { GetFileInformationByHandle(handle, &mut info) };
    unsafe { CloseHandle(handle) };
    if queried != TRUE {
        return Err("kernel object identity could not be queried".into());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("object is a reparse point".into());
    }
    Ok(ObjectIdentity {
        volume: info.dwVolumeSerialNumber,
        index: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    })
}

/// Classification of a ProgramData descendant, used before any mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AncestorState {
    /// Expected on a fresh runner: production will create and protect it.
    ExpectedMissing,
    /// Already present and verified trusted; never deleted by this test.
    TrustedExisting,
    /// A junction/symlink substitution — always refused.
    UnsafeReparse,
    /// Present but the Administrators DACL could not be confirmed.
    UntrustedAcl,
    /// State could not be established — fail closed.
    UnverifiableAcl,
}

fn classify_ancestor(path: &Path) -> Result<AncestorState, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            if reject_reparse(path).is_err() {
                return Ok(AncestorState::UnsafeReparse);
            }
            match dacl_grants_full_control_to(path, SID_ADMINISTRATORS) {
                Ok(true) => Ok(AncestorState::TrustedExisting),
                Ok(false) => Ok(AncestorState::UntrustedAcl),
                Err(_) => Ok(AncestorState::UnverifiableAcl),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AncestorState::ExpectedMissing),
        Err(e) => Err(format!("ancestor state unverifiable: {e}")),
    }
}

/// A verified atomic-rename transition: the kernel object must actually change
/// and the canonical document generation must advance.
fn verify_identity_transition(
    previous: ObjectIdentity,
    observed: ObjectIdentity,
    previous_generation: u64,
    observed_generation: u64,
) -> Result<(), String> {
    if observed == previous {
        return Err("no replacement observed: leaf identity did not change".into());
    }
    if observed_generation <= previous_generation {
        return Err(format!(
            "generation did not advance: {previous_generation} -> {observed_generation}"
        ));
    }
    Ok(())
}

/// RAII owner for the protected registry. The test-only cross-process mutex is
/// acquired BEFORE any protected-path state is inspected and held for the
/// whole lifetime. It is NOT the production writer lock (see E3 note below).
struct OwnedRegistry {
    path: PathBuf,
    /// Missing ancestors recorded deepest-first during preflight.
    new_dirs: Vec<PathBuf>,
    /// Kernel identity of each directory once production has created it.
    dir_identities: Vec<(PathBuf, ObjectIdentity)>,
    identity: Option<ObjectIdentity>,
    history: Vec<(u64, ObjectIdentity)>,
    residual_state: Option<String>,
    _lock: registry::windows::TestMutexGuard,
    cleaned: bool,
}

impl OwnedRegistry {
    fn try_acquire(path: &Path, expected: &Path, ancestor_root: &Path) -> Result<Self, String> {
        if !path.is_absolute()
            || path.to_string_lossy().to_lowercase() != expected.to_string_lossy().to_lowercase()
        {
            return Err("substitute registry path".into());
        }
        // Lock first: no protected state is read until exclusive ownership is held.
        let lock = acquire_test_mutex(&test_mutex_name(expected))
            .map_err(|e| format!("cross-process lock unavailable: {}", e.as_str()))?;
        match fs::symlink_metadata(path) {
            Ok(_) => return Err("pre-existing protected registry — refusing to mutate".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("registry state unverifiable: {e}")),
        }
        let root = ancestor_root;
        if reject_reparse(root).is_err() {
            return Err("ProgramData root is a reparse point".into());
        }
        if dacl_grants_full_control_to(root, SID_ADMINISTRATORS).is_err() {
            return Err("ProgramData ACL cannot be verified".into());
        }
        let mut new_dirs = Vec::new();
        let mut current = path.parent();
        while let Some(dir) = current {
            if dir == root {
                break;
            }
            match classify_ancestor(dir)? {
                AncestorState::ExpectedMissing => new_dirs.push(dir.to_path_buf()),
                AncestorState::TrustedExisting => {}
                AncestorState::UnsafeReparse => {
                    return Err(format!("unsafe reparse ancestor {}", dir.display()));
                }
                AncestorState::UntrustedAcl => {
                    return Err(format!("untrusted ACL on ancestor {}", dir.display()));
                }
                AncestorState::UnverifiableAcl => {
                    return Err(format!("unverifiable ACL on ancestor {}", dir.display()));
                }
            }
            current = dir.parent();
        }
        Ok(Self {
            path: path.to_path_buf(),
            new_dirs,
            dir_identities: Vec::new(),
            identity: None,
            history: Vec::new(),
            residual_state: None,
            _lock: lock,
            cleaned: false,
        })
    }

    /// After production has created/verified its ancestors, confirm the parent
    /// is a trusted non-reparse object, then bind to the exact installed leaf.
    fn record_created_identity(&mut self, generation: u64) -> Result<(), String> {
        let parent = self.path.parent().ok_or("registry path has no parent")?;
        if reject_reparse(parent).is_err() {
            return Err("created registry parent is a reparse point".into());
        }
        if dacl_grants_full_control_to(parent, SID_ADMINISTRATORS) != Ok(true) {
            return Err("created registry parent ACL is not trusted".into());
        }
        // Production has now created the previously missing ancestors. Bind
        // their real identities before any of them may ever be removed.
        let mut dir_identities = Vec::new();
        for dir in &self.new_dirs {
            if reject_reparse(dir).is_err() {
                return Err(format!(
                    "created ancestor {} is a reparse point",
                    dir.display()
                ));
            }
            dir_identities.push((dir.clone(), object_identity(dir)?));
        }
        let identity = object_identity(&self.path)?;
        self.dir_identities = dir_identities;
        self.history.push((generation, identity));
        self.identity = Some(identity);
        Ok(())
    }

    /// A partial install or failed identity capture leaves state this test
    /// cannot prove it owns. Mark it for manual investigation and disable all
    /// deletion — never guess at ownership to make cleanup look complete.
    fn mark_partial_failure(&mut self, stage: &str) {
        self.residual_state = Some(stage.to_owned());
        self.cleaned = true;
    }

    /// Verify the bound object is still the one this test created/approved.
    fn verify_current_identity(&self, generation: u64) -> Result<(), String> {
        let expected = self.history.last().ok_or("no identity has been recorded")?;
        if expected.0 != generation {
            return Err(format!(
                "recorded generation {} != expected {generation}",
                expected.0
            ));
        }
        let observed = object_identity(&self.path)?;
        if observed != expected.1 {
            return Err("unexplained identity substitution before approved update".into());
        }
        Ok(())
    }

    /// Bind cleanup to the verified object produced by an approved atomic
    /// replacement. Requires a genuinely new kernel object and an advanced
    /// canonical generation; otherwise fails closed.
    fn bind_updated_identity(&mut self, generation: u64) -> Result<(), String> {
        let previous = self.identity.ok_or("no previous identity recorded")?;
        let previous_generation = self
            .history
            .last()
            .ok_or("no previous generation recorded")?
            .0;
        let observed = object_identity(&self.path)?;
        let installed = load_registry(&self.path)
            .map_err(|e| format!("updated registry invalid: {}", e.as_str()))?;
        verify_identity_transition(
            previous,
            observed,
            previous_generation,
            installed.generation,
        )?;
        if installed.generation != generation {
            return Err(format!(
                "unexpected installed generation {} != {generation}",
                installed.generation
            ));
        }
        self.history.push((generation, observed));
        self.identity = Some(observed);
        Ok(())
    }

    fn cleanup(&mut self) -> Result<(), String> {
        // One cleanup attempt only: no destructive retries.
        self.cleaned = true;
        let Some(recorded) = self.identity else {
            return Err("STAGE2_CLEANUP_FAILURE: no owned object identity recorded".to_owned());
        };
        let Some(parent) = self.path.parent() else {
            return Err("STAGE2_CLEANUP_FAILURE: registry path has no parent".to_owned());
        };
        if reject_reparse(parent).is_err()
            || dacl_grants_full_control_to(parent, SID_ADMINISTRATORS).is_err()
        {
            return Err("STAGE2_CLEANUP_FAILURE: parent reparse or unverifiable ACL".to_owned());
        }
        let current = object_identity(&self.path)
            .map_err(|e| format!("STAGE2_CLEANUP_FAILURE: owned object unverifiable: {e}"))?;
        if current != recorded {
            return Err(format!(
                "STAGE2_CLEANUP_FAILURE: object identity changed at {} — refusing to delete",
                self.path.display()
            ));
        }
        if reject_reparse(&self.path).is_err() {
            return Err("STAGE2_CLEANUP_FAILURE: owned object became a reparse point".into());
        }
        if let Err(e) = fs::remove_file(&self.path) {
            return Err(format!("STAGE2_CLEANUP_FAILURE: leaf removal failed: {e}"));
        }
        if fs::symlink_metadata(&self.path).is_ok() {
            return Err("STAGE2_CLEANUP_FAILURE: owned object still present after removal".into());
        }
        let mut failures = Vec::new();
        // new_dirs is recorded deepest-first. Removing in that order is the
        // only safe order: a parent is never removed while it still contains
        // a child created by this test.
        for dir in self.new_dirs.iter() {
            if fs::symlink_metadata(dir).is_err() {
                continue;
            }
            let Some((_, recorded)) = self.dir_identities.iter().find(|(path, _)| path == dir)
            else {
                failures.push(format!("{}: no recorded directory identity", dir.display()));
                continue;
            };
            if reject_reparse(dir).is_err() {
                failures.push(format!("{}: reparse substitution", dir.display()));
                continue;
            }
            match object_identity(dir) {
                Ok(current) if current == *recorded => {}
                Ok(_) => {
                    failures.push(format!("{}: directory identity changed", dir.display()));
                    continue;
                }
                Err(e) => {
                    failures.push(format!("{}: {e}", dir.display()));
                    continue;
                }
            }
            let mut entries = match fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) => {
                    failures.push(format!("{}: contents unverifiable: {e}", dir.display()));
                    continue;
                }
            };
            if let Some(entry) = entries.next() {
                let name = entry
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .unwrap_or_else(|_| "<unreadable>".to_owned());
                failures.push(format!("{}: unexpected contents {name}", dir.display()));
                continue;
            }
            if let Err(e) = fs::remove_dir(dir) {
                failures.push(format!("{}: {e}", dir.display()));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(format!("STAGE2_CLEANUP_FAILURE: {}", failures.join(" | ")))
        }
    }
}

impl Drop for OwnedRegistry {
    fn drop(&mut self) {
        if let Some(stage) = &self.residual_state {
            eprintln!(
                "PARTIAL_INSTALL_MANUAL_INVESTIGATION_REQUIRED: {stage} at {}; unverified state left untouched",
                self.path.display()
            );
            return;
        }
        if !self.cleaned
            && let Err(e) = self.cleanup()
        {
            eprintln!("{e}");
        }
    }
}

/// RAII owner for a uniquely created temporary directory. Never removes a
/// path it did not atomically create.
struct OwnedTempDir {
    path: PathBuf,
}

impl OwnedTempDir {
    fn create_new(base: &Path, prefix: &str) -> Result<Self, String> {
        for _ in 0..8 {
            let candidate = base.join(unique_suffix(prefix));
            match fs::create_dir(&candidate) {
                Ok(()) => return Ok(Self { path: candidate }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("fixture creation failed: {e}")),
            }
        }
        Err("could not allocate a collision-free fixture".into())
    }
}

impl Drop for OwnedTempDir {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_dir_all(&self.path) {
            eprintln!("STAGE2_CLEANUP_FAILURE: {}: {e}", self.path.display());
        }
    }
}

struct OwnedSentinel {
    path: PathBuf,
}

impl OwnedSentinel {
    fn create_new(base: &Path) -> Result<Self, String> {
        for _ in 0..8 {
            let candidate = base.join(unique_suffix("pr341-stage2-unowned-sentinel"));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    if let Err(e) = file.write_all(b"do not delete") {
                        let _ = fs::remove_file(&candidate);
                        return Err(format!("sentinel write failed: {e}"));
                    }
                    return Ok(Self { path: candidate });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("sentinel creation failed: {e}")),
            }
        }
        Err("could not allocate a collision-free sentinel".into())
    }
}

impl Drop for OwnedSentinel {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_file(&self.path) {
            eprintln!("STAGE2_CLEANUP_FAILURE: {}: {e}", self.path.display());
        }
    }
}

fn grant_full_control(path: &Path, sid: &str) {
    let output = Command::new("icacls")
        .arg(path)
        .arg("/grant")
        .arg(format!("*{sid}:(OI)(CI)F"))
        .output()
        .expect("icacls must be available on the hosted runner");
    assert!(
        output.status.success(),
        "ACL setup failure must fail the test, never skip: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn grant_everyone_full_control(path: &Path) {
    grant_full_control(path, SID_EVERYONE);
}

#[test]
fn stage2_runtime_attestation_is_fail_closed() {
    // Deterministic negative control: no privileged effects, no opt-in needed.
    assert!(
        runtime_isolation_attestation(None).is_err(),
        "missing attestation must refuse"
    );
    let base = PathBuf::from(
        std::env::var("RUNNER_TEMP")
            .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned()),
    );
    let dir = OwnedTempDir::create_new(&base, "pr341-stage2-attestation")
        .expect("create attestation fixture");
    let forged = dir.path.join("forged.json");
    fs::write(
        &forged,
        r#"{"githubHosted":true,"disposable":true,"programDataIsolated":true}"#,
    )
    .expect("write forged attestation");
    assert!(
        runtime_isolation_attestation(Some(&forged)).is_err(),
        "an unapproved attestation must refuse even with spoofed environment claims"
    );
}

#[test]
fn stage2_preexisting_registry_is_refused() {
    let base = PathBuf::from(
        std::env::var("RUNNER_TEMP")
            .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned()),
    );
    let dir = OwnedTempDir::create_new(&base, "pr341-stage2-preexisting").expect("create fixture");
    let leaf = dir.path.join("registry.json");
    fs::write(&leaf, b"pre-existing").expect("create pre-existing leaf");
    assert!(
        OwnedRegistry::try_acquire(&leaf, &leaf, &dir.path).is_err(),
        "a pre-existing target must be a hard refusal"
    );
    assert_eq!(fs::read(&leaf).expect("leaf intact"), b"pre-existing");
}

#[test]
fn stage2_temp_fixture_collision_is_refused() {
    let base = PathBuf::from(
        std::env::var("RUNNER_TEMP")
            .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned()),
    );
    let dir = OwnedTempDir::create_new(&base, "pr341-stage2-collision").expect("create fixture");
    let occupied = dir.path.join("occupied");
    fs::create_dir(&occupied).expect("create occupying directory");
    // A collision is detected by create_new semantics; never remove or inherit it.
    assert!(
        fs::create_dir(&occupied).is_err(),
        "create_dir must fail on collision"
    );
    assert!(occupied.is_dir(), "pre-existing directory must survive");
}

#[test]
fn stage2_cleanup_failure_is_reported() {
    let dir = OwnedTempDir::create_new(&temp_base(), "pr341-stage2-cleanup").expect("fixture");
    let leaf = dir.path.join("registry.json");
    let mut owned =
        OwnedRegistry::try_acquire(&leaf, &leaf, &dir.path).expect("acquire owned fixture guard");
    // No identity was recorded: cleanup must fail closed and leave objects alone.
    let result = owned.cleanup();
    assert!(
        result.is_err(),
        "cleanup failure must be reported visibly: {result:?}"
    );
    assert!(!leaf.exists(), "no object existed before cleanup");
}

#[test]
fn stage2_cleanup_rejects_replaced_object() {
    let dir = OwnedTempDir::create_new(&temp_base(), "pr341-stage2-replace").expect("fixture");
    grant_full_control(&dir.path, SID_ADMINISTRATORS);
    let leaf = dir.path.join("registry.json");
    let mut owned =
        OwnedRegistry::try_acquire(&leaf, &leaf, &dir.path).expect("acquire owned fixture guard");
    fs::write(&leaf, b"owned-by-this-test").expect("create owned object");
    owned
        .record_created_identity(1)
        .expect("record real object identity");

    // External substitution: same path, different kernel object.
    fs::remove_file(&leaf).expect("remove owned object");
    fs::write(&leaf, b"replacement").expect("create replacement object");
    let result = owned.cleanup();
    assert!(
        result.is_err(),
        "a replaced object must not be deleted: {result:?}"
    );
    assert_eq!(
        fs::read(&leaf).expect("replacement survives"),
        b"replacement",
        "unexpected object must be left untouched"
    );
}

#[test]
fn stage2_cleanup_rejects_reparse_substitution() {
    let dir = OwnedTempDir::create_new(&temp_base(), "pr341-stage2-reparse").expect("fixture");
    grant_full_control(&dir.path, SID_ADMINISTRATORS);
    let leaf = dir.path.join("registry.json");
    let target = dir.path.join("elsewhere.txt");
    let mut owned =
        OwnedRegistry::try_acquire(&leaf, &leaf, &dir.path).expect("acquire owned fixture guard");
    fs::write(&leaf, b"owned-by-this-test").expect("create owned object");
    owned
        .record_created_identity(1)
        .expect("record real object identity");

    fs::remove_file(&leaf).expect("remove owned object");
    fs::write(&target, b"do not follow").expect("create substitution target");
    std::os::windows::fs::symlink_file(&target, &leaf).expect("create file symlink");
    let result = owned.cleanup();
    assert!(
        result.is_err(),
        "a reparse substitution must never be deleted: {result:?}"
    );
    assert!(
        fs::symlink_metadata(&leaf)
            .expect("symlink survives")
            .file_type()
            .is_symlink(),
        "substitution must be left in place"
    );
}

#[test]
fn stage2_in_process_lock_blocks_then_acquires() {
    // Deterministic D1 regression: contention is tested WITHOUT an owned
    // registry guard, so no thread waits on a lock the test itself retains.
    let name = test_mutex_name(&static_preflight().expect("static preflight"));
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let holder_name = name.clone();
    let holder = thread::spawn(move || {
        let _guard = acquire_test_mutex(&holder_name).expect("holder lock");
        held_tx.send(()).expect("signal held");
        let _ = release_rx.recv();
    });
    held_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("holder acquired");

    let (acquired_tx, acquired_rx) = mpsc::channel();
    let contender_name = name;
    let contender = thread::spawn(move || {
        let _guard = acquire_test_mutex(&contender_name).expect("contender lock");
        acquired_tx.send(()).expect("signal acquired");
    });
    assert!(
        acquired_rx
            .recv_timeout(Duration::from_millis(250))
            .is_err(),
        "contender must block while another holder owns the mutex"
    );
    release_tx.send(()).expect("release holder");
    acquired_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("contender acquired only after release");
    holder.join().expect("holder clean exit");
    contender.join().expect("contender clean exit");
}

#[test]
fn stage2_ancestor_classification_missing_then_trusted() {
    let base = OwnedTempDir::create_new(&temp_base(), "pr341-stage2-ancestors").expect("fixture");
    grant_full_control(&base.path, SID_ADMINISTRATORS);
    let parent = base.path.join("missing-parent");
    assert_eq!(
        classify_ancestor(&parent).expect("missing classification"),
        AncestorState::ExpectedMissing,
        "a missing registry descendant must be a permitted first-install case"
    );
    fs::create_dir(&parent).expect("create descendant");
    grant_full_control(&parent, SID_ADMINISTRATORS);
    assert_eq!(
        classify_ancestor(&parent).expect("existing classification"),
        AncestorState::TrustedExisting,
        "an existing verified descendant must be trusted and never deleted"
    );
}

#[test]
fn stage2_ancestor_reparse_is_refused() {
    let base =
        OwnedTempDir::create_new(&temp_base(), "pr341-stage2-ancestor-reparse").expect("fixture");
    let target = base.path.join("target");
    fs::create_dir(&target).expect("target");
    let link = base.path.join("substitute");
    std::os::windows::fs::symlink_dir(&target, &link).expect("junction");
    assert_eq!(
        classify_ancestor(&link).expect("reparse classification"),
        AncestorState::UnsafeReparse,
        "a junction in place of an ancestor must be refused"
    );
}

#[test]
fn stage2_identity_transition_requires_replacement() {
    let object = ObjectIdentity {
        volume: 1,
        index: 10,
    };
    assert!(
        verify_identity_transition(object, object, 1, 2).is_err(),
        "identical kernel identity proves no atomic rename occurred"
    );
    let replacement = ObjectIdentity {
        volume: 1,
        index: 11,
    };
    assert!(
        verify_identity_transition(object, replacement, 1, 1).is_err(),
        "a replacement without generation advance must be refused"
    );
    assert!(
        verify_identity_transition(object, replacement, 1, 2).is_ok(),
        "a distinct object with an advanced generation is a valid transition"
    );
}

#[test]
fn stage2_failed_identity_transition_missing_object_is_refused() {
    let base = OwnedTempDir::create_new(&temp_base(), "pr341-stage2-transition").expect("fixture");
    grant_full_control(&base.path, SID_ADMINISTRATORS);
    let leaf = base.path.join("registry.json");
    let mut owned =
        OwnedRegistry::try_acquire(&leaf, &leaf, &base.path).expect("acquire owned fixture guard");
    fs::write(&leaf, b"generation-1").expect("create generation-1 object");
    owned
        .record_created_identity(1)
        .expect("bind generation-1 identity");
    fs::remove_file(&leaf).expect("simulate failed replacement");
    assert!(
        owned.bind_updated_identity(2).is_err(),
        "a missing object after an attempted update must fail closed"
    );
}

#[test]
fn stage2_multilevel_cleanup_removes_deepest_first() {
    let base = OwnedTempDir::create_new(&temp_base(), "pr341-stage2-multilevel").expect("fixture");
    grant_full_control(&base.path, SID_ADMINISTRATORS);
    let leaf = base.path.join("a/b/c/registry.json");
    let mut owned =
        OwnedRegistry::try_acquire(&leaf, &leaf, &base.path).expect("acquire owned guard");
    fs::create_dir_all(leaf.parent().expect("nested parent")).expect("nested directories");
    for dir in ["a", "a/b", "a/b/c"] {
        grant_full_control(&base.path.join(dir), SID_ADMINISTRATORS);
    }
    fs::write(&leaf, b"owned").expect("owned leaf");
    owned.record_created_identity(1).expect("record identities");
    assert_eq!(owned.new_dirs.len(), 3, "three owned ancestors recorded");

    owned.cleanup().expect("deepest-first cleanup must succeed");
    assert!(
        !base.path.join("a").exists(),
        "all nested owned directories must be removed"
    );
    assert!(base.path.exists(), "pre-existing parent must survive");
}

#[test]
fn stage2_cleanup_preserves_unexpected_contents() {
    let base = OwnedTempDir::create_new(&temp_base(), "pr341-stage2-unexpected").expect("fixture");
    grant_full_control(&base.path, SID_ADMINISTRATORS);
    let leaf = base.path.join("a/b/c/registry.json");
    let mut owned =
        OwnedRegistry::try_acquire(&leaf, &leaf, &base.path).expect("acquire owned guard");
    fs::create_dir_all(leaf.parent().expect("nested parent")).expect("nested directories");
    for dir in ["a", "a/b", "a/b/c"] {
        grant_full_control(&base.path.join(dir), SID_ADMINISTRATORS);
    }
    fs::write(&leaf, b"owned").expect("owned leaf");
    owned.record_created_identity(1).expect("record identities");
    let unexpected = base.path.join("a/b/unowned.txt");
    fs::write(&unexpected, b"not ours").expect("unowned content");

    let result = owned.cleanup();
    assert!(
        result.is_err(),
        "unexpected contents must block directory removal: {result:?}"
    );
    assert!(unexpected.exists(), "unowned content must be preserved");
    assert!(
        base.path.join("a/b").exists(),
        "the directory holding unexpected content must be preserved"
    );
    assert!(
        base.path.join("a").exists(),
        "pre-existing parent must survive"
    );
}

#[test]
fn stage2_cleanup_refuses_directory_reparse_substitution() {
    let base = OwnedTempDir::create_new(&temp_base(), "pr341-stage2-dir-reparse").expect("fixture");
    grant_full_control(&base.path, SID_ADMINISTRATORS);
    let leaf = base.path.join("a/b/c/registry.json");
    let mut owned =
        OwnedRegistry::try_acquire(&leaf, &leaf, &base.path).expect("acquire owned guard");
    fs::create_dir_all(leaf.parent().expect("nested parent")).expect("nested directories");
    for dir in ["a", "a/b", "a/b/c"] {
        grant_full_control(&base.path.join(dir), SID_ADMINISTRATORS);
    }
    fs::write(&leaf, b"owned").expect("owned leaf");
    owned.record_created_identity(1).expect("record identities");

    let owned_dir = base.path.join("a/b/c");
    let substituted = base.path.join("elsewhere");
    fs::create_dir(&substituted).expect("substitution target");
    fs::remove_dir_all(&owned_dir).expect("remove owned directory");
    std::os::windows::fs::symlink_dir(&substituted, &owned_dir).expect("junction");
    let result = owned.cleanup();
    assert!(
        result.is_err(),
        "a reparse substitution must be refused: {result:?}"
    );
    assert!(
        fs::symlink_metadata(&owned_dir)
            .expect("junction survives")
            .file_type()
            .is_symlink(),
        "unexpected substitution must be left in place"
    );
}

#[test]
fn stage2_partial_install_leaves_unverified_state() {
    let base = OwnedTempDir::create_new(&temp_base(), "pr341-stage2-partial").expect("fixture");
    grant_full_control(&base.path, SID_ADMINISTRATORS);
    let leaf = base.path.join("a/b/c/registry.json");
    let mut owned =
        OwnedRegistry::try_acquire(&leaf, &leaf, &base.path).expect("acquire owned guard");

    // Simulate a provisioning failure after creating some state but before
    // identity capture. None of it is provably owned.
    fs::create_dir_all(leaf.parent().expect("nested parent")).expect("partial directories");
    fs::write(&leaf, b"partial").expect("partial leaf");
    owned.mark_partial_failure("simulated first-install failure");
    assert!(
        owned.residual_state.is_some(),
        "partial failure must be recorded"
    );
    let result = owned.cleanup();
    assert!(result.is_err(), "unverified state must not be deleted");
    assert!(leaf.exists(), "partial leaf preserved for investigation");
    assert!(
        base.path.join("a").exists(),
        "partial directories preserved for investigation"
    );
}

#[test]
fn stage2_lock_child_process_helper() {
    // Invoked only by stage2_cross_process_lock_is_enforced with a private env.
    if std::env::var_os(LOCK_CHILD_VAR).is_none() {
        eprintln!("NOT_RUN — STAGE2_DISABLED: lock child helper");
        return;
    }
    let path = static_preflight().expect("static preflight");
    let name = test_mutex_name(&path);
    let _guard = acquire_test_mutex(&name).expect("child lock");
    let marker = PathBuf::from(std::env::var(LOCK_MARKER_VAR).expect("lock marker path"));
    fs::write(&marker, b"acquired").expect("write lock marker");
    thread::sleep(Duration::from_millis(1200));
}

#[test]
fn stage2_cross_process_lock_is_enforced() {
    require_stage2!();
    let path = static_preflight().expect("static preflight");
    let name = test_mutex_name(&path);
    let base = temp_base();
    let marker_dir = OwnedTempDir::create_new(&base, "pr341-stage2-lock").expect("marker dir");
    let marker = marker_dir.path.join("acquired.marker");

    let _guard = acquire_test_mutex(&name).expect("parent lock");
    let child = Command::new(std::env::current_exe().expect("current exe"))
        .arg("--exact")
        .arg("stage2_lock_child_process_helper")
        .arg("--nocapture")
        .env(LOCK_CHILD_VAR, "1")
        .env(LOCK_MARKER_VAR, &marker)
        .spawn()
        .expect("spawn lock contender child");

    thread::sleep(Duration::from_millis(500));
    assert!(
        !marker.exists(),
        "a second PROCESS must not acquire the shared lock while the first holds it"
    );
    drop(_guard);
    let deadline = SystemTime::now() + Duration::from_secs(8);
    while !marker.exists() && SystemTime::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    assert!(marker.exists(), "child must acquire the lock after release");
    assert!(
        child
            .wait_with_output()
            .expect("child result")
            .status
            .success(),
        "contender child must exit cleanly"
    );
}

#[test]
fn stage2_deterministic_privileged_scenario() {
    require_stage2!();
    let expected = static_preflight().expect("static preflight");
    let supplied = std::env::var_os(ATTESTATION_PATH_VAR).map(PathBuf::from);
    runtime_isolation_attestation(supplied.as_deref())
        .expect("Stage 2 remains BLOCKED without independently approved isolation attestation");
    let authority = require_provisioning_authority().expect("real elevated provisioning token");
    assert!(
        authority.contains(SID_ADMINISTRATORS) || authority.contains("S-1-5-18"),
        "provisioning authority must be Administrators or SYSTEM"
    );

    let root = registry::programdata_root().expect("ProgramData root");
    let mut owned = OwnedRegistry::try_acquire(&expected, &expected, &root)
        .expect("hard refusal on pre-existing or unverifiable protected state");
    let sid = SID_ADMINISTRATORS;
    let first = document(1, vec![binding(sid, "stage2-owner", false, 1)]);
    if let Err(e) = provision_registry(&expected, &first) {
        owned.mark_partial_failure("first-install provisioning");
        panic!("elevated first install failed: {e:?}");
    }
    if let Err(e) = owned.record_created_identity(1) {
        owned.mark_partial_failure("first-install identity capture");
        panic!("bind cleanup to installed object failed: {e}");
    }

    assert_eq!(load_registry(&expected).expect("installed bytes"), first);
    assert!(validate_registry_location(&expected).is_ok());
    assert_eq!(dacl_grants_full_control_to(&expected, sid), Ok(true));
    assert_eq!(
        dacl_grants_full_control_to(&expected, SID_EVERYONE),
        Ok(false)
    );
    let entries: Vec<_> = fs::read_dir(expected.parent().unwrap())
        .expect("parent")
        .filter_map(Result::ok)
        .collect();
    assert_eq!(entries.len(), 1, "atomic publication leaves no temp files");

    let revoked = document(2, vec![binding(sid, "stage2-owner", true, 2)]);
    if let Err(e) = owned.verify_current_identity(1) {
        owned.mark_partial_failure("pre-update identity verification");
        panic!("expected generation-1 state before update: {e}");
    }
    let first_identity = owned.identity.expect("generation-1 identity");
    if let Err(e) = provision_registry(&expected, &revoked) {
        owned.mark_partial_failure("generation-2 update provisioning");
        panic!("monotonic revocation update failed: {e:?}");
    }
    if let Err(e) = owned.bind_updated_identity(2) {
        owned.mark_partial_failure("generation-2 identity capture");
        panic!("verified generation-2 identity transition failed: {e}");
    }
    let second_identity = owned.identity.expect("generation-2 identity");
    assert_ne!(
        first_identity, second_identity,
        "atomic rename must produce a distinct kernel object"
    );
    assert_eq!(owned.history.len(), 2, "both generations have provenance");
    assert_eq!(
        resolve_owner(&expected, sid),
        Ok(None),
        "revocation is permanent"
    );
    assert_eq!(
        provision_registry(&expected, &first),
        Err(Refusal::NonMonotonicUpdate),
        "stale generation refused"
    );
    assert_eq!(
        resolve_owner(&expected, SID_EVERYONE),
        Ok(None),
        "no cross-owner authorization"
    );

    // Contention is verified in stage2_in_process_lock_blocks_then_acquires
    // and stage2_cross_process_lock_is_enforced, never while this exclusive
    // registry guard is held.

    let base = temp_base();
    let acl_dir = OwnedTempDir::create_new(&base, "pr341-stage2-acl").expect("ACL fixture");
    grant_everyone_full_control(&acl_dir.path);
    assert!(validate_file_security(&acl_dir.path).is_err());
    let junction = acl_dir.path.join("junction");
    std::os::windows::fs::symlink_dir(&acl_dir.path, &junction).expect("junction");
    assert!(reject_reparse(&junction.join(registry::REGISTRY_FILE_NAME)).is_err());

    let sentinel = OwnedSentinel::create_new(&base).expect("unique sentinel");
    assert!(sentinel.path.exists());

    owned
        .cleanup()
        .expect("owned cleanup must succeed and be visible");
    assert!(!expected.exists(), "only test-owned registry removed");
    assert!(sentinel.path.exists(), "unowned sentinel survives cleanup");
}

/// E3 scope note: `acquire_test_mutex` creates a TEST-ONLY named mutex with a
/// restricted Local DACL. Production `provision_registry` uses the separate
/// private `acquire_update_lock` / `lock_name(path, true)` identity. The two
/// mutex tests in this file prove synchronization between test participants
/// only; they do NOT prove production writer exclusion.
#[test]
fn stage2_production_writer_lock_evidence_is_pending() {
    eprintln!(
        "PENDING — PRODUCTION_WRITER_LOCK_NOT_COVERED: production acquire_update_lock evidence requires an approved runtime environment"
    );
}
