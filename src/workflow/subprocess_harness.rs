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
//!   containment and a memory-bounded capture: on timeout, cancellation,
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
//! Output capture (R3.3): child stdout/stderr flow through OS pipes drained
//! concurrently into bounded in-memory buffers. No file ever touches disk,
//! so disk usage is zero by construction, pathname replacement is
//! structurally impossible, and fast writers cannot consume unbounded
//! memory: retained bytes saturate at the budget and their total is
//! counted continuously.
//!
//! Structured redaction (R3.2): the shared redaction policy — known
//! secrets first (exact replacement), then credential-shaped patterns —
//! runs on the captured stream values. The redacted values are
//! hex-encoded into the result document, so gate-7's patterns cannot
//! match the text and cannot corrupt the JSON grammar on the way to
//! durable evidence, while known secrets (including opaque ones that
//! match no pattern) never reach persisted output.
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

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt as _;

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
/// Buffer retain cap per stream equals the output budget; streams are
/// counted independently and the sum enforces the total.
const DRAIN_READ_CHUNK: usize = 8192;

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
    /// Known secret values, redacted verbatim (before any encoding) from
    /// captured output. The caller supplies these from the same trusted
    /// source as the governed request's `known_secrets`.
    pub known_secrets: Vec<String>,
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
            known_secrets: Vec::new(),
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

    pub fn known_secrets(mut self, known_secrets: Vec<String>) -> Self {
        self.known_secrets = known_secrets;
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
        // harness's declaration before any process can exist. The governed
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
// Memory-bounded pipe capture (R3.3)
// ---------------------------------------------------------------------------

/// Drain state behind one pipe: bounded retention, total byte count.
#[derive(Clone)]
struct DrainState {
    retained: Vec<u8>,
    total: u64,
    done: bool,
}

/// A bounded in-memory drain of one child pipe.
///
/// `retain_cap` bounds retained bytes; `total` counts every byte read so
/// budget checks see the true write volume. The reader blocks on the OS
/// pipe (no file ever exists), so disk usage is zero by construction and
/// fast writers cannot allocate unbounded memory: retention saturates at
/// `retain_cap` and excess is dropped after counting.
fn spawn_drain<R: Read + Send + 'static>(mut reader: R, retain_cap: usize) -> DrainHandle {
    let state = Arc::new(Mutex::new(DrainState {
        retained: Vec::new(),
        total: 0,
        done: false,
    }));
    let thread_state = Arc::clone(&state);
    let handle = std::thread::spawn(move || {
        let mut chunk = vec![0u8; DRAIN_READ_CHUNK];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let mut state = thread_state.lock().expect("drain state");
                    state.total = state.total.saturating_add(n as u64);
                    let room = retain_cap.saturating_sub(state.retained.len());
                    if room > 0 {
                        state.retained.extend_from_slice(&chunk[..room.min(n)]);
                    }
                }
                Err(_) => break,
            }
        }
        thread_state.lock().expect("drain state").done = true;
    });
    DrainHandle {
        state,
        handle,
        joined: false,
    }
}

struct DrainHandle {
    state: Arc<Mutex<DrainState>>,
    handle: std::thread::JoinHandle<()>,
    joined: bool,
}

impl DrainHandle {
    /// Snapshot (retained text, total bytes, drain done).
    fn snapshot(&self) -> (Vec<u8>, u64, bool) {
        let state = self.state.lock().expect("drain state");
        (state.retained.clone(), state.total, state.done)
    }

    fn total(&self) -> u64 {
        self.state.lock().expect("drain state").total
    }

    /// Wait for EOF (all writers died with the contained tree), then
    /// return the final drain state — retained bytes, total, done. The
    /// caller must verify `done` before sealing evidence; an unfinished
    /// drain is a failure, never a silent success.
    fn finish(mut self) -> DrainState {
        self.joined = true;
        let started = Instant::now();
        loop {
            if self.snapshot().2 || started.elapsed() > Duration::from_millis(2000) {
                break;
            }
            std::thread::sleep(POLL);
        }
        if self.snapshot().2 {
            let _ = self.handle.join();
        }
        self.state.lock().expect("drain state").clone()
    }
}

