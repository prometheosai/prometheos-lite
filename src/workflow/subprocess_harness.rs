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
//! - The capability handler runs the child with a bounded lifetime:
//!   strict argument validation, an exact-match executable allowlist, a
//!   workspace-contained working directory (paths that escape the governed
//!   root refuse; there is no fallback to the process cwd), an explicit
//!   deadline, cancellation, and output caps. On refusal/timeout/cancel
//!   the child is killed and reaped before the handler returns.
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
    /// Bytes of stdout/stderr retained as evidence after completion.
    pub max_output_bytes: usize,
    /// Bytes of serialized arguments accepted per invocation.
    pub max_args_bytes: usize,
    /// Cooperative cancellation source; setting it aborts the running child.
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
            let stderr = parsed
                .get("stderr")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .trim();
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

/// Spool files for child stdout/stderr, removed on drop (best effort).
struct SpoolFiles {
    stdout: PathBuf,
    stderr: PathBuf,
}

impl Drop for SpoolFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.stdout);
        let _ = std::fs::remove_file(&self.stderr);
    }
}

fn spool_files() -> Result<SpoolFiles> {
    let nonce = SPOOL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir();
    let stdout = dir.join(format!("prometheos-subprocess-{pid}-{nonce}.out"));
    let stderr = dir.join(format!("prometheos-subprocess-{pid}-{nonce}.err"));
    let out = std::fs::File::create(&stdout)
        .with_context(|| format!("cannot create subprocess stdout spool {}", stdout.display()))?;
    let err = std::fs::File::create(&stderr)
        .with_context(|| format!("cannot create subprocess stderr spool {}", stderr.display()))?;
    drop(out);
    drop(err);
    Ok(SpoolFiles { stdout, stderr })
}

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
    let spool = spool_files()?;
    let out_file = std::fs::OpenOptions::new()
        .write(true)
        .open(&spool.stdout)
        .with_context(|| {
            format!(
                "cannot open subprocess stdout spool {}",
                spool.stdout.display()
            )
        })?;
    let err_file = std::fs::OpenOptions::new()
        .write(true)
        .open(&spool.stderr)
        .with_context(|| {
            format!(
                "cannot open subprocess stderr spool {}",
                spool.stderr.display()
            )
        })?;

    let started = Instant::now();
    let deadline = started + Duration::from_millis(timeout_ms);
    let mut cmd = Command::new(exe);
    cmd.args(&argv)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file));
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            anyhow::bail!("subprocess spawn failed for executable {exe:?}: {error}")
        }
    };

    // ---- Deadline / cancellation / high-water loop. ----
    // The child is always killed AND reaped before any refusal returns.
    let high_water = (config.max_output_bytes as u64).saturating_mul(4) + 1024 * 1024;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if config.cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("subprocess cancelled");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("subprocess timed out after {timeout_ms}ms");
        }
        let written = std::fs::metadata(&spool.stdout)
            .map(|m| m.len())
            .unwrap_or(0)
            + std::fs::metadata(&spool.stderr)
                .map(|m| m.len())
                .unwrap_or(0);
        if written > high_water {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("subprocess output exceeded the configured limit");
        }
        std::thread::sleep(POLL);
    };
    let duration_ms = started.elapsed().as_millis() as u64;

    let (stdout, stdout_len) = read_capped(&spool.stdout, config.max_output_bytes);
    let (stderr, stderr_len) = read_capped(&spool.stderr, config.max_output_bytes);
    let output_cap = u64::try_from(config.max_output_bytes).unwrap_or(u64::MAX);
    let truncated = stdout_len > output_cap || stderr_len > output_cap;
    let exit_code = status.code();

    Ok(serde_json::json!({
        "exitCode": exit_code,
        "success": exit_code == Some(0),
        "stdout": stdout,
        "stderr": stderr,
        "truncated": truncated,
        "durationMs": duration_ms,
    })
    .to_string())
}

/// Read a spool file, retaining at most `cap` bytes. Returns the retained
/// text (lossy) and the file's total byte length for truncation evidence.
fn read_capped(path: &Path, cap: usize) -> (String, u64) {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return (String::new(), 0),
    };
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut retained = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let remaining = cap.saturating_sub(retained.len());
                if remaining == 0 {
                    // Drain the rest without retaining.
                    continue;
                }
                retained.extend_from_slice(&buf[..remaining.min(n)]);
            }
            Err(_) => break,
        }
    }
    (String::from_utf8_lossy(&retained).into_owned(), total)
}
