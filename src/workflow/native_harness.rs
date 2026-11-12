//! Governed native workspace-inspection adapter (issue #154, slice F).
//!
//! [`NativeHarness`] is the second real execution adapter conforming to
//! [`ExecutionHarness`]: an in-process, read-only workspace file inspector
//! that reports a deterministic, structured fact about one governed file —
//! its verified byte size and SHA-256 content digest. It is NOT a
//! declaration-only registry entry and NOT the synthetic conformance probe
//! (`harness.native.probe`).
//!
//! Effect discipline (no bypass around #117/#118):
//!
//! - The adapter never reads a file itself. Its `run` invokes the declared
//!   `harness.native.inspect` capability through [`HarnessDispatch::call`],
//!   the only sanctioned effect path. The capability handler
//!   ([`native_capability`]) is registered by the caller in the
//!   [`crate::workflow::node_runner::CapabilityRegistry`] alongside the
//!   reviewed governance permit that authorizes it.
//! - Strict argument validation: exactly one `path` argument, a string,
//!   relative, non-empty, and resource-bounded.
//! - Path confinement: the requested path must be relative (an absolute or
//!   Windows drive/UNC form fails), must contain no `..` segment on any
//!   platform. The file is opened through a handle-verified operation and
//!   its identity, type, and size are validated on the retained handle
//!   before any byte is read (Windows: the opened handle's final path must
//!   lie beneath the canonical workspace root, following symlinks whose
//!   resolved target is inside it; Unix: a dirfd-relative component walk
//!   with a strict no-follow policy — a final-component symlink is refused
//!   rather than followed — plus expected-identity capture and
//!   verification of the opened handle). Unix is intentionally stricter
//!   than Windows here. A concurrent path/symlink swap can only make the
//!   inspection refuse — never redirect the read outside the workspace.
//!   Symlink escapes, directories, missing files, and files above the size
//!   budget are refused before any read.
//! - The handler only reads bytes. It never writes to the workspace, never
//!   follows content outside the governance boundary, and returns a digest —
//!   file CONTENT never leaves the handler, so a secret-bearing file cannot
//!   leak its contents into evidence.
//! - File-size and argument/output limits are enforced by the handler on
//!   every invocation; there is no unbounded mode. Exceeding a budget is a
//!   typed refusal and never reports success.
//!
//! Portable continuity: the adapter owns no durable work state. Its
//! `checkpoint`/`resume` bind the harness identity + revision to a digest
//! of the canonical [`PortableWorkState`](crate::workflow::portable_state::PortableWorkState),
//! which lives outside the adapter. Resume re-verifies the checkpoint and
//! refuses incompatible harness/revision/digest bindings. Any later step
//! can simply re-run against the same portable state, so no hidden
//! session/process state is required for continuation. Evidence correlation:
//! every outcome carries the journaled evidence digest.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::workflow::artifact_integrity::sha256_hex;
use crate::workflow::execution_harness::{
    ExecutionHarness, HARNESS_SCHEMA_VERSION, HarnessCheckpointV1, HarnessContinuation,
    HarnessDeclarationV1, HarnessDispatch, HarnessEvent, HarnessFamily, HarnessOutcome,
    HarnessStep,
};
use crate::workflow::node_runner::Capability;
use crate::workflow::redaction::Redactor;
use crate::workflow::soma::capability::{
    AdapterFamily, Identity, Locality, RuntimeCapabilitySet, SUPPORTED_CAPABILITY_SCHEMA_VERSION,
};
use crate::workflow::soma::profile::{Boundaries, Boundary};
use crate::workflow::workspace::ADAPTER_REVISION;

/// Implementation-owned harness id.
pub const NATIVE_HARNESS_ID: &str = "lite.harness.native.inspect";
/// The single declared capability (registry key + declared capability).
pub const NATIVE_INSPECT_CAPABILITY: &str = "harness.native.inspect";
/// Identity name/revision of the implementation.
pub const NATIVE_IDENTITY_NAME: &str = "lite-native-inspect";
pub const NATIVE_REVISION: &str = "v1";