// ---------------------------------------------------------------------------
// Process-tree containment (R1)
// ---------------------------------------------------------------------------

/// Owned process-tree containment for one child invocation.
///
/// - Windows: a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`,
///   created before spawn and assigned to the SUSPENDED child process
///   (created with `CREATE_SUSPENDED`), which is then resumed — no
///   executable code runs before containment is installed.
/// - Unix: the child is launched in its own session via `setsid` at spawn
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
    /// Create the Windows job / arm the Unix group install hook. Called
    /// before any process is spawned.
    fn new() -> Self {
        Self {
            #[cfg(windows)]
            job: None,
            #[cfg(unix)]
            pgid: None,
        }
    }

    /// Windows: create and configure the job now (pre-spawn). Fails closed
    /// when the job cannot be established.
    #[cfg(windows)]
    fn create_job() -> Result<winapi::shared::ntdef::HANDLE> {
        use winapi::shared::minwindef::{DWORD, LPVOID};
        use winapi::um::handleapi::CloseHandle;
        use winapi::um::jobapi2::{CreateJobObjectW, SetInformationJobObject};
        use winapi::um::winnt::{
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation,
        };

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
        Ok(job)
    }

    /// Assign a suspended child to the job, then resume it. Only reached
    /// with a process that has not run any user code yet.
    #[cfg(windows)]
    fn arm(&mut self, child: &WinProcess) -> Result<()> {
        use winapi::um::jobapi2::AssignProcessToJobObject;
        let job = Self::create_job()
            .with_context(|| "containment job initialization failed".to_string())?;
        let assigned = unsafe { AssignProcessToJobObject(job, child.process) };
        if assigned == 0 {
            unsafe {
                winapi::um::handleapi::CloseHandle(job);
            }
            anyhow::bail!(
                "subprocess containment unavailable: AssignProcessToJobObject failed ({})",
                std::io::Error::last_os_error()
            );
        }
        self.job = Some(job);
        Ok(())
    }

    #[cfg(unix)]
    fn arm(&mut self, child: &std::process::Child) -> Result<()> {
        self.pgid = Some(child.id() as i32);
        Ok(())
    }

    /// Terminate the entire owned tree. The direct child is still reaped by
    /// the caller's wait. After this returns, a descendant cannot be
    /// expected to write anything further.
    fn terminate(&mut self) {
        #[cfg(unix)]
        {
            if let Some(pgid) = self.pgid {
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
            }
        }
        #[cfg(windows)]
        {
            if let Some(job) = self.job.take() {
                let _ = unsafe { winapi::um::jobapi2::TerminateJobObject(job, 1) };
            }
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
// Child process handle (platform spawn + wait)
// ---------------------------------------------------------------------------

/// A running governed child + its tree containment.
enum SpawnedChild {
    #[cfg(unix)]
    Unix {
        child: std::process::Child,
        containment: Containment,
    },
    #[cfg(windows)]
    Windows {
        process: WinProcess,
        containment: Containment,
    },
}

#[cfg(windows)]
#[derive(Clone)]
struct WinProcess {
    process: winapi::shared::ntdef::HANDLE,
}

impl SpawnedChild {
    /// Poll once: `None` = still running, `Some(exit code)` = exited.
    fn try_wait(&mut self) -> Option<i32> {
        #[cfg(unix)]
        {
            match self {
                SpawnedChild::Unix { child, .. } => child
                    .try_wait()
                    .ok()
                    .flatten()
                    .map(|status| status.code().unwrap_or(-1)),
                #[cfg(windows)]
                SpawnedChild::Windows { .. } => unreachable!(),
            }
        }
        #[cfg(windows)]
        {
            match self {
                SpawnedChild::Windows { process, .. } => process.try_wait(),
                #[cfg(unix)]
                SpawnedChild::Unix { .. } => unreachable!(),
            }
        }
    }

    /// Terminate the tree (containment) and reap the direct child.
    fn terminate_and_reap(&mut self) {
        match self {
            #[cfg(unix)]
            SpawnedChild::Unix { child, containment } => {
                containment.terminate();
                let _ = child.kill();
                let _ = child.wait();
            }
            #[cfg(windows)]
            SpawnedChild::Windows {
                process,
                containment,
            } => {
                containment.terminate();
                process.clone().finish();
            }
        }
    }

    /// Sweep descendants after normal direct-child exit.
    fn sweep_on_normal(&mut self) {
        match self {
            #[cfg(unix)]
            SpawnedChild::Unix { containment, .. } => containment.sweep_on_normal(),
            #[cfg(windows)]
            SpawnedChild::Windows {
                containment,
                process,
            } => {
                containment.sweep_on_normal();
                // The process has already exited: release its handle now
                // rather than leaking it per invocation.
                process.clone().finish();
            }
        }
    }
}

#[cfg(windows)]
impl WinProcess {
    /// Poll once: `None` = still running, `Some(exit code)` = exited.
    /// `GetExitCodeProcess` returns `STILL_ACTIVE` while the process runs.
    fn try_wait(&self) -> Option<i32> {
        use winapi::um::minwinbase::STILL_ACTIVE;
        use winapi::um::processthreadsapi::GetExitCodeProcess;

        let mut code: u32 = 0;
        let ok = unsafe { GetExitCodeProcess(self.process, &mut code) };
        if ok == 0 || code == STILL_ACTIVE {
            return None;
        }
        Some(code as i32)
    }

    /// Wait for the (possibly job-terminated) process to exit, then release
    /// the handle.
    fn finish(self) {
        loop {
            if self.try_wait().is_some() {
                break;
            }
            std::thread::sleep(POLL);
        }
        unsafe {
            winapi::um::handleapi::CloseHandle(self.process);
        }
    }
}

// ---------------------------------------------------------------------------
// Windows pipe creation for the drain threads (CreatePipe is in the
// `namedpipeapi` feature, which is not vendored in this crate; a narrow
// local extern covers exactly the one call needed).
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win_pipe {
    use super::PixReader;
    use winapi::shared::minwindef::{BOOL, DWORD};
    use winapi::shared::ntdef::HANDLE;
    use winapi::um::minwinbase::SECURITY_ATTRIBUTES;

    unsafe extern "system" {
        /// Mirrors `CreatePipe`'s call shape in the Win32 API.
        pub fn CreatePipe(
            hReadPipe: *mut HANDLE,
            hWritePipe: *mut HANDLE,
            lpSecurityAttributes: *mut SECURITY_ATTRIBUTES,
            dwSize: DWORD,
        ) -> BOOL;
    }

    /// Create a real OS pipe (read end, write end). The write end is marked
    /// inheritable so the child process can inherit it; the read end is
    /// explicitly made non-inheritable so the child's execution can never
    /// block EOF on the drain thread.
    pub fn create_pipe() -> Option<(PixReader, HANDLE)> {
        use winapi::shared::minwindef::TRUE;
        use winapi::um::handleapi::{CloseHandle, SetHandleInformation};
        use winapi::um::minwinbase::SECURITY_ATTRIBUTES;
        use winapi::um::winbase::HANDLE_FLAG_INHERIT;

        let mut sa: SECURITY_ATTRIBUTES = unsafe { std::mem::zeroed() };
        sa.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
        sa.bInheritHandle = TRUE;
        let mut read = std::ptr::null_mut();
        let mut write = std::ptr::null_mut();
        let ok = unsafe { CreatePipe(&mut read, &mut write, &mut sa as *mut _, 0) };
        if ok == 0 || read.is_null() || write.is_null() {
            if !read.is_null() {
                unsafe { CloseHandle(read) };
            }
            if !write.is_null() {
                unsafe { CloseHandle(write) };
            }
            return None;
        }
        // The parent never spawns children of its own from the read end;
        // keep it out of the child process entirely.
        let _ = unsafe { SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0) };
        Some((PixReader::new(read), write))
    }
}

/// Windows HANDLE-backed pipe reader for the drain threads.
#[cfg(windows)]
struct PixReader(winapi::shared::ntdef::HANDLE);

#[cfg(windows)]
impl PixReader {
    fn new(handle: winapi::shared::ntdef::HANDLE) -> Self {
        Self(handle)
    }
}

// The pipe read end is an OS handle owned exclusively by this wrapper and
// used from exactly one thread at a time (the drain thread).
#[cfg(windows)]
unsafe impl Send for PixReader {}

#[cfg(windows)]
impl std::io::Read for PixReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use winapi::shared::minwindef::DWORD;
        use winapi::um::fileapi::ReadFile;

        let mut read: DWORD = 0;
        let ok = unsafe {
            ReadFile(
                self.0,
                buf.as_mut_ptr() as *mut _,
                buf.len() as DWORD,
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            // Includes ERROR_BROKEN_PIPE (all writers died): that is EOF.
            return Err(std::io::Error::last_os_error());
        }
        Ok(read as usize)
    }
}

