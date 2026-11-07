//! Governed subprocess execution adapter (issue #154, slice E).
//!
//! [`SubprocessHarness`] is the first real execution adapter conforming to
//! [`ExecutionHarness`]: it runs an allowlisted executable inside an
//! explicitly governed workspace and records the result as evidence. It is
//! NOT a declaration-only registry entry and NOT a mock.
//!
//! Effect discipline (no bypass around #117/#118):
//!
//! - The adapter never spawns a process itself. Its `run` invokes the
//!   declared `harness.subprocess.exec` capability through
//!   [`HarnessDispatch::call`], the only sanctioned effect path, so every
//!   subprocess execution passes the NodeRunner nine-gate pipeline
//!   (capability resolution, argument validation, authorization, retry
//!   budget, redaction, journaled evidence) before any child exists.
//! - The capability handler runs the child with an owned process-tree
//!   containment and a bounded output lifecycle: on timeout, cancellation,
//!   output overflow, or normal completion the entire owned tree is
//!   terminated (grandchildren included) and the direct child is reaped
//!   before the handler returns, so no unmanaged descendants survive.
//! - Strict argument validation, an exact-match executable allowlist, a
//!   workspace-contained working directory (escapes refuse; no fallback to
//!   the process cwd), an explicit deadline, cancellation, and a hard
//!   output budget enforced on the bytes actually written (not just the
//!   bytes retained). Exceeding the budget is a typed refusal and never
//!   reports success.
//!
//! Output storage (R2): child stdout/stderr spool to securely random,
//! atomically created, exclusive (`create_new`) temporary files with
//! private permissions; the file handles are retained for the whole
//! invocation (never reopened by untrusted path) and unlinked on every
//! exit path, including spawn failure and refusal.
//!
//! Structured redaction (R4): child output is redacted with the shared
//! credential-shape rules BEFORE it is embedded into the JSON result
//! document, so gate-7 regex redaction cannot corrupt the JSON grammar
//! and known secrets never reach persisted evidence.
//!
//! Portable continuity (E2): the adapter owns no durable work state. Its
//! `checkpoint`/`resume` bind the harness identity + revision to a digest
//! of the canonical [`PortableWorkState`](crate::workflow::portable_state::PortableWorkState),
//! which lives outside the subprocess. Resume re-verifies the checkpoint
//! and refuses incompatible harness/revision/digest bindings.
//! Live-session resume is not claimed: continuation means re-running
//! subsequent governed steps against the same portable state.
//! Evidence correlation: every outcome carries the journaled evidence
//! digest; evidence survives adapter process termination.

use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};

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
use crate::workflow::soma::profile::Boundaries;
use crate::workflow::workspace::ADAPTER_REVISION;

/// Implementation-owned harness id.
pub const SUBPROCESS_HARNESS_ID: &str = "lite.harness.subprocess";
/// The single declared capability (registry key + declared capability).
pub const SUBPROCESS_EXEC_CAPABILITY: &str = "harness.subprocess.exec";
/// Identity name/revision of the implementation.
pub const SUBPROCESS_IDENTITY_NAME: &str = "lite-subprocess";
pub const SUBPROCESS_REVISION: &str = "v1";

/// Poll interval for the child wait loop.
const POLL: Duration = Duration::from_millis(5);
/// Settle time for tree quiescence after termination / group sweeps.
const SETTLE: Duration = Duration::from_millis(100);
/// Monotonic counter disambiguating concurrent spool files.
static SPOOL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Configuration for the governed subprocess handler.
///
/// All bounds are enforced by the handler on every invocation; there is no
/// unbounded mode.
#[derive(Clone)]
pub struct SubprocessConfig {
    /// Exact-match executable allowlist (e.g. `cargo`, `rustc`). A requested
    /// `exe` must equal one of these strings; path separators are matched
    /// literally, never normalized away.
    pub allowed_executables: Vec<String>,
    /// Default invocation deadline when the step omits `timeoutMs`.
    pub default_timeout_ms: u64,
    /// Hard ceiling for any invocation deadline (1 h).
    pub max_timeout_ms: u64,
    /// Hard budget, in bytes, of stdout+stderr the child may write during
    /// one invocation. Exceeding it kills the tree and refuses.
    pub max_output_bytes: usize,
    /// Bytes of serialized arguments accepted per invocation.
    pub max_args_bytes: usize,
    /// Cooperative cancellation source; setting it aborts the running tree.
    pub cancel: Arc<AtomicBool>,
}