/// Bound for one inspection read window; hashing is incremental so memory
/// never grows with the file.
const READ_CHUNK: usize = 8192;

/// Configuration for the governed native-inspection handler.
///
/// All bounds are enforced by the handler on every invocation; there is no
/// unbounded mode.
#[derive(Clone)]
pub struct NativeInspectConfig {
    /// Files larger than this are refused before any read.
    pub max_file_bytes: u64,
    /// Bytes of serialized arguments accepted per invocation.
    pub max_args_bytes: usize,
    /// Bytes of serialized result documents accepted per invocation.
    pub max_output_bytes: usize,
    /// Bound for the inspected `path` argument value.
    pub max_path_chars: usize,
    /// Known secret values, redacted verbatim from the result document.
    pub known_secrets: Vec<String>,
}

impl Default for NativeInspectConfig {
    fn default() -> Self {
        Self {
            max_file_bytes: 8 * 1024 * 1024,
            max_args_bytes: 64 * 1024,
            max_output_bytes: 64 * 1024,
            max_path_chars: 4096,
            known_secrets: Vec::new(),
        }
    }
}

impl NativeInspectConfig {
    pub fn max_file_bytes(mut self, max_file_bytes: u64) -> Self {
        self.max_file_bytes = max_file_bytes;
        self
    }

    pub fn max_args_bytes(mut self, max_args_bytes: usize) -> Self {
        self.max_args_bytes = max_args_bytes;
        self
    }

    pub fn max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes;
        self
    }

    pub fn known_secrets(mut self, known_secrets: Vec<String>) -> Self {
        self.known_secrets = known_secrets;
        self
    }
}

/// The implementation-owned, sealed declaration for the native harness.
/// Shared by [`NativeHarness`] and the canonical catalog inventory so the
/// two cannot diverge.
pub fn native_declaration() -> HarnessDeclarationV1 {
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: NATIVE_HARNESS_ID.to_string(),
        identity: Identity {
            name: NATIVE_IDENTITY_NAME.to_string(),
            revision: NATIVE_REVISION.to_string(),
        },
        family: HarnessFamily::Native,
        declared_capabilities: vec![NATIVE_INSPECT_CAPABILITY.to_string()],
        claimed_capabilities: vec![NATIVE_INSPECT_CAPABILITY.to_string()],
        capabilities: RuntimeCapabilitySet {
            id: "lite.harness.native.inspect.caps".to_string(),
            schema_version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            identity: Identity {
                name: NATIVE_IDENTITY_NAME.to_string(),
                revision: NATIVE_REVISION.to_string(),
            },
            declared_at: "2026-11-08T00:00:00Z".to_string(),
            spec_versions: vec![SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string()],
            adapter_families: vec![
                AdapterFamily {
                    family: "git-worktree".to_string(),
                    revision: ADAPTER_REVISION.to_string(),
                },
                AdapterFamily {
                    family: "existing-read-only".to_string(),
                    revision: ADAPTER_REVISION.to_string(),
                },
            ],
            effect_classes: vec!["filesystem-read".to_string()],
            workspace: Some(Boundaries {
                workspace: Some(Boundary::Read),
                filesystem: None,
                git: None,
                process: None,
                network: None,
                secret: None,
            }),
            checkpoint_import: Some(true),
            checkpoint_export: Some(true),
            resume_support: Some(true),
            // The inspection result is one terminal evidence document; nothing
            // is streamed incrementally, so streaming is not claimed.
            event_streaming: Some(false),
            read_projection: Some(true),
            locality: Some(Locality::Local),
            privacy_classes: Vec::new(),
            max_structured_input: None,
            max_structured_output: None,
            budget_ceilings: None,
            evidence_attachment: Some(true),
            validation: Some(true),
            approval_escalation: Some(false),
        },
        content_digest: None,
    }
    .sealed()
}