#[cfg(windows)]
impl Drop for PixReader {
    fn drop(&mut self) {
        unsafe {
            winapi::um::handleapi::CloseHandle(self.0);
        }
    }
}

// ---------------------------------------------------------------------------
// Governed workspace binding (no fallbacks)
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

// ---------------------------------------------------------------------------
// Spawn with containment
// ---------------------------------------------------------------------------

/// Pipe endpoints created for one invocation. Readers feed the drain
/// threads; writers are handed to the child and its copies are closed by
/// the parent immediately after a successful spawn so EOF is exact.
#[cfg(unix)]
fn make_pipe() -> Result<(std::fs::File, std::fs::File)> {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        anyhow::bail!(
            "cannot create subprocess pipe: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(unsafe {
        (
            std::fs::File::from_raw_fd(fds[0]),
            std::fs::File::from_raw_fd(fds[1]),
        )
    })
}

#[cfg(windows)]
fn make_pipe() -> Result<(PixReader, winapi::shared::ntdef::HANDLE)> {
    win_pipe::create_pipe().ok_or_else(|| {
        anyhow::anyhow!(
            "cannot create subprocess pipe: {}",
            std::io::Error::last_os_error()
        )
    })
}

#[cfg(unix)]
fn spawn_contained(
    exe: &str,
    argv: &[String],
    cwd: &Path,
    stdout_w: std::fs::File,
    stderr_w: std::fs::File,
) -> Result<SpawnedChild> {
    use std::os::unix::io::FromRawFd as _;
    use std::os::unix::process::CommandExt as _;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(argv)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(stdout_w))
        .stderr(std::process::Stdio::from(stderr_w));
    // New session + process group: the child's pid becomes the group id so
    // the group contains every descendant (except those that re-setpgid,
    // which process-group containment fundamentally cannot govern).
    cmd.pre_exec(|| {
        if unsafe { libc::setsid() } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    });
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            anyhow::bail!("subprocess spawn failed for executable {exe:?}: {error}")
        }
    };
    let mut containment = Containment::new();
    containment.arm(&mut child)?;
    Ok(SpawnedChild::Unix { child, containment })
}