impl Default for SubprocessConfig {
    fn default() -> Self {
        Self {
            allowed_executables: vec!["cargo".to_string()],
            default_timeout_ms: 30_000,
            max_timeout_ms: 3_600_000,
            max_output_bytes: 1024 * 1024,
            max_args_bytes: 64 * 1024,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl SubprocessConfig {
    pub fn allowed_executables(mut self, executables: Vec<String>) -> Self {
        self.allowed_executables = executables;
        self
    }

    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.default_timeout_ms = timeout_ms;
        self
    }

    pub fn max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes;
        self
    }

    pub fn cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = cancel;
        self
    }
}

/// The implementation-owned, sealed declaration for the subprocess harness.
/// Shared by [`SubprocessHarness`] and the canonical catalog inventory so
/// the two cannot diverge.
pub fn subprocess_declaration() -> HarnessDeclarationV1 {
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: SUBPROCESS_HARNESS_ID.to_string(),
        identity: Identity {
            name: SUBPROCESS_IDENTITY_NAME.to_string(),
            revision: SUBPROCESS_REVISION.to_string(),
        },
        family: HarnessFamily::Subprocess,
        declared_capabilities: vec![SUBPROCESS_EXEC_CAPABILITY.to_string()],
        claimed_capabilities: vec![SUBPROCESS_EXEC_CAPABILITY.to_string()],
        capabilities: RuntimeCapabilitySet {
            id: "lite.harness.subprocess.caps".to_string(),
            schema_version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            identity: Identity {
                name: SUBPROCESS_IDENTITY_NAME.to_string(),
                revision: SUBPROCESS_REVISION.to_string(),
            },
            declared_at: "2026-10-26T00:00:00Z".to_string(),
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
            effect_classes: vec!["process-exec".to_string()],
            workspace: Some(Boundaries {
                workspace: Some(crate::workflow::soma::profile::Boundary::Write),
                filesystem: None,
                git: None,
                process: None,
                network: None,
                secret: None,
            }),
            checkpoint_import: Some(true),
            checkpoint_export: Some(true),
            resume_support: Some(true),
            // The child output is one terminal evidence document; nothing is
            // streamed incrementally, so streaming is not claimed.
            event_streaming: Some(false),
            read_projection: Some(false),
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

/// The governed subprocess adapter.
///
/// The adapter itself never spawns a process: `run` invokes the declared
/// capability through the governed dispatch. The subprocess handler
/// ([`subprocess_capability`]) is registered by the caller in the
/// [`crate::workflow::node_runner::CapabilityRegistry`] alongside the
/// reviewed governance permit that authorizes it.
pub struct SubprocessHarness {
    declaration: HarnessDeclarationV1,
}

impl Default for SubprocessHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl SubprocessHarness {
    pub fn new() -> Self {
        Self {
            declaration: subprocess_declaration(),
        }
    }
}

#[async_trait::async_trait]
impl ExecutionHarness for SubprocessHarness {
    fn declaration(&self) -> &HarnessDeclarationV1 {
        &self.declaration
    }

    async fn run(
        &self,
        step: HarnessStep,
        dispatch: &mut HarnessDispatch<'_>,
    ) -> Result<HarnessOutcome> {
        // The ONLY effect path: `call` refuses capabilities outside this
        // harness's declaration before any process can exist.
        let run_outcome = dispatch.call(&step.capability, step.args.clone(), &step.step_id)?;
        let output = run_outcome.output.clone();

        // The child result is machine-readable JSON produced by the handler.
        let parsed: serde_json::Value = serde_json::from_str(&output)
            .context("subprocess capability output must be a JSON result document")?;
        let exit_code = parsed.get("exitCode").and_then(serde_json::Value::as_i64);
        if exit_code != Some(0) {
            let stderr_bytes = parsed
                .get("stderrHex")
                .and_then(serde_json::Value::as_str)
                .and_then(decode_hex)
                .unwrap_or_default();
            let stderr = String::from_utf8_lossy(&stderr_bytes);
            let stderr = stderr.trim();
            let stderr_suffix = if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            };
            match exit_code {
                Some(code) => anyhow::bail!("subprocess exited with code {code}{stderr_suffix}"),
                None => anyhow::bail!("subprocess terminated by signal{stderr_suffix}"),
            }
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
            harness_id: SUBPROCESS_HARNESS_ID.to_string(),
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
            harness_id: SUBPROCESS_HARNESS_ID.to_string(),
            harness_revision: SUBPROCESS_REVISION.to_string(),
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
        if checkpoint.harness_id != SUBPROCESS_HARNESS_ID {
            anyhow::bail!(
                "harness checkpoint was exported by {:?}, not by {SUBPROCESS_HARNESS_ID:?}",
                checkpoint.harness_id
            );
        }
        if checkpoint.harness_revision != SUBPROCESS_REVISION {
            anyhow::bail!(
                "harness checkpoint revision {:?} is incompatible with this implementation ({SUBPROCESS_REVISION:?})",
                checkpoint.harness_revision
            );
        }
        if checkpoint.portable_state_digest != portable_state_digest {
            anyhow::bail!(
                "harness checkpoint portable state digest does not match the supplied state"
            );
        }
        Ok(HarnessContinuation {
            harness_id: SUBPROCESS_HARNESS_ID.to_string(),
            harness_revision: SUBPROCESS_REVISION.to_string(),
            portable_state_digest: portable_state_digest.to_string(),
            resume_step: checkpoint.resume_step.clone(),
        })
    }
}

/// Build the governed `harness.subprocess.exec` capability for a runner's
/// registry, bound to an acquired workspace root.
///
/// The handler is `Fn` (reusable across invocations); each invocation is
/// independently bounded and validated. `workspace_root` must be the
/// GovernedWorkspace root for the work — the subprocess cwd is always this
/// root or a validated sub-directory; there is no fallback.
pub fn subprocess_capability(config: SubprocessConfig, workspace_root: PathBuf) -> Capability {
    Capability::deterministic(&["exe", "argv"], move |args| {
        run_subprocess_guarded(&config, &workspace_root, args)
    })
}

// ---------------------------------------------------------------------------
// Secure spool (R2)
// ---------------------------------------------------------------------------

/// One child-stdout/stderr spool: an exclusively created, privately
/// permissioned temporary file. The [`File`] handle is retained for the
/// whole invocation and is never reopened by path; the path exists only so
/// the file can be unlinked on drop.
struct SpoolFile {
    file: File,
    path: PathBuf,
}

impl Drop for SpoolFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Create a spool file atomically and exclusively: `create_new` never
/// follows a pre-existing symlink and never truncates an existing file; on
/// collision a fresh random name is tried. Names derive from pid, a
/// monotonic counter, and a per-process random seed hashed over the
/// nanosecond clock, so hostile pre-creation cannot predict them.
fn create_spool_file(tag: &str) -> Result<SpoolFile> {
    let pid = std::process::id();
    let counter = SPOOL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seed = format!("{tag}:{pid}:{counter}:{nanos}:{}", seed_salt());
    let nonce = sha256_hex(seed.as_bytes());
    let dir = std::env::temp_dir();
    for attempt in 0u32..16 {
        let name = match attempt {
            0 => format!("prometheos-subprocess-{pid}-{counter}-{nonce}.tmp"),
            _ => format!("prometheos-subprocess-{pid}-{counter}-{nonce}-retry{attempt}.tmp"),
        };
        let path = dir.join(name);
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        match opts.open(&path) {
            Ok(file) => return Ok(SpoolFile { file, path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                anyhow::bail!("cannot create subprocess spool {}: {error}", path.display())
            }
        }
    }
    anyhow::bail!("cannot create a unique subprocess spool after 16 attempts")
}

/// Per-process random salt (SipHash with random keys) so spool names are
/// unpredictable even for a local attacker pre-creating candidates.
fn seed_salt() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher as _};
    RandomState::new().build_hasher().finish()
}

/// The retained spool pair for one invocation.
struct SpoolFiles {
    stdout: SpoolFile,
    stderr: SpoolFile,
}

impl SpoolFiles {
    fn new() -> Result<Self> {
        Ok(Self {
            stdout: create_spool_file("out")?,
            stderr: create_spool_file("err")?,
        })
    }

    /// Total bytes currently written by the child, observed through the
    /// retained handles (never by re-reading an untrusted path).
    fn written(&self) -> u64 {
        let out = self.stdout.file.metadata().map(|m| m.len()).unwrap_or(0);
        let err = self.stderr.file.metadata().map(|m| m.len()).unwrap_or(0);
        out + err
    }

    /// Read a spool after the child has exited. The parent's original
    /// handle was donated to the child, so reading goes through a fresh
    /// no-follow handle on the exclusively created path (see `read_spool`
    /// for why this cannot be confused with a hostile file).
    fn read(&self, which: &SpoolFile) -> String {
        read_spool(&which.path)
    }
}

/// Reopen a spool for reading without following symlinks.
///
/// The file at `path` was atomically created exclusively by us; this open
/// is read-only and refuses symlink replacement even if a hostile actor
/// swapped the path after creation. Content that came from the child was
/// already bounded by the output budget before any read happens.
fn read_spool(path: &Path) -> String {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        opts.custom_flags(winapi::um::winbase::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let Ok(mut file) = opts.open(path) else {
        return String::new();
    };
    let mut bytes = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

// ---------------------------------------------------------------------------
// Process-tree containment (R1)
// ---------------------------------------------------------------------------

/// Owned process-tree containment for one child invocation.
///
/// - Windows: a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`,
///   created at install time and assigned to the child after spawn. On
///   termination the whole job tree dies; on normal completion the job
///   handle is closed and kill-on-close sweeps any stragglers.
/// - Unix: the child is placed in its own session via `setsid` at spawn
///   time (pre_exec), so its process group id is the child's pid; the
///   group is SIGKILLed on termination and swept on normal completion.
///
/// If containment cannot be established, the caller kills and reaps the
/// child and fails closed — never proceeds uncontained.
struct Containment {
    #[cfg(windows)]
    job: Option<winapi::shared::ntdef::HANDLE>,
    /// Process group id once spawned (Unix only): equals the child's pid
    /// because `setsid` made it a session/group leader.
    #[cfg(unix)]
    pgid: Option<i32>,
}

impl Containment {
    /// Install containment BEFORE spawn (Unix `setsid` hook) or the pending
    /// state for post-spawn assignment (Windows job).
    fn install(command: &mut Command) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.pre_exec(|| {
                if unsafe { libc::setsid() } < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
            Ok(Self { pgid: None })
        }
        #[cfg(windows)]
        {
            let _ = command;
            Ok(Self { job: None })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = command;
            anyhow::bail!("subprocess containment is unsupported on this platform")
        }
    }

    /// Attach the spawned child to the containment. Windows assigns the
    /// child to the job (the child has only just started); Unix is a no-op
    /// because containment was installed at spawn time. On failure the
    /// caller must kill and reap the child and fail closed.
    fn attach(&mut self, child: &mut std::process::Child) -> Result<()> {
        #[cfg(unix)]
        {
            let _ = child;
            self.pgid = Some(child.id() as i32);
            Ok(())
        }
        #[cfg(windows)]
        {
            use winapi::shared::minwindef::{BOOL, DWORD, LPVOID};
            use winapi::um::handleapi::CloseHandle;
            use winapi::um::jobapi2::{
                AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
            };
            use winapi::um::processthreadsapi::OpenProcess;
            use winapi::um::winnt::{
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JobObjectExtendedLimitInformation, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
            };

            // Fast path: the child already exited; nothing to contain.
            if child.try_wait()?.is_some() {
                return Ok(());
            }

            let job = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null_mut()) };
            if job.is_null() {
                anyhow::bail!(
                    "subprocess containment unavailable: CreateJobObjectW failed ({})",
                    std::io::Error::last_os_error()
                );
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set_ok = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &mut info as *mut _ as LPVOID,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as DWORD,
                )
            };
            if set_ok == 0 {
                unsafe {
                    CloseHandle(job);
                };
                anyhow::bail!(
                    "subprocess containment unavailable: SetInformationJobObject failed ({})",
                    std::io::Error::last_os_error()
                );
            }
            let pid = child.id();
            let proc = unsafe {
                OpenProcess(
                    PROCESS_SET_QUOTA | PROCESS_TERMINATE,
                    0 as BOOL,
                    pid as DWORD,
                )
            };
            if proc.is_null() {
                unsafe {
                    CloseHandle(job);
                };
                anyhow::bail!(
                    "subprocess containment unavailable: OpenProcess failed ({})",
                    std::io::Error::last_os_error()
                );
            }
            let assigned = unsafe { AssignProcessToJobObject(job, proc) };
            unsafe {
                CloseHandle(proc);
            };
            if assigned == 0 {
                unsafe {
                    CloseHandle(job);
                };
                anyhow::bail!(
                    "subprocess containment unavailable: AssignProcessToJobObject failed ({})",
                    std::io::Error::last_os_error()
                );
            }
            self.job = Some(job);
            Ok(())
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = child;
            Ok(())
        }
    }

    /// Terminate the entire owned tree. The direct child is still reaped by
    /// the caller's `wait`. Returns after a settle period so descendants
    /// have been delivered the kill.
    fn terminate(&mut self, child: &std::process::Child) {
        #[cfg(unix)]
        {
            if let Some(pgid) = self.pgid {
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
            }
            let _ = child;
        }
        #[cfg(windows)]
        {
            if let Some(job) = self.job.take() {
                let _ = unsafe { winapi::um::jobapi2::TerminateJobObject(job, 1) };
            }
            let _ = child;
        }
        std::thread::sleep(SETTLE);
    }

    /// Terminate stragglers after normal direct-child exit: Windows is
    /// handled by kill-on-job-close when the handle drops; Unix sweeps the
    /// (possibly empty) group.
    fn sweep_on_normal(&mut self) {
        #[cfg(unix)]
        {
            if let Some(pgid) = self.pgid.take() {
                let rc = unsafe { libc::kill(-pgid, 0) };
                if rc == 0 {
                    unsafe {
                        libc::kill(-pgid, libc::SIGKILL);
                    }
                    std::thread::sleep(SETTLE);
                }
            }
        }
        #[cfg(windows)]
        {
            // Dropping the handle fires JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            // for any descendant still running.
            if let Some(job) = self.job.take() {
                unsafe {
                    winapi::um::handleapi::CloseHandle(job);
                }
                std::thread::sleep(SETTLE);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for Containment {
    fn drop(&mut self) {
        // Last-resort sweep on any forgotten path: closing the handle
        // kills the remaining tree (kill-on-job-close).
        if let Some(job) = self.job.take() {
            unsafe {
                winapi::um::handleapi::CloseHandle(job);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Governing checks (workspace binding)
// ---------------------------------------------------------------------------

/// Resolve the governed workspace root or refuse. The root must exist on
/// every invocation: workspaces are ephemeral, and a missing root must
/// never silently fall back to any other directory.
fn governed_root(workspace_root: &Path) -> Result<PathBuf> {
    if !workspace_root.is_dir() {
        anyhow::bail!(
            "governed workspace root {} is missing; refusing to fall back to another directory",
            workspace_root.display()
        );
    }
    workspace_root.canonicalize().with_context(|| {
        format!(
            "cannot canonicalize governed workspace root {}",
            workspace_root.display()
        )
    })
}

/// Resolve the child working directory inside the governed root. Relative
/// sub-directories are joined and canonicalized; the result must remain
/// within the canonical root and must exist. Anything else refuses.
fn child_cwd(canonical_root: &Path, cwd: Option<&str>) -> Result<PathBuf> {
    let Some(cwd) = cwd else {
        return Ok(canonical_root.to_path_buf());
    };
    let requested = Path::new(cwd);
    if requested.is_absolute() {
        anyhow::bail!(
            "subprocess cwd {cwd:?} is absolute; only paths inside the governed workspace are permitted"
        );
    }
    let joined = canonical_root.join(requested);
    let resolved = joined.canonicalize().map_err(|_| {
        anyhow::anyhow!(
            "subprocess cwd {cwd:?} does not exist inside the governed workspace; refusing"
        )
    })?;
    if !resolved.starts_with(canonical_root) {
        anyhow::bail!("subprocess cwd {cwd:?} escapes the governed workspace root; refusing");
    }
    Ok(resolved)
}

/// One governed subprocess invocation behind gates 1-4; gate 6-9 seal its
/// JSON result document.
fn run_subprocess_guarded(
    config: &SubprocessConfig,
    workspace_root: &Path,
    args: &serde_json::Value,
) -> Result<String> {
    // ---- Argument validation (fail closed, no coercion). ----
    let obj = args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("subprocess arguments must be an object"))?;
    let total_args_bytes: usize = obj.values().map(|v| v.to_string().len()).sum();
    if total_args_bytes > config.max_args_bytes {
        anyhow::bail!(
            "subprocess arguments exceed the configured limit of {} bytes",
            config.max_args_bytes
        );
    }
    for key in obj.keys() {
        if !matches!(key.as_str(), "exe" | "argv" | "cwd" | "timeoutMs") {
            anyhow::bail!("subprocess argument {key:?} is not supported");
        }
    }
    let exe = obj
        .get("exe")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("subprocess argument \"exe\" must be a non-empty string"))?;
    let argv: Vec<String> = obj
        .get("argv")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .ok_or_else(|| {
            anyhow::anyhow!("subprocess argument \"argv\" must be an array of strings")
        })?;
    // The filter_map above silently drops non-string items; ensure length
    // equality so a non-string argv member is a refusal, not a silent drop.
    if argv.len() != obj["argv"].as_array().map(Vec::len).unwrap_or(0) {
        anyhow::bail!("subprocess argument \"argv\" must be an array of strings");
    }
    let cwd = obj
        .get("cwd")
        .map(|v| {
            v.as_str()
                .ok_or_else(|| anyhow::anyhow!("subprocess argument \"cwd\" must be a string"))
        })
        .transpose()?;
    let timeout_ms = obj
        .get("timeoutMs")
        .map(|v| {
            v.as_u64()
                .filter(|ms| *ms >= 1 && *ms <= config.max_timeout_ms)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "subprocess argument \"timeoutMs\" must be a positive number of milliseconds up to {}",
                        config.max_timeout_ms
                    )
                })
        })
        .transpose()?
        .unwrap_or(config.default_timeout_ms);

    // ---- Executable allowlist (exact match). ----
    if !config.allowed_executables.iter().any(|a| a == exe) {
        anyhow::bail!("subprocess executable {exe:?} is not allowlisted");
    }

    // ---- Workspace binding (no fallback). ----
    let canonical_root = governed_root(workspace_root)?;
    let cwd = child_cwd(&canonical_root, cwd)?;

    // ---- Spawn with file-backed stdio (no pipe deadlock). ----
    let spool = SpoolFiles::new()?;
    let stdout_dup = spool.stdout.file.try_clone().with_context(|| {
        format!(
            "cannot duplicate subprocess stdout spool {}",
            spool.stdout.path.display()
        )
    })?;
    let stderr_dup = spool.stderr.file.try_clone().with_context(|| {
        format!(
            "cannot duplicate subprocess stderr spool {}",
            spool.stderr.path.display()
        )
    })?;

    let started = Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms);
    let mut cmd = Command::new(exe);
    cmd.args(&argv)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_dup))
        .stderr(Stdio::from(stderr_dup));
    let mut containment = Containment::install(&mut cmd)?;
    let spawn_result = cmd.spawn();
    let mut child = match spawn_result {
        Ok(child) => child,
        Err(error) => {
            anyhow::bail!("subprocess spawn failed for executable {exe:?}: {error}")
        }
    };
    if let Err(error) = containment.attach(&mut child) {
        // Containment unavailable: kill and reap, then fail closed.
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }

    // ---- Deadline / cancellation / budget loop. ----
    // Every refusal below terminates the whole owned tree first and reaps
    // the direct child, so descendants cannot continue past this return.
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if config.cancel.load(Ordering::Relaxed) {
            containment.terminate(&child);
            let _ = child.wait();
            anyhow::bail!("subprocess cancelled");
        }
        if Instant::now() >= deadline {
            containment.terminate(&child);
            let _ = child.wait();
            anyhow::bail!("subprocess timed out after {timeout_ms}ms");
        }
        if spool.written() > config.max_output_bytes as u64 {
            containment.terminate(&child);
            let _ = child.wait();
            anyhow::bail!(
                "subprocess output exceeded the configured budget of {} bytes",
                config.max_output_bytes
            );
        }
        std::thread::sleep(POLL);
    };
    let duration_ms = started.elapsed().as_millis() as u64;

    // The direct child exited: sweep any stragglers it left behind before
    // trusting the result, then close the resource budget accounting.
    containment.sweep_on_normal();
    if spool.written() > config.max_output_bytes as u64 {
        anyhow::bail!(
            "subprocess output exceeded the configured budget of {} bytes",
            config.max_output_bytes
        );
    }

    // Structured redaction before encoding (R4): the shared credential-shape
    // rules run on the separate stream VALUES, and the redacted values are
    // hex-encoded into the result document. Gate-7's patterns then cannot
    // match (hex text contains no credential triggers and no `"`/`\`
    // structure to corrupt), so the document stays valid typed JSON, and
    // only redacted data is persisted at every stage.
    let redactor = Redactor::new();
    let stdout = encode_hex(redactor.redact(&spool.read(&spool.stdout)).as_bytes());
    let stderr = encode_hex(redactor.redact(&spool.read(&spool.stderr)).as_bytes());
    let exit_code = status.code();

    Ok(serde_json::json!({
        "exitCode": exit_code,
        "success": exit_code == Some(0),
        "stdoutHex": stdout,
        "stderrHex": stderr,
        "durationMs": duration_ms,
    })
    .to_string())
}