/// The governed native-inspection adapter.
///
/// The adapter itself never reads a file: `run` invokes the declared
/// capability through the governed dispatch. The capability handler
/// ([`native_capability`]) is registered by the caller in the
/// [`crate::workflow::node_runner::CapabilityRegistry`] alongside the
/// reviewed governance permit that authorizes it.
pub struct NativeHarness {
    declaration: HarnessDeclarationV1,
}

impl Default for NativeHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeHarness {
    pub fn new() -> Self {
        Self {
            declaration: native_declaration(),
        }
    }
}

#[async_trait::async_trait]
impl ExecutionHarness for NativeHarness {
    fn declaration(&self) -> &HarnessDeclarationV1 {
        &self.declaration
    }

    async fn run(
        &self,
        step: HarnessStep,
        dispatch: &mut HarnessDispatch<'_>,
    ) -> Result<HarnessOutcome> {
        // The ONLY effect path: `call` refuses capabilities outside this
        // harness's declaration before any file can be touched. The governed
        // request's known secrets ride along so the handler redacts them
        // verbatim — the redaction boundary is authoritative, not config
        // folklore.
        let mut args = step.args.clone();
        if let Some(map) = args.as_object_mut() {
            map.insert(
                "knownSecrets".to_string(),
                serde_json::json!(dispatch.known_secrets()),
            );
        }
        let run_outcome = dispatch.call(&step.capability, args, &step.step_id)?;
        let output = run_outcome.output.clone();

        // The inspection result is machine-readable JSON produced by the
        // handler; an unparseable document is not evidence of a successful
        // inspection.
        let parsed: serde_json::Value = serde_json::from_str(&output)
            .context("native capability output must be a JSON result document")?;
        if parsed.get("path").is_none() || parsed.get("sha256").is_none() {
            anyhow::bail!("native inspection result is missing required path/sha256 fields");
        }

        let events = vec![
            HarnessEvent {
                sequence: 0,
                correlation_id: step.step_id.clone(),
                kind: "started".to_string(),
                detail_digest: sha256_hex(format!("{}:started", step.step_id).as_bytes()),
            },
            HarnessEvent {
                sequence: 1,
                correlation_id: step.step_id.clone(),
                kind: "completed".to_string(),
                detail_digest: sha256_hex(format!("{}:completed", step.step_id).as_bytes()),
            },
        ];
        Ok(HarnessOutcome {
            harness_id: NATIVE_HARNESS_ID.to_string(),
            output,
            evidence_entry_digest: run_outcome.evidence_entry.entry_digest.clone(),
            events,
            content_digest: None,
        }
        .sealed())
    }

    async fn checkpoint(
        &self,
        _outcome: &HarnessOutcome,
        portable_state_digest: &str,
    ) -> Result<HarnessCheckpointV1> {
        // Bind this implementation to the digest of the portable work state
        // that lives in the store — process memory carries no durable truth.
        Ok(HarnessCheckpointV1 {
            schema_version: HARNESS_SCHEMA_VERSION.to_string(),
            harness_id: NATIVE_HARNESS_ID.to_string(),
            harness_revision: NATIVE_REVISION.to_string(),
            portable_state_digest: portable_state_digest.to_string(),
            resume_step: None,
            content_digest: None,
        }
        .sealed())
    }

    async fn resume(
        &self,
        checkpoint: &HarnessCheckpointV1,
        portable_state_digest: &str,
    ) -> Result<HarnessContinuation> {
        // Re-verify the sealed binding, then require exact identity,
        // revision, and portable-state agreement.
        checkpoint.verify()?;
        if checkpoint.harness_id != NATIVE_HARNESS_ID {
            anyhow::bail!(
                "harness checkpoint was exported by {:?}, not by {NATIVE_HARNESS_ID:?}",
                checkpoint.harness_id
            );
        }
        if checkpoint.harness_revision != NATIVE_REVISION {
            anyhow::bail!(
                "harness checkpoint revision {:?} is incompatible with this implementation ({NATIVE_REVISION:?})",
                checkpoint.harness_revision
            );
        }
        if checkpoint.portable_state_digest != portable_state_digest {
            anyhow::bail!(
                "harness checkpoint portable state digest does not match the supplied state"
            );
        }
        Ok(HarnessContinuation {
            harness_id: NATIVE_HARNESS_ID.to_string(),
            harness_revision: NATIVE_REVISION.to_string(),
            portable_state_digest: portable_state_digest.to_string(),
            resume_step: checkpoint.resume_step.clone(),
        })
    }
}