#[cfg(windows)]
fn spawn_contained(
    exe: &str,
    argv: &[String],
    cwd: &Path,
    stdout_w: winapi::shared::ntdef::HANDLE,
    stderr_w: winapi::shared::ntdef::HANDLE,
) -> Result<SpawnedChild> {
    use winapi::um::fileapi::CreateFileW;
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::minwinbase::SECURITY_ATTRIBUTES;
    use winapi::um::processthreadsapi::{CreateProcessW, ResumeThread, STARTUPINFOW};
    use winapi::um::winbase::{CREATE_NO_WINDOW, CREATE_SUSPENDED, STARTF_USESTDHANDLES};

    // A null-device stdin keeps the child from blocking on inherited input.
    // Inheritable so CreateProcessW can actually hand it to the child.
    let nul_name: Vec<u16> = std::ffi::OsStr::new("NUL")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut nul_sa: SECURITY_ATTRIBUTES = unsafe { std::mem::zeroed() };
    nul_sa.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
    nul_sa.bInheritHandle = 1;
    let nul = unsafe {
        CreateFileW(
            nul_name.as_ptr(),
            0x80000000, // GENERIC_READ
            0x00000003, // FILE_SHARE_READ | FILE_SHARE_WRITE
            &mut nul_sa as *mut _,
            3,    // OPEN_EXISTING
            0x80, // FILE_ATTRIBUTE_NORMAL
            std::ptr::null_mut(),
        )
    };
    if nul.is_null() || nul == INVALID_HANDLE_VALUE {
        unsafe {
            CloseHandle(stdout_w);
            CloseHandle(stderr_w);
        };
        anyhow::bail!("cannot open NUL device for subprocess stdin");
    }

    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    startup.dwFlags = STARTF_USESTDHANDLES;
    startup.hStdInput = nul;
    startup.hStdOutput = stdout_w;
    startup.hStdError = stderr_w;

    let mut command_line = build_command_line(exe, argv);
    let cwd_wide: Vec<u16> = std::ffi::OsStr::new(cwd)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut process_info: winapi::um::processthreadsapi::PROCESS_INFORMATION =
        unsafe { std::mem::zeroed() };

    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            1, // inherit handles (our stdio write ends only)
            CREATE_SUSPENDED | CREATE_NO_WINDOW,
            std::ptr::null_mut(),
            cwd_wide.as_ptr(),
            &startup as *const STARTUPINFOW as *mut _,
            &mut process_info,
        )
    };
    if created == 0 {
        let error = std::io::Error::last_os_error();
        if !nul.is_null() {
            unsafe {
                CloseHandle(nul);
            };
        }
        unsafe {
            CloseHandle(stdout_w);
            CloseHandle(stderr_w);
        };
        anyhow::bail!("subprocess spawn failed for executable {exe:?}: {error}");
    }

    // Parent's own copies of the stdio write ends and the NUL stdin can go
    // now: the child holds its inherited copies, and keeping ours open
    // would block EOF for the drain threads forever.
    unsafe {
        CloseHandle(stdout_w);
        CloseHandle(stderr_w);
        CloseHandle(nul);
    }
    // Copy the (out, err) finished captures.
    let process_handle = process_info.hProcess;
    let thread_handle = process_info.hThread;
    // Parent's own write-end copies can go now: the child already holds its
    // inherited copies, and keeping ours would block EOF for our drains.
    // (The handles are also closed by process exit in the child.)
    let mut containment = Containment::new();
    // Assign the job while the child is still suspended (no code ran yet);
    // only then resume. On failure the suspended process never ran, so
    // terminate it immediately and fail closed.
    let armed = {
        let child = WinProcess {
            process: process_handle,
        };
        containment.arm(&child).is_ok()
    };
    if !armed {
        unsafe {
            winapi::um::processthreadsapi::TerminateProcess(process_handle, 1);
            CloseHandle(process_handle);
            CloseHandle(thread_handle);
        }
        return Err(anyhow::anyhow!(
            "subprocess containment unavailable: job assignment failed; process never ran"
        ));
    }
    let resumed = unsafe { ResumeThread(thread_handle) };
    unsafe {
        CloseHandle(thread_handle);
    };
    if resumed == u32::MAX {
        anyhow::bail!(
            "subprocess resume failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(SpawnedChild::Windows {
        process: WinProcess {
            process: process_handle,
        },
        containment,
    })
}