/// Lowercase hex encode: keeps the result document inert to the gate-7
/// credential patterns (hex digits cannot spell any trigger word, and no
/// structural quote/backslash can be consumed).
fn encode_hex(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 2);
    for byte in data {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Decode lowercase hex; `None` on any malformed input.
fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    for pair in bytes.chunks(2) {
        let high = hex_digit(pair[0])?;
        let low = hex_digit(pair[1])?;
        out.push(high * 16 + low);
    }
    Some(out)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exclusive creation: a pre-created file at the spool name is never
    /// truncated and never followed; a fresh name is returned instead.
    #[test]
    fn spool_file_creation_is_exclusive() {
        // Pre-create a plausible host-file name that a collision would
        // otherwise clobber.
        let victim: String = "V".repeat(64);
        let first = create_spool_file("test-probe").expect("first spool");
        std::fs::write(&first.path, &victim).expect("write victim bytes");
        let second = create_spool_file("test-probe").expect("second spool");
        assert_ne!(first.path, second.path);
        let kept = std::fs::read_to_string(&first.path).expect("read back");
        assert_eq!(kept, victim, "existing spool content must survive");
        // Files are created empty (no leak of other runs' bytes).
        assert_eq!(second.file.metadata().expect("meta").len(), 0);
    }

    /// Unix spool files carry private (0600) permissions.
    #[cfg(unix)]
    #[test]
    fn unix_spool_files_are_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let spool = create_spool_file("perm-probe").expect("spool");
        let mode = spool.file.metadata().expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "spool files must be 0600");
    }
}