/// Build the governed `harness.native.inspect` capability for a runner's
/// registry, bound to an acquired workspace root.
///
/// The handler is `Fn` (reusable across invocations); each invocation is
/// independently validated and confined to `workspace_root`. Refusals never
/// mutate the workspace and never surface file content.
pub fn native_capability(config: NativeInspectConfig, workspace_root: PathBuf) -> Capability {
    Capability::deterministic(&["path"], move |args| {
        run_native_inspect_guarded(&config, &workspace_root, args)
    })
}

// ---------------------------------------------------------------------------
// Governed inspection
// ---------------------------------------------------------------------------

/// Decoded, strictly validated invocation arguments.
struct InspectArgs {
    path: String,
    known_secrets: Vec<String>,
}

fn decode_args(args: &serde_json::Value, max_args_bytes: usize) -> Result<InspectArgs> {
    let raw = serde_json::to_string(args).context("arguments serialize")?;
    if raw.len() > max_args_bytes {
        bail!(
            "native inspection arguments exceed budget ({} > {max_args_bytes} bytes)",
            raw.len()
        );
    }
    let map = args
        .as_object()
        .context("native inspection arguments must be a JSON object")?;
    for key in map.keys() {
        if key != "path" && key != "knownSecrets" {
            bail!("native inspection rejects unsupported argument {key:?}");
        }
    }
    let path = map
        .get("path")
        .and_then(serde_json::Value::as_str)
        .context("native inspection requires a string 'path' argument")?;
    if path.is_empty() {
        bail!("native inspection rejects an empty path");
    }
    let known_secrets = match map.get("knownSecrets") {
        None => Vec::new(),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(_) => bail!("native inspection 'knownSecrets' must be an array of strings"),
    };
    Ok(InspectArgs {
        path: path.to_string(),
        known_secrets,
    })
}

/// Validate a caller-supplied path as hostile input before any filesystem
/// access. Absolute and `..` forms are refused on every platform; a Windows
/// drive/UNC form is refused even when running on Unix.
fn validate_relative_path(raw: &str, max_path_chars: usize) -> Result<()> {
    if raw.chars().count() > max_path_chars {
        bail!("native inspection path exceeds {max_path_chars} characters");
    }
    if raw.contains('\0') {
        bail!("native inspection path contains a NUL byte");
    }
    if Path::new(raw).is_absolute() || raw.starts_with('/') || raw.starts_with('\\') {
        bail!("native inspection path must be relative: {raw:?}");
    }
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        bail!("native inspection path must be relative (drive form): {raw:?}");
    }
    for segment in raw.split(['/', '\\']) {
        if segment == ".." {
            bail!("native inspection path escapes the workspace: {raw:?}");
        }
    }
    Ok(())
}

/// Identity of an opened file handle. A handle pins its file for its whole
/// lifetime, so a stable identity proves the inspected handle never
/// retargeted between verification and hashing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenFileId {
    /// Volume serial number (Windows) or device id (Unix).
    primary: u64,
    /// File index (Windows) or inode (Unix).
    secondary: u64,
}

/// Identity of the file an opened handle refers to, read from the handle
/// itself (`fstat` on Unix, `GetFileInformationByHandle` on Windows — a
/// handle query, never a pathname stat).
fn open_file_id(file: &std::fs::File) -> Result<OpenFileId> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle as _;
        use winapi::shared::ntdef::HANDLE;
        use winapi::um::fileapi::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        // SAFETY: `info` is valid for the call; on success it is fully
        // initialized.
        let ok = unsafe {
            GetFileInformationByHandle(file.as_raw_handle() as HANDLE, info.as_mut_ptr())
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error())
                .context("native inspection could not verify the opened handle");
        }
        // SAFETY: the call succeeded and fully initialized `info`.
        let info = unsafe { info.assume_init() };
        Ok(OpenFileId {
            primary: u64::from(info.dwVolumeSerialNumber),
            secondary: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        })
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = file
            .metadata()
            .context("native inspection could not verify the opened handle")?;
        Ok(OpenFileId {
            primary: metadata.dev(),
            secondary: metadata.ino(),
        })
    }
    #[cfg(not(any(windows, unix)))]
    {
        unreachable!("open_file_id is only defined on Windows and Unix")
    }
}