/// Build the Windows command line (exe + argv) with MSVC C-runtime
/// quoting. `CreateProcessW` parses the first token as the program path,
/// so space/tab/quote characters force quoting with backslash escapes.
#[cfg(windows)]
fn quote_cmd_arg(arg: &str) -> String {
    if arg.is_empty() || arg.contains([' ', '\t', '"']) {
        let mut out = String::from("\"");
        let mut backslashes = 0usize;
        for c in arg.chars() {
            match c {
                '\\' => backslashes += 1,
                '"' => {
                    out.push_str(&"\\".repeat(backslashes + 1));
                    out.push('"');
                    backslashes = 0;
                }
                _ => {
                    out.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                    out.push(c);
                }
            }
        }
        out.push_str(&"\\".repeat(backslashes * 2));
        out.push('"');
        out
    } else {
        arg.to_string()
    }
}

#[cfg(windows)]
fn build_command_line(exe: &str, argv: &[String]) -> Vec<u16> {
    let mut line = quote_cmd_arg(exe);
    for arg in argv {
        line.push(' ');
        line.push_str(&quote_cmd_arg(arg));
    }
    std::ffi::OsStr::new(&line)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

// ---------------------------------------------------------------------------
// Guarded handler (the capability body)
// ---------------------------------------------------------------------------

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
        if !matches!(
            key.as_str(),
            "exe" | "argv" | "cwd" | "timeoutMs" | "knownSecrets"
        ) {
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
    let request_known_secrets: Vec<String> = obj
        .get("knownSecrets")
        .map(|v| {
            let arr = v.as_array().ok_or_else(|| {
                anyhow::anyhow!("subprocess argument \"knownSecrets\" must be an array of strings")
            })?;
            if arr.len() > 256 {
                anyhow::bail!("subprocess argument \"knownSecrets\" exceeds the entry limit");
            }
            let mut out = Vec::with_capacity(arr.len());
            let mut total = 0usize;
            for item in arr {
                let s = item.as_str().ok_or_else(|| {
                    anyhow::anyhow!(
                        "subprocess argument \"knownSecrets\" must be an array of strings"
                    )
                })?;
                total += s.len();
                if total > 128 * 1024 {
                    anyhow::bail!("subprocess argument \"knownSecrets\" exceeds the size limit");
                }
                out.push(s.to_string());
            }
            Ok(out)
        })
        .transpose()?
        .unwrap_or_default();
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
    let cwd_path = cwd;

    // ---- Pipes + contained spawn. ----
    let (out_r, out_w) = make_pipe()?;
    let (err_r, err_w) = make_pipe()?;
    let mut child = spawn_contained(exe, &argv, &cwd_path, out_w, err_w)?;

    let stdout_drain = spawn_drain(out_r, config.max_output_bytes);
    let stderr_drain = spawn_drain(err_r, config.max_output_bytes);

    // ---- Deadline / cancellation / budget loop. ----
    let started = Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms);
    let status = loop {
        if let Some(status) = child.try_wait() {
            break status;
        }
        if config.cancel.load(Ordering::Relaxed) {
            child.terminate_and_reap();
            anyhow::bail!("subprocess cancelled");
        }
        if Instant::now() >= deadline {
            child.terminate_and_reap();
            anyhow::bail!("subprocess timed out after {timeout_ms}ms");
        }
        if stdout_drain.total() + stderr_drain.total() > config.max_output_bytes as u64 {
            child.terminate_and_reap();
            anyhow::bail!(
                "subprocess output exceeded the configured budget of {} bytes",
                config.max_output_bytes
            );
        }
        std::thread::sleep(POLL);
    };
    let duration_ms = started.elapsed().as_millis() as u64;

    // The direct child exited: sweep any stragglers it left behind, then
    // verify capture completion BEFORE taking the evidence snapshot. Every
    // writer died with the contained tree, so EOF is guaranteed-measurable:
    // a drain that did not finish within the grace is a failure, never a
    // silent empty or partial capture.
    child.sweep_on_normal();
    let stdout_state = stdout_drain.finish();
    let stderr_state = stderr_drain.finish();
    if !stdout_state.done || !stderr_state.done {
        anyhow::bail!("subprocess output capture did not complete");
    }
    if stdout_state.total + stderr_state.total > config.max_output_bytes as u64 {
        anyhow::bail!(
            "subprocess output exceeded the configured budget of {} bytes",
            config.max_output_bytes
        );
    }

    // Structured redaction before encoding (R3.2): the shared policy —
    // known secrets verbatim first, then credential patterns — runs on the
    // captured VALUES, which are then hex-encoded into the document. Gate-7
    // patterns cannot match hex text, so the JSON grammar survives
    // redaction and only redacted data is persisted. The effective secret
    // list binds the governed request's secrets (passed through the args)
    // with the registration-time ones, so the boundary is authoritative.
    let mut secrets = config.known_secrets.clone();
    for secret in &request_known_secrets {
        if !secrets.contains(secret) {
            secrets.push(secret.clone());
        }
    }
    let redactor = Redactor::with_known_secrets(Redactor::new(), &secrets);
    let stdout_text = redactor.redact(&String::from_utf8_lossy(&stdout_state.retained));
    let stderr_text = redactor.redact(&String::from_utf8_lossy(&stderr_state.retained));
    let exit_code = Some(status);

    Ok(serde_json::json!({
        "exitCode": exit_code,
        "success": exit_code == Some(0),
        "stdoutHex": encode_hex(stdout_text.as_bytes()),
        "stderrHex": encode_hex(stderr_text.as_bytes()),
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