/// Open the inspected file beneath the canonical workspace root through a
/// platform handle-verified operation, and return the retained handle plus
/// the size reported by that handle. Validation happens on the opened
/// handle — never by reopening the pathname — so a concurrent path or
/// symlink swap cannot redirect the read outside the workspace: at worst
/// the inspection refuses. Where a platform cannot establish that
/// guarantee, this fails closed instead of degrading to a pathname check.
#[cfg(windows)]
fn open_confined_file(root: &Path, raw: &str) -> Result<(std::fs::File, u64)> {
    open_confined_file_windows(root, raw)
}

#[cfg(unix)]
fn open_confined_file(root: &Path, raw: &str) -> Result<(std::fs::File, u64)> {
    open_confined_file_unix(root, raw)
}

#[cfg(not(any(windows, unix)))]
fn open_confined_file(_root: &Path, _raw: &str) -> Result<(std::fs::File, u64)> {
    bail!("native inspection file confinement is not supported on this platform")
}

// ---------------------------------------------------------------------------
// Windows handle confinement
// ---------------------------------------------------------------------------
//
// `CreateFileW` opens the candidate file (following symlinks and junctions,
// as usual) and returns a handle that pins the file identity from that
// instant on. `GetFinalPathNameByHandleW` then reports the final path of
// the file the HANDLE actually refers to — a swap of the pathname after
// the open cannot change what the handle points at, and a swap before the
// open is caught because the handle's final path resolves outside the
// workspace and the open is refused. The handle is validated (regular
// file, size) before any byte is read, and hashed on the retained handle.

// `GetFinalPathNameByHandleW` relies on the winapi `VOLUME_NAME_DOS` flag
// to select the `\\?\C:\...` DOS form, comparable to a canonicalized root.
// The winapi constant is zero on the target Windows build; a scratch
// probe verified this value returns the resolved `\\?\C:\...` final path —
// including symlink targets opened through a link — which is exactly the
// form `std::fs::canonicalize` produces for the workspace root.

/// Normalize a Windows path string for case-insensitive, separator-
/// insensitive comparison.
#[cfg(windows)]
fn comparable_windows_path(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    path.replace('/', "\\").to_ascii_lowercase()
}

/// Whether the final path of an opened handle lies beneath the canonical
/// workspace root, compared on a whole-component boundary.
#[cfg(windows)]
fn opened_path_is_within(final_path: &str, root: &Path) -> bool {
    let root_s = comparable_windows_path(&root.to_string_lossy());
    let root_s = root_s.trim_end_matches('\\');
    let final_s = comparable_windows_path(final_path);
    final_s == root_s || final_s.starts_with(&format!("{root_s}\\"))
}

/// The final path of the file an open handle refers to, in DOS form. The
/// buffer grows when Win32 reports the required size.
#[cfg(windows)]
fn opened_final_path(handle: std::os::windows::io::RawHandle) -> Result<String> {
    use winapi::shared::ntdef::HANDLE;
    use winapi::um::fileapi::GetFinalPathNameByHandleW;
    use winapi::um::winbase::VOLUME_NAME_DOS;
    let mut buffer: Vec<u16> = vec![0; 1024];
    loop {
        // SAFETY: `buffer` is valid for `buffer.len()` WCHARs; Win32 writes
        // a NUL-terminated string on success and reports the required size
        // (excluding the terminator) when the buffer is too small.
        let required = unsafe {
            GetFinalPathNameByHandleW(
                handle as HANDLE,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                VOLUME_NAME_DOS,
            )
        };
        if required == 0 {
            return Err(std::io::Error::last_os_error())
                .context("native inspection could not resolve the opened file path");
        }
        if (required as usize) < buffer.len() {
            buffer.truncate(required as usize);
            return Ok(String::from_utf16_lossy(&buffer));
        }
        buffer.resize((required as usize) + 1, 0);
    }
}

#[cfg(windows)]
fn open_confined_file_windows(root: &Path, raw: &str) -> Result<(std::fs::File, u64)> {
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, RawHandle};
    use winapi::um::fileapi::{CreateFileW, OPEN_EXISTING};
    use winapi::um::handleapi::INVALID_HANDLE_VALUE;
    use winapi::um::winbase::FILE_FLAG_BACKUP_SEMANTICS;
    use winapi::um::winnt::{FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ};

    let candidate = root.join(raw);
    let wide: Vec<u16> = candidate
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide` is NUL-terminated. Read access with permissive
    // sharing keeps inspection concurrent-read safe; BACKUP_SEMANTICS lets
    // directories open so the regular-file check below refuses them
    // explicitly instead of surfacing an opaque OS error.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::NotFound {
            bail!("native inspection target does not exist: {raw:?}");
        }
        return Err(err)
            .context("native inspection could not open the target file confined to the workspace");
    }
    // SAFETY: the freshly created handle is owned by the caller; wrapping
    // it transfers unique ownership to the `File`.
    let file = unsafe { std::fs::File::from_raw_handle(handle as RawHandle) };
    // Confinement is proven on the OPENED handle: the final path this
    // handle actually refers to must lie beneath the canonical root.
    let final_path = opened_final_path(file.as_raw_handle())
        .context("native inspection could not verify confinement of the opened file")?;
    if !opened_path_is_within(&final_path, root) {
        bail!("native inspection path escapes the workspace: {raw:?}");
    }
    // Type and size come from the opened handle as well.
    let metadata = file
        .metadata()
        .context("native inspection could not stat the opened file")?;
    if !metadata.is_file() {
        bail!("native inspection target is not a regular file: {raw:?}");
    }
    Ok((file, metadata.len()))
}

// ---------------------------------------------------------------------------
// Unix dirfd-walk confinement
// ---------------------------------------------------------------------------
//
// Every step is resolved relative to a retained directory handle (`openat`
// from the parent descriptor), so no later pathname resolution can
// redirect the walk. Intermediate components must be real directories
// (`O_DIRECTORY | O_NOFOLLOW`). The final component is handled with a
// strict no-follow policy: its identity is captured with `fstatat` using
// `AT_SYMLINK_NOFOLLOW` (a final symlink is refused as not-a-regular-file
// instead of being followed), it is opened with `O_NOFOLLOW` (so a swap
// between capture and open cannot retarget the open onto a symlink), and
// the opened handle's `fstat` identity must equal the captured identity
// before any byte is read — a swap of the final component in either
// direction can only refuse. FIFOs and other non-regular files are refused
// before the (potentially blocking) open.

#[cfg(unix)]
fn open_confined_file_unix(root: &Path, raw: &str) -> Result<(std::fs::File, u64)> {
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::io::{AsRawFd as _, FromRawFd as _};

    // The workspace root is the trusted anchor — an acquired, governed
    // directory whose canonical path the caller resolved once. The root
    // itself is pinned by handle before any child resolution happens.
    let root_name = std::ffi::CString::new(root.as_os_str().as_bytes())
        .context("native inspection workspace root is not a valid path")?;
    // SAFETY: `root_name` is a valid NUL-terminated path.
    let root_fd = unsafe {
        libc::openat(
            libc::AT_FDCWD,
            root_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if root_fd < 0 {
        return Err(std::io::Error::last_os_error())
            .context("native inspection workspace root is not accessible");
    }
    // SAFETY: `root_fd` is a fresh descriptor; ownership transfers to `File`.
    let mut directory = unsafe { std::fs::File::from_raw_fd(root_fd) };
    let trailing_separator = raw.ends_with('/');
    let mut components = raw
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .peekable();
    while let Some(component) = components.next() {
        let component_cstr = std::ffi::CString::new(component.as_bytes())
            .context("native inspection path contains a NUL byte")?;
        if components.peek().is_some() {
            // Intermediate: must be a real directory inside the parent
            // handle; a symlink (even one pointing inside) fails closed —
            // directory symlinks are traversal boundaries this adapter
            // does not follow.
            // SAFETY: `component_cstr` is NUL-terminated; `directory` is a
            // live directory handle.
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    component_cstr.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::NotFound {
                    bail!("native inspection target does not exist: {raw:?}");
                }
                return Err(err).context(
                    "native inspection could not open a workspace directory confined to the root",
                );
            }
            // SAFETY: `fd` is fresh; ownership transfers. The previous
            // directory handle is dropped (closed) — the walk has moved on.
            directory = unsafe { std::fs::File::from_raw_fd(fd) };
            continue;
        }
        // Final component: refuse a trailing separator exactly as a
        // directory would be refused below.
        if trailing_separator {
            bail!("native inspection target is not a regular file: {raw:?}");
        }
        // Capture the identity of the confined path's final component
        // WITHOUT following a symlink there — `AT_SYMLINK_NOFOLLOW` stats
        // the component itself, so a final symlink is refused below
        // instead of being resolved (a resolved target outside the
        // workspace must never be inspected, not even when the captured
        // identity happens to match the followed target).
        let mut expected_stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: `component_cstr` is NUL-terminated; `expected_stat` is
        // initialized by the call on success.
        let rc = unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                component_cstr.as_ptr(),
                expected_stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::NotFound {
                bail!("native inspection target does not exist: {raw:?}");
            }
            return Err(err).context("native inspection could not stat the confined target");
        }
        // SAFETY: `fstatat` succeeded and fully initialized the stat.
        let expected_stat = unsafe { expected_stat.assume_init() };
        // Regular file before the (potentially blocking) open: FIFO and
        // device opens would otherwise hang waiting for a writer.
        if (expected_stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
            bail!("native inspection target is not a regular file: {raw:?}");
        }
        // Open the file relative to the retained parent handle WITHOUT
        // following symlinks: between the capture above and this open, a
        // swap that placed a symlink at the final component fails here
        // (ELOOP) instead of retargeting the open onto the link target.
        // SAFETY: `component_cstr` is NUL-terminated; `directory` is live.
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                component_cstr.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::NotFound {
                bail!("native inspection target does not exist: {raw:?}");
            }
            return Err(err).context(
                "native inspection could not open the target file confined to the workspace",
            );
        }
        // SAFETY: `fd` is fresh; ownership transfers to `File`.
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        // Validate the OPENED handle: its identity must equal the captured
        // workspace file. A swap of the final component anywhere between
        // capture and open lands on a different inode and is refused here.
        let metadata = file
            .metadata()
            .context("native inspection could not stat the opened file")?;
        let expected = OpenFileId {
            primary: u64::from(expected_stat.st_dev),
            secondary: u64::from(expected_stat.st_ino),
        };
        if open_file_id(&file)? != expected {
            bail!(
                "native inspection opened handle does not match the verified workspace file: {raw:?}"
            );
        }
        if !metadata.is_file() {
            bail!("native inspection target is not a regular file: {raw:?}");
        }
        return Ok((file, metadata.len()));
    }
    // No components: the path named the workspace root itself, which is a
    // directory, never an inspectable file.
    bail!("native inspection target is not a regular file: {raw:?}")
}

/// Hash bytes from the exact retained handle — never from the pathname.
/// The size budget was enforced on the handle size before this call and is
/// re-enforced against the bytes actually read (a file growing mid-read
/// cannot smuggle unbounded input), and the handle identity is verified
/// unchanged before and after reading. Only the digest leaves this
/// function — content never does.
fn digest_open_file(
    file: &mut std::fs::File,
    expected_size: u64,
    max_file_bytes: u64,
) -> Result<String> {
    use sha2::Digest as _;
    let opened_id =
        open_file_id(file).context("native inspection could not verify the opened file")?;
    let mut hasher = sha2::Sha256::new();
    let mut total: u64 = 0;
    let mut buf = [0u8; READ_CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .context("native inspection read failed")?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        if total > max_file_bytes {
            bail!("native inspection file exceeds size budget ({max_file_bytes} bytes)");
        }
        hasher.update(&buf[..n]);
    }
    // The handle must still be the file we snapshotted before reading.
    if open_file_id(file).context("native inspection could not verify the opened file")?
        != opened_id
    {
        bail!("native inspection opened handle changed during read; refusing to report");
    }
    if total != expected_size {
        bail!("native inspection file changed during read; refusing to report");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn run_native_inspect_guarded(
    config: &NativeInspectConfig,
    workspace_root: &Path,
    args: &serde_json::Value,
) -> Result<String> {
    let decoded = decode_args(args, config.max_args_bytes)?;
    validate_relative_path(&decoded.path, config.max_path_chars)?;
    // Anchor: the workspace root is canonicalized once. Every confined open
    // resolves beneath it through a retained handle, never by reopening a
    // pathname.
    let root = std::fs::canonicalize(workspace_root).with_context(|| {
        format!(
            "workspace root {} is not accessible",
            workspace_root.display()
        )
    })?;
    let (mut file, handle_size) = open_confined_file(&root, &decoded.path)?;
    // The size budget is enforced on the size the opened handle reports,
    // before any byte is read.
    if handle_size > config.max_file_bytes {
        bail!(
            "native inspection file exceeds size budget ({} > {} bytes)",
            handle_size,
            config.max_file_bytes
        );
    }
    let digest = digest_open_file(&mut file, handle_size, config.max_file_bytes)?;
    // Structure-aware redaction (mirroring slice E): sensitive STRING VALUES
    // are redacted before JSON serialization, so quote/backslash-bearing
    // secrets and credential-shaped filenames can never damage JSON
    // grammar. Only the value changes — the verified digest and byte
    // count are preserved exactly, and file bytes never enter this
    // document.
    let mut secrets = config.known_secrets.clone();
    secrets.extend(decoded.known_secrets);
    let redactor = Redactor::new().with_known_secrets(&secrets);
    let redacted_path = redactor.redact(&decoded.path);
    let result = serde_json::json!({
        "path": redacted_path,
        "sizeBytes": handle_size,
        "sha256": digest,
        "contentReturned": false,
    });
    let text = serde_json::to_string(&result).context("inspection result serializes")?;
    if text.len() > config.max_output_bytes {
        bail!(
            "native inspection result exceeds output budget ({} > {} bytes)",
            text.len(),
            config.max_output_bytes
        );
    }
    // Defense in depth: verify — never rewrite — that no known secret or
    // credential-shaped residue survives in the persisted document. If one
    // does (e.g. a secret substring inside the digest hex), the whole
    // inspection refuses instead of persisting a secret or corrupting the
    // result's verified facts.
    if redactor.redact(&text) != text {
        bail!(
            "native inspection result still contains a known secret after redaction; refusing to persist"
        );
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::{OpenFileId, open_file_id};

    /// The handle identity that binds hashing to the verified file must
    /// be stable across handles to the same file and distinct across
    /// different files — otherwise the opened-handle checks could not
    /// distinguish "same authorized file" from a substituted one.
    #[test]
    fn open_file_identity_distinguishes_opened_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let first = std::fs::File::open(&a).unwrap();
        let second = std::fs::File::open(&a).unwrap();
        let other = std::fs::File::open(&b).unwrap();
        let first_id = open_file_id(&first).unwrap();
        assert_eq!(first_id, open_file_id(&second).unwrap());
        assert_ne!(first_id, open_file_id(&other).unwrap());
        // Sanity: identity fields carry real data (volume/device + index/inode).
        assert!(first_id.primary != 0 || first_id.secondary != 0);
        let _ = OpenFileId {
            primary: first_id.primary,
            secondary: first_id.secondary,
        };
    }
}
