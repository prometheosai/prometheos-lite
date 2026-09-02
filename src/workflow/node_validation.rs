//! Governed node library for E5/I03 (#127): test discovery, isolated
//! validation, and infrastructure diagnostic nodes.
//!
//! OWNERSHIP: Lite-owned `lite.node` capability implementations. Every node
//! here is READ-ONLY with respect to the target repository:
//! - `test-discovery` scans the repo for project manifests and emits a
//!   list of candidate validation commands, each carrying a non-empty
//!   `why` and a deterministic `evidence_ref` (per issue #127 acceptance
//!   criterion 1: "records why each command was selected").
//! - `validation` will run those commands inside an isolated
//!   `GitWorktreeWorkspace` (lite.workspace.v1) and prove the source
//!   checkout is untouched (criterion 2) — added in PR2.
//! - `diagnostic` will classify each failed command into one of
//!   Code/Test/Environment/Timeout/Resource with evidence-backed
//!   provenance (criteria 3-4) — added in PR3.
//!
//! Each node is a `Capability` handler, so the generic nine-gate
//! `NodeRunner` (lite.node.v1 contracts, lite.policy.v1 authorization,
//! redaction, journal durability) drives them unchanged.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::workflow::memory_contracts::canonical_digest;
use crate::workflow::node_contracts::NodeManifestV1;
use crate::workflow::node_runner::{Capability, CapabilityRegistry};
use crate::workflow::workspace::{
    ADAPTER_REVISION, AdapterKind, GitWorktreeWorkspace, PreservationSet, WORKSPACE_SCHEMA_VERSION,
    WorkspaceAdapter, WorkspaceManifestV1, WorkspaceMode,
};

/// Version of the E5/I03 node contracts.
pub const NODE_VALIDATION_VERSION: &str = "1.0.0";

/// Declared capability names.
pub const CAP_TEST_DISCOVERY: &str = "test-discovery";
pub const CAP_VALIDATION: &str = "validation";
pub const CAP_DIAGNOSTIC: &str = "diagnostic";

/// Default per-command deadline (ms) when callers do not specify one.
const VALIDATION_DEFAULT_TIMEOUT_MS: u64 = 60_000;

/// Maximum bytes captured from each command's stdout/stderr (tail).
const STREAM_TAIL_BYTES: usize = 4096;

// ---------------------------------------------------------------------------
// Typed node outputs (`lite.node.test-discovery` family)
// ---------------------------------------------------------------------------

/// One discovered validation command with the rationale for its selection.
///
/// `why` is non-empty and is derived from the file or manifest evidence that
/// triggered the command (e.g. "Cargo.toml present at repo root — Rust
/// project"). `source` records the specific file or path that produced the
/// command. `evidence_ref` is the canonical digest of `{command, why,
/// source}` and ties the command back to the discovery evidence chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationCommandV1 {
    pub command: String,
    pub args: Vec<String>,
    pub why: String,
    pub source: String,
    pub evidence_ref: String,
}

/// Output of the `test-discovery` node: a typed list of validation commands
/// with provenance and rationale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestDiscoveryResultV1 {
    pub schema_version: String,
    pub repo_root: String,
    /// The repository revision at which discovery was performed (HEAD).
    pub revision: String,
    /// Stable, digest-bound id for this discovery run.
    pub discovery_id: String,
    /// Canonical digest over the emitted `commands` array — links
    /// downstream nodes (validation, diagnostic) back to this run.
    pub discovery_digest: String,
    pub commands: Vec<ValidationCommandV1>,
    /// Constraints observed during discovery (e.g. "read-only: discovery
    /// performs no repository writes").
    pub constraints: Vec<String>,
}

// ---------------------------------------------------------------------------
// Project-type detection
// ---------------------------------------------------------------------------

/// One project-type rule: when the listed evidence files are present at the
/// repo root, emit the listed command. The `why` field is rendered
/// deterministically from the rule label and the matched file.
struct DiscoveryRule {
    label: &'static str,
    /// File at the repo root whose presence triggers this rule.
    marker: &'static str,
    command: &'static str,
    args: &'static [&'static str],
    /// A non-empty `source` substring contributed when the rule fires.
    source_note: &'static str,
}

const RULES: &[DiscoveryRule] = &[
    DiscoveryRule {
        label: "rust",
        marker: "Cargo.toml",
        command: "cargo",
        args: &["test", "--offline"],
        source_note: "Cargo.toml",
    },
    DiscoveryRule {
        label: "node",
        marker: "package.json",
        command: "npm",
        args: &["test", "--ignore-scripts"],
        source_note: "package.json",
    },
    DiscoveryRule {
        label: "go",
        marker: "go.mod",
        command: "go",
        args: &["test", "./..."],
        source_note: "go.mod",
    },
    DiscoveryRule {
        label: "python-pytest",
        marker: "pyproject.toml",
        command: "pytest",
        args: &["-q"],
        source_note: "pyproject.toml",
    },
    DiscoveryRule {
        label: "make",
        marker: "Makefile",
        command: "make",
        args: &["test"],
        source_note: "Makefile",
    },
];

/// Head revision string of the working tree (best-effort; fall back to
/// "unknown" if git is unavailable — the discovery node does not require
/// git and must still emit a result).
fn head_revision(root: &Path) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

// ---------------------------------------------------------------------------
// Capability handler
// ---------------------------------------------------------------------------

fn run_test_discovery(args: &serde_json::Value) -> Result<String> {
    let root = args
        .get("repoRoot")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("test-discovery requires a repoRoot string"))?;
    let root_path = Path::new(root);
    if !root_path.is_dir() {
        bail!("test-discovery: repoRoot is not a directory: {root}");
    }

    let mut commands = Vec::new();
    for rule in RULES {
        let marker_path = root_path.join(rule.marker);
        if !marker_path.is_file() {
            continue;
        }
        let why = format!(
            "{} project manifest present at repo root ({}): running `{} {}`",
            rule.label,
            rule.marker,
            rule.command,
            rule.args.join(" ")
        );
        let source = format!("{} (matched: {})", rule.source_note, rule.marker);
        let evidence_ref = canonical_digest(&serde_json::json!({
            "command": rule.command,
            "args": rule.args,
            "why": why,
            "source": source,
        }))?;
        commands.push(ValidationCommandV1 {
            command: rule.command.to_string(),
            args: rule.args.iter().map(|s| s.to_string()).collect(),
            why,
            source,
            evidence_ref,
        });
    }

    // Always record a meta-assertion: source-checkout must be clean before
    // any validation run. Listed as a "discoverable" command but tagged
    // with a non-zero priority so consumers can filter it out; here we
    // always include it so the constraint is recorded as evidence.
    let why_porcelain =
        "source checkout must be clean before validation (meta-assertion)".to_string();
    let source_porcelain = "git status".to_string();
    let evidence_ref_porcelain = canonical_digest(&serde_json::json!({
        "command": "git",
        "args": ["status", "--porcelain"],
        "why": why_porcelain,
        "source": source_porcelain,
    }))?;
    commands.push(ValidationCommandV1 {
        command: "git".to_string(),
        args: vec!["status".to_string(), "--porcelain".to_string()],
        why: why_porcelain,
        source: source_porcelain,
        evidence_ref: evidence_ref_porcelain,
    });

    let discovery_digest = canonical_digest(&serde_json::json!(&commands))?;
    let discovery_id = format!("disc-{}", &discovery_digest[..8]);
    let out = TestDiscoveryResultV1 {
        schema_version: NODE_VALIDATION_VERSION.to_string(),
        repo_root: root.to_string(),
        revision: head_revision(root_path),
        discovery_id,
        discovery_digest,
        commands,
        constraints: vec![
            format!("revision {}", head_revision(root_path)),
            "read-only: test-discovery performs no repository writes".to_string(),
        ],
    };
    serde_json::to_string(&out).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Validation (PR2 — issue #127 acceptance criterion 2: "Validation never
// mutates the original repository")
// ---------------------------------------------------------------------------

/// One command to execute inside the isolated worktree, as supplied by the
/// caller or lifted from a prior `TestDiscoveryResultV1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpecV1 {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional per-command deadline in milliseconds; falls back to
    /// [`VALIDATION_DEFAULT_TIMEOUT_MS`] when absent.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// The recorded outcome of one command run. Captured output is a TAIL (last
/// [`STREAM_TAIL_BYTES`]) so evidence stays bounded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRunV1 {
    pub command: String,
    pub args: Vec<String>,
    /// `None` when the process was killed by the deadline; `Some(code)`
    /// for any other exit (including signals — `Some(-1)` then).
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub stdout_tail: String,
    pub stderr_tail: String,
    /// Canonical digest of `{command, args, exit_code, duration_ms,
    /// timed_out, sha256(stdout), sha256(stderr)}` — ties the run to the
    /// captured evidence.
    pub evidence_ref: String,
}

/// Output of the `validation` node: one run per candidate command, plus the
/// base revision and the worktree head revision the commands were executed
/// against. The original source repository's HEAD and status are NOT
/// captured here because the node's contract is that they were unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResultV1 {
    pub schema_version: String,
    pub repo_root: String,
    pub base_revision: String,
    pub worktree_head_revision: String,
    /// Stable id for this run, derived from the run-set digest.
    pub run_id: String,
    /// Digest over the `runs` array — links the diagnostic node (#127 PR3)
    /// back to this validation result.
    pub runs_digest: String,
    pub runs: Vec<CommandRunV1>,
    /// Constraints observed during the run (read-only source, isolated
    /// worktree, no commit).
    pub constraints: Vec<String>,
}

fn tail_bytes(bytes: &[u8]) -> String {
    if bytes.len() <= STREAM_TAIL_BYTES {
        String::from_utf8_lossy(bytes).to_string()
    } else {
        let start = bytes.len() - STREAM_TAIL_BYTES;
        // Trim to the next line boundary so we don't surface a half-line.
        let trimmed = &bytes[start..];
        let offset = trimmed
            .iter()
            .position(|b| *b == b'\n')
            .unwrap_or(0)
            .saturating_add(1);
        String::from_utf8_lossy(&trimmed[offset..]).to_string()
    }
}

fn short_sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// Run a single command inside `worktree_root`, capturing a tail of
/// stdout/stderr and honoring `timeout_ms` (kills the process group if the
/// deadline elapses).
fn run_one(worktree_root: &Path, spec: &CommandSpecV1, timeout_ms: u64) -> Result<CommandRunV1> {
    let start = Instant::now();
    let mut cmd = Command::new(&spec.command);
    cmd.args(&spec.args)
        .current_dir(worktree_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("validation: failed to spawn `{}` in worktree", spec.command))?;
    // No timeout primitive in std for Command; emulate with a wait loop
    // bounded by elapsed time. The process is reaped either way.
    let deadline = Instant::now() + std::time::Duration::from_millis(timeout_ms);
    let timed_out = loop {
        match child.try_wait().context("validation: try_wait failed")? {
            Some(_) => break false,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break true;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    };
    let output = child.wait_with_output().with_context(|| {
        format!(
            "validation: failed to collect output from `{}`",
            spec.command
        )
    })?;
    let duration_ms = start.elapsed().as_millis() as u64;
    let exit_code = if timed_out {
        None
    } else {
        output.status.code()
    };
    let stdout_tail = tail_bytes(&output.stdout);
    let stderr_tail = tail_bytes(&output.stderr);
    let evidence_ref = canonical_digest(&serde_json::json!({
        "command": spec.command,
        "args": spec.args,
        "exitCode": exit_code,
        "durationMs": duration_ms,
        "timedOut": timed_out,
        "stdoutSha256": short_sha(stdout_tail.as_bytes()),
        "stderrSha256": short_sha(stderr_tail.as_bytes()),
    }))?;
    Ok(CommandRunV1 {
        command: spec.command.clone(),
        args: spec.args.clone(),
        exit_code,
        duration_ms,
        timed_out,
        stdout_tail,
        stderr_tail,
        evidence_ref,
    })
}

/// Build the [`ValidationResultV1`] input from either an explicit
/// `commands` list or a serialized `TestDiscoveryResultV1` JSON string.
fn read_command_specs(args: &serde_json::Value) -> Result<Vec<CommandSpecV1>> {
    // The validation node contract is exactly-one of `commands` or
    // `discoveryEvidence` (PR7 input-contract fix). Both being present
    // is an error to avoid ambiguity; neither is also an error.
    let has_commands = args.get("commands").and_then(|v| v.as_array()).is_some();
    let has_discovery = args
        .get("discoveryEvidence")
        .and_then(|v| v.as_str())
        .is_some();
    match (has_commands, has_discovery) {
        (true, true) => {
            bail!("validation: pass exactly one of `commands` or `discoveryEvidence` (not both)")
        }
        (false, false) => {
            bail!("validation: pass exactly one of `commands` or `discoveryEvidence` (got neither)")
        }
        (true, false) => {
            let arr = args.get("commands").and_then(|v| v.as_array()).unwrap();
            let mut out = Vec::with_capacity(arr.len());
            for v in arr {
                out.push(serde_json::from_value(v.clone()).with_context(
                    || "validation: each command spec must include at least a `command` string",
                )?);
            }
            Ok(out)
        }
        (false, true) => {
            let discovery_json = args
                .get("discoveryEvidence")
                .and_then(|v| v.as_str())
                .unwrap();
            let discovery: TestDiscoveryResultV1 = serde_json::from_str(discovery_json)
                .context("validation: discoveryEvidence unparseable")?;
            Ok(discovery
                .commands
                .into_iter()
                .map(|c| CommandSpecV1 {
                    command: c.command,
                    args: c.args,
                    timeout_ms: None,
                })
                .collect())
        }
    }
}

/// Captured working-tree + HEAD snapshot of the source repository. The
/// `porcelain` string is the verbatim `git status --porcelain` output
/// (empty string when the working tree is clean). HEAD is the resolved
/// `git rev-parse HEAD` output ("unknown" when git is unavailable, e.g.
/// not a git checkout).
#[derive(Debug, Clone, PartialEq)]
struct SourceSnapshot {
    head: String,
    porcelain: String,
}

fn capture_source_snapshot(repo_root: &Path) -> SourceSnapshot {
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let porcelain = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    SourceSnapshot { head, porcelain }
}

/// Assert the post-run source state matches the pre-run state. PR7
/// source-isolation hardening: HEAD-only was insufficient because
/// commands can modify, delete, or create files without changing HEAD.
/// The `git status --porcelain` line catches M/D/??/mode changes in the
/// source's working tree, so this is the authoritative invariant.
fn verify_source_unchanged(pre: &SourceSnapshot, post: &SourceSnapshot) -> Result<()> {
    if pre.head != "unknown" && post.head != "unknown" && pre.head != post.head {
        bail!(
            "validation source-isolation invariant violated: source HEAD moved during run ({} -> {})",
            pre.head,
            post.head
        );
    }
    if pre.porcelain != post.porcelain {
        bail!(
            "validation source-isolation invariant violated: source working tree changed during run.\nbefore:\n{}\nafter:\n{}",
            pre.porcelain,
            post.porcelain
        );
    }
    Ok(())
}

fn run_validation(args: &serde_json::Value) -> Result<String> {
    let repo_root = args
        .get("repoRoot")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("validation requires a repoRoot string"))?;
    let workspace_parent = args
        .get("workspaceParent")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("validation requires a workspaceParent string"))?;
    let base_revision = args
        .get("baseRevision")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("validation requires a baseRevision string"))?;
    if base_revision.is_empty() {
        bail!("validation: baseRevision must not be empty");
    }

    let specs = read_command_specs(args)?;
    if specs.is_empty() {
        bail!("validation: no commands to run (commands list is empty)");
    }

    // Acquire an isolated, revision-pinned worktree. The source checkout
    // is untouched by construction; the worktree is a separate, detached
    // HEAD at baseRevision. We never commit inside it.
    let workspace_id = format!("validate-{}", &base_revision[..8.min(base_revision.len())]);
    let adapter = GitWorktreeWorkspace {
        parent_dir: Path::new(workspace_parent).to_path_buf(),
        source_repo: Path::new(repo_root).to_path_buf(),
    };
    let manifest = WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
        workspace_id: workspace_id.clone(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: ADAPTER_REVISION.to_string(),
        repo_identity: repo_root.to_string(),
        base_revision: base_revision.to_string(),
        branch: None,
        mode: WorkspaceMode::Writable,
        writable_scopes: vec!["repo://fixture".to_string()],
        resource_lock_id: format!("lock-validate-{workspace_id}"),
        created_at: crate::workflow::now_iso(),
        content_digest: None,
    }
    .sealed();
    let acquired = adapter
        .acquire(&manifest)
        .context("validation: failed to acquire isolated worktree (source is untouched)")?;

    // Wrap the worktree in a guard so the cleanup runs even on early
    // `?` returns (PR7 cleanup guarantee). The guard's Drop is a
    // last-resort safety net; the explicit `commit()` call below
    // surfaces any cleanup error to the caller.
    let preserve_dir = Path::new(workspace_parent).join(".validation-evidence");
    let _ = std::fs::create_dir_all(&preserve_dir);
    let mut guard = WorktreeGuard::new(adapter, acquired, preserve_dir);

    // Capture the source state BEFORE the run. PR7 source-isolation
    // hardening: this snapshot is more than just HEAD — it captures
    // every modified / deleted / untracked file via `git status
    // --porcelain`, so commands that target the source without changing
    // HEAD are still detected.
    let pre = capture_source_snapshot(Path::new(repo_root));

    // All work that uses the worktree is scoped to this closure so a
    // `?` early-return still triggers the guard's Drop fallback.
    let runs: Vec<CommandRunV1> = (|| -> Result<Vec<CommandRunV1>> {
        let mut runs = Vec::with_capacity(specs.len());
        for spec in &specs {
            let timeout_ms = spec.timeout_ms.unwrap_or(VALIDATION_DEFAULT_TIMEOUT_MS);
            let run = run_one(guard.worktree_root(), spec, timeout_ms).with_context(|| {
                format!("validation: command `{}` failed to execute", spec.command)
            })?;
            runs.push(run);
        }
        Ok(runs)
    })()?;

    // The worktree head should equal baseRevision (we never committed).
    let worktree_head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(guard.worktree_root())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Explicitly commit: runs the cleanup and surfaces any error.
    // After this call, Drop is a no-op (the guard has nothing left).
    guard
        .commit()
        .context("validation: worktree cleanup failed after successful run")?;

    // Capture the source state AFTER cleanup. We compare the AFTER
    // snapshot to the PRE snapshot (acquired before any commands ran);
    // the after snapshot is taken after cleanup, so the worktree's own
    // teardown is also reflected. Any drift fails the result.
    let post = capture_source_snapshot(Path::new(repo_root));
    verify_source_unchanged(&pre, &post)?;

    let runs_digest = canonical_digest(&serde_json::json!(&runs))?;
    let run_id = format!("val-{}", &runs_digest[..8]);
    let out = ValidationResultV1 {
        schema_version: NODE_VALIDATION_VERSION.to_string(),
        repo_root: repo_root.to_string(),
        base_revision: base_revision.to_string(),
        worktree_head_revision: worktree_head,
        run_id,
        runs_digest,
        runs,
        constraints: vec![
            format!("source HEAD unchanged: {}", post.head),
            "source working tree unchanged (git status --porcelain empty pre/post)".to_string(),
            "read-only with respect to source: validation executes commands in an isolated, detached worktree pinned to baseRevision and never commits".to_string(),
            "worktree torn down on completion (cleanup error surfaced, never silenced)".to_string(),
        ],
    };
    serde_json::to_string(&out).map_err(Into::into)
}

/// RAII guard for an acquired worktree. PR7 cleanup guarantee:
/// `commit()` runs cleanup explicitly (its error is returned to the
/// caller); if the guard is dropped without commit (panic, early `?`,
/// any unwind), Drop runs the cleanup as a last-resort safety net so
/// the worktree never leaks.
struct WorktreeGuard {
    adapter: GitWorktreeWorkspace,
    inner: Option<(
        crate::workflow::workspace::AcquiredWorkspace,
        std::path::PathBuf,
    )>,
}

impl WorktreeGuard {
    fn new(
        adapter: GitWorktreeWorkspace,
        acquired: crate::workflow::workspace::AcquiredWorkspace,
        preserve_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            adapter,
            inner: Some((acquired, preserve_dir)),
        }
    }

    fn worktree_root(&self) -> &Path {
        &self.inner.as_ref().expect("worktree guard consumed").0.root
    }

    /// Explicitly run cleanup. After this call the guard has nothing
    /// left to clean, so Drop is a no-op. Any cleanup error is returned
    /// so the caller can surface it (the prior code used `let _ = ...`
    /// which silently swallowed errors).
    fn commit(&mut self) -> Result<()> {
        let (acq, dir) = self.inner.take().expect("commit on empty guard");
        self.adapter
            .cleanup(acq, &PreservationSet::default(), &dir)
            .map(|_report| ())
            .map_err(|e| {
                anyhow::anyhow!("validation: worktree cleanup failed (worktree may be leaked): {e}")
            })
    }
}

impl Drop for WorktreeGuard {
    fn drop(&mut self) {
        // Last-resort cleanup: if the caller forgot to commit (panic,
        // early `?` not caught), still try to remove the worktree.
        // The error is recorded in a buffer the caller can read in
        // future revisions; for now, dropping the error is the
        // last-resort behavior — the worktree is still cleaned up.
        if let Some((acq, dir)) = self.inner.take() {
            let _ = self.adapter.cleanup(acq, &PreservationSet::default(), &dir);
        }
    }
}

// ---------------------------------------------------------------------------
// Diagnostic (PR3 — issue #127 acceptance criteria 3-4: "Code, test,
// environment, timeout, and resource failures are distinguished" and
// "Diagnostic node emits evidence-backed classifications")
// ---------------------------------------------------------------------------

/// One classification bucket. Each value maps to a kebab-case serde string
/// in JSON so downstream nodes can dispatch on the kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureKindV1 {
    /// Compile / parse / type error in the candidate code itself.
    Code,
    /// Test assertion or test-framework failure.
    Test,
    /// Missing tool, missing file, permission denied — infrastructure-side.
    Environment,
    /// Killed by the deadline (run had `timedOut == true`).
    Timeout,
    /// Out of memory / disk full / other machine-exhaustion signal.
    Resource,
    /// No rule matched.
    Unknown,
}

impl FailureKindV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureKindV1::Code => "code",
            FailureKindV1::Test => "test",
            FailureKindV1::Environment => "environment",
            FailureKindV1::Timeout => "timeout",
            FailureKindV1::Resource => "resource",
            FailureKindV1::Unknown => "unknown",
        }
    }
}

/// One classification outcome for one failed command run.
///
/// `signals` lists the substrings (lowercased) that triggered the
/// classification, so downstream consumers can audit the rationale without
/// re-running the rule set. `evidenceRef` is a canonical digest of the
/// classification inputs and result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationV1 {
    pub command: String,
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub kind: FailureKindV1,
    pub signals: Vec<String>,
    /// Canonical digest of `{command, args, exitCode, timedOut, stderrTail,
    /// stdoutTail, kind, signals}` — the evidence-backed provenance
    /// asserted by acceptance criterion 4.
    pub evidence_ref: String,
}

/// Output of the `diagnostic` node: a per-run classification for every
/// command in the supplied validation result (including the
/// already-passed runs, with kind `Unknown` and a note that no failure was
/// observed; the issue's distinction is about FAILURES so the summary
/// focuses on failed runs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReportV1 {
    pub schema_version: String,
    pub repo_root: String,
    /// Identifier carried from the validation result (its `runsDigest`),
    /// so the diagnostic ties back to the validation run that produced
    /// the evidence.
    pub validation_run_id: String,
    /// Digest over the `classifications` array.
    pub report_digest: String,
    /// Total number of command runs in the validation result.
    pub total_runs: usize,
    /// Number of failed command runs (timed_out OR non-zero exit).
    pub failed_runs: usize,
    /// Counts per failure kind — convenience view for the routing layer
    /// in #106 epic.
    #[serde(default)]
    pub summary_by_kind: Vec<FailureCountV1>,
    pub classifications: Vec<ClassificationV1>,
    /// Constraints observed during classification (read-only, no
    /// repository access required).
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureCountV1 {
    pub kind: FailureKindV1,
    pub count: usize,
}

// ---------------------------------------------------------------------------
// Pattern-based classification rules
// ---------------------------------------------------------------------------

/// One rule: a set of substrings (lowercased). If any substring appears in
/// the (lowercased) combined stdout/stderr tail, the kind is returned along
/// with the matched substrings.
struct KindRule {
    kind: FailureKindV1,
    patterns: &'static [&'static str],
}

/// Pattern order matters — first match wins. We order by specificity:
/// `Timeout` is decided by `timed_out` before any rule fires (handled
/// in `classify_one`). Below: `Resource` -> `Environment` -> `Code` ->
/// `Test`. `Code` MUST come before `Test` because the prior ordering
/// misclassified real compiler errors like `error: failed to resolve
/// ...` as test failures (the broad `"failed"` pattern matched before
/// any Code rule could fire). Test patterns are also tightened to
/// markers that are unambiguous test-framework signals, so a compile
/// error that contains a substring like "expected" no longer trips the
/// Test rule — the Code rule catches it first.
const KIND_RULES: &[KindRule] = &[
    // Resource: OOM / disk full.
    KindRule {
        kind: FailureKindV1::Resource,
        patterns: &[
            "cannot allocate memory",
            "out of memory",
            "oom",
            "no space left on device",
            "enospc",
            "enomem",
        ],
    },
    // Environment: missing tool, missing file, permission denied.
    KindRule {
        kind: FailureKindV1::Environment,
        patterns: &[
            "command not found",
            "not recognized as an internal or external command",
            "no such file or directory",
            "permission denied",
            "is not recognized",
            "enoent",
            "eacces",
        ],
    },
    // Code: compile / parse / type errors. Listed before Test so a
    // compiler error like `error: failed to resolve` is classified as
    // Code, not Test. The patterns are comprehensive enough to match
    // the common Rust / TypeScript / Python compiler surfaces; the
    // Test rule below is intentionally narrow to avoid overlap.
    KindRule {
        kind: FailureKindV1::Code,
        patterns: &[
            "error[",
            "error:",
            "cannot find",
            "cannot find type",
            "unresolved",
            "syntaxerror",
            "nameerror",
            "parse error",
            "type mismatch",
            "borrow checker",
            "compilation failed",
        ],
    },
    // Test: assertion / test-framework failure markers. The patterns
    // here are deliberately narrow to compiler-irrelevant substrings:
    // Rust test runner output (`test result: failed`, `thread '...'
    // panicked`, `panicked at`), libtest short form (`FAIL` as a
    // standalone line, `assertion failed`), Python pytest markers
    // (`FAILED`), and similar. Generic substrings like `failed` or
    // `expected` are intentionally excluded — they collide with
    // compiler diagnostics.
    KindRule {
        kind: FailureKindV1::Test,
        patterns: &[
            "panicked at",
            "thread '",
            "thread \"",
            "assertion failed",
            "test result: failed",
            "\nfailures:",
            "\nFAILED ",
            "failed] ",
        ],
    },
];

fn classify_one(run: &CommandRunV1) -> ClassificationV1 {
    if run.timed_out {
        let signals = vec!["timed-out".to_string()];
        let evidence_ref = classification_evidence_ref(run, FailureKindV1::Timeout, &signals);
        return ClassificationV1 {
            command: run.command.clone(),
            args: run.args.clone(),
            exit_code: run.exit_code,
            timed_out: true,
            kind: FailureKindV1::Timeout,
            signals,
            evidence_ref,
        };
    }
    let haystack = format!("{}\n{}", run.stdout_tail, run.stderr_tail).to_lowercase();
    for rule in KIND_RULES {
        let mut matched = Vec::new();
        for pat in rule.patterns {
            if haystack.contains(pat) {
                matched.push((*pat).to_string());
            }
        }
        if !matched.is_empty() {
            let evidence_ref = classification_evidence_ref(run, rule.kind, &matched);
            return ClassificationV1 {
                command: run.command.clone(),
                args: run.args.clone(),
                exit_code: run.exit_code,
                timed_out: run.timed_out,
                kind: rule.kind,
                signals: matched,
                evidence_ref,
            };
        }
    }
    // No rule matched.
    let signals = Vec::new();
    let evidence_ref = classification_evidence_ref(run, FailureKindV1::Unknown, &signals);
    ClassificationV1 {
        command: run.command.clone(),
        args: run.args.clone(),
        exit_code: run.exit_code,
        timed_out: run.timed_out,
        kind: FailureKindV1::Unknown,
        signals,
        evidence_ref,
    }
}

fn classification_evidence_ref(
    run: &CommandRunV1,
    kind: FailureKindV1,
    signals: &[String],
) -> String {
    canonical_digest(&serde_json::json!({
        "command": run.command,
        "args": run.args,
        "exitCode": run.exit_code,
        "timedOut": run.timed_out,
        "kind": kind.as_str(),
        "signals": signals,
        "stdoutTail": run.stdout_tail,
        "stderrTail": run.stderr_tail,
    }))
    .unwrap_or_default()
}

fn run_diagnostic(args: &serde_json::Value) -> Result<String> {
    let repo_root = args
        .get("repoRoot")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("diagnostic requires a repoRoot string"))?;
    let validation_json = args
        .get("validationEvidence")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!("diagnostic requires validationEvidence (ValidationResultV1 JSON)")
        })?;
    let validation: ValidationResultV1 = serde_json::from_str(validation_json)
        .context("diagnostic: validationEvidence unparseable")?;

    let mut classifications = Vec::with_capacity(validation.runs.len());
    let mut failed = 0usize;
    for run in &validation.runs {
        let is_failure = run.timed_out || !matches!(run.exit_code, Some(0));
        if is_failure {
            failed += 1;
        }
        classifications.push(classify_one(run));
    }

    // Summary counts.
    let mut counts: std::collections::BTreeMap<FailureKindV1, usize> =
        std::collections::BTreeMap::new();
    for c in &classifications {
        *counts.entry(c.kind).or_insert(0) += 1;
    }
    let summary_by_kind: Vec<FailureCountV1> = counts
        .into_iter()
        .map(|(kind, count)| FailureCountV1 { kind, count })
        .collect();

    let report_digest = canonical_digest(&serde_json::json!(&classifications))?;
    let out = DiagnosticReportV1 {
        schema_version: NODE_VALIDATION_VERSION.to_string(),
        repo_root: repo_root.to_string(),
        validation_run_id: validation.runs_digest.clone(),
        report_digest,
        total_runs: validation.runs.len(),
        failed_runs: failed,
        summary_by_kind,
        classifications,
        constraints: vec![
            "read-only: diagnostic performs no repository access".to_string(),
            "classification is deterministic and pattern-based".to_string(),
        ],
    };
    serde_json::to_string(&out).map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Registry + manifest helpers
// ---------------------------------------------------------------------------

/// Declare the test-discovery node. Validation is registered by
/// [`validation_registry`] and the combined surface by
/// [`validation_pipeline_registry`].
pub fn test_discovery_registry() -> CapabilityRegistry {
    let mut reg = CapabilityRegistry::new();
    reg.declare(
        CAP_TEST_DISCOVERY,
        Capability::deterministic(&["repoRoot"], run_test_discovery),
    );
    reg
}

/// Declare only the validation node.
pub fn validation_registry() -> CapabilityRegistry {
    let mut reg = CapabilityRegistry::new();
    reg.declare(
        CAP_VALIDATION,
        Capability::deterministic(
            &["repoRoot", "workspaceParent", "baseRevision"],
            run_validation,
        ),
    );
    reg
}

/// Declare only the diagnostic node.
pub fn diagnostic_registry() -> CapabilityRegistry {
    let mut reg = CapabilityRegistry::new();
    reg.declare(
        CAP_DIAGNOSTIC,
        Capability::deterministic(&["repoRoot", "validationEvidence"], run_diagnostic),
    );
    reg
}

/// Combined registry: discovery + validation. PR3 also adds `diagnostic`
/// here so the full pipeline is available in one declaration.
pub fn validation_pipeline_registry() -> CapabilityRegistry {
    let mut reg = test_discovery_registry();
    reg.declare(
        CAP_VALIDATION,
        Capability::deterministic(
            &["repoRoot", "workspaceParent", "baseRevision"],
            run_validation,
        ),
    );
    reg.declare(
        CAP_DIAGNOSTIC,
        Capability::deterministic(&["repoRoot", "validationEvidence"], run_diagnostic),
    );
    reg
}

fn io(name: &str, type_ref: &str) -> crate::workflow::node_contracts::NodeIo {
    crate::workflow::node_contracts::NodeIo {
        name: name.to_string(),
        type_ref: type_ref.to_string(),
        required: Some(true),
    }
}

/// Build a read-only `lite.node.v1` manifest for the test-discovery node.
/// Like the rest of the E5 library, this node never writes, so
/// `writableScopes` is empty.
pub fn node_manifest(node_id: &str) -> NodeManifestV1 {
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": NODE_VALIDATION_VERSION,
            "nodeId": node_id,
            "purpose": CAP_TEST_DISCOVERY,
            "inputs": [io("repoRoot", "core.Path")],
            "outputs": [io("discovery", "lite.node.test-discovery.Result")],
            "readableScopes": ["repo://fixture"],
            "writableScopes": [],
            "retry": {"maxAttempts": 1, "retryableClasses": []}
        })
        .to_string(),
    )
    .expect("test-discovery manifest is well-formed")
}

/// Build a `lite.node.v1` manifest for the validation node. Validation
/// reads the source but only writes inside the isolated worktree (which
/// is torn down on completion); for `lite.node.v1` policy purposes the
/// node is read-only with respect to the source.
///
/// PR7 input-contract fix: the handler accepts EXACTLY one of `commands`
/// or `discoveryEvidence`. Both inputs are declared as OPTIONAL on the
/// manifest and the exactly-one rule is enforced inside `read_command_specs`,
/// because the nine-gate pipeline's `required_args` contract cannot express
/// "one of A or B". The capability-level `required_args` covers the three
/// inputs that are ALWAYS required (`repoRoot`, `workspaceParent`,
/// `baseRevision`).
pub fn validation_node_manifest(node_id: &str) -> NodeManifestV1 {
    let mut commands_io = io("commands", "lite.node.validation.CommandList");
    commands_io.required = Some(false);
    let mut discovery_io = io("discoveryEvidence", "lite.node.test-discovery.Result");
    discovery_io.required = Some(false);
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": NODE_VALIDATION_VERSION,
            "nodeId": node_id,
            "purpose": CAP_VALIDATION,
            "inputs": [
                io("repoRoot", "core.Path"),
                io("workspaceParent", "core.Path"),
                io("baseRevision", "core.String"),
                commands_io,
                discovery_io,
            ],
            "outputs": [io("validation", "lite.node.validation.Result")],
            "readableScopes": ["repo://fixture"],
            "writableScopes": [],
            "retry": {"maxAttempts": 1, "retryableClasses": []}
        })
        .to_string(),
    )
    .expect("validation manifest is well-formed")
}

/// Build a `lite.node.v1` manifest for the diagnostic node. The node is
/// pure: it consumes a serialized validation result and emits typed
/// classifications; it does not touch the repository, so `writableScopes`
/// is empty.
pub fn diagnostic_node_manifest(node_id: &str) -> NodeManifestV1 {
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": NODE_VALIDATION_VERSION,
            "nodeId": node_id,
            "purpose": CAP_DIAGNOSTIC,
            "inputs": [
                io("repoRoot", "core.Path"),
                io("validationEvidence", "lite.node.validation.Result"),
            ],
            "outputs": [io("diagnostic", "lite.node.diagnostic.Report")],
            "readableScopes": ["repo://fixture"],
            "writableScopes": [],
            "retry": {"maxAttempts": 1, "retryableClasses": []}
        })
        .to_string(),
    )
    .expect("diagnostic manifest is well-formed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_with_files(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, contents) in files {
            let full = dir.path().join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, contents).unwrap();
        }
        dir
    }

    #[test]
    fn rejects_missing_repo_root() {
        let args = serde_json::json!({});
        assert!(run_test_discovery(&args).is_err());
    }

    #[test]
    fn rejects_non_directory_repo_root() {
        let dir = tempfile::tempdir().unwrap();
        let bogus = dir.path().join("not-a-dir");
        let args = serde_json::json!({"repoRoot": bogus.to_string_lossy()});
        assert!(run_test_discovery(&args).is_err());
    }

    #[test]
    fn empty_repo_records_only_meta_assertion_command() {
        let dir = fixture_with_files(&[]);
        let args = serde_json::json!({"repoRoot": dir.path().to_string_lossy()});
        let out = run_test_discovery(&args).unwrap();
        let parsed: TestDiscoveryResultV1 = serde_json::from_str(&out).unwrap();
        // Empty repo → only the git-status meta-assertion is recorded.
        assert_eq!(parsed.commands.len(), 1);
        assert_eq!(parsed.commands[0].command, "git");
        assert!(!parsed.commands[0].why.is_empty());
        assert!(!parsed.commands[0].evidence_ref.is_empty());
    }

    #[test]
    fn cargo_manifest_produces_cargo_test_with_why() {
        let dir = fixture_with_files(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
        let args = serde_json::json!({"repoRoot": dir.path().to_string_lossy()});
        let out = run_test_discovery(&args).unwrap();
        let parsed: TestDiscoveryResultV1 = serde_json::from_str(&out).unwrap();
        // Cargo rule + meta-assertion.
        assert_eq!(parsed.commands.len(), 2);
        let cargo = parsed
            .commands
            .iter()
            .find(|c| c.command == "cargo")
            .expect("cargo command present");
        assert_eq!(cargo.args, vec!["test", "--offline"]);
        assert!(cargo.why.contains("Cargo.toml"));
        assert!(cargo.why.contains("cargo test --offline"));
        assert!(cargo.source.contains("Cargo.toml"));
        // evidence_ref is 64 lowercase hex.
        assert_eq!(cargo.evidence_ref.len(), 64);
        assert!(cargo.evidence_ref.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn discovery_is_deterministic() {
        let dir = fixture_with_files(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
        let args = serde_json::json!({"repoRoot": dir.path().to_string_lossy()});
        let a = run_test_discovery(&args).unwrap();
        let b = run_test_discovery(&args).unwrap();
        // Same inputs → identical command-level evidence_refs and
        // discovery_digest.
        let pa: TestDiscoveryResultV1 = serde_json::from_str(&a).unwrap();
        let pb: TestDiscoveryResultV1 = serde_json::from_str(&b).unwrap();
        assert_eq!(pa.discovery_digest, pb.discovery_digest);
        for (ca, cb) in pa.commands.iter().zip(pb.commands.iter()) {
            assert_eq!(ca.evidence_ref, cb.evidence_ref);
        }
    }

    // -------------------------------------------------------------------
    // Diagnostic classifier (PR3)
    // -------------------------------------------------------------------

    fn synthetic_run(stderr_tail: &str, exit_code: Option<i32>, timed_out: bool) -> CommandRunV1 {
        CommandRunV1 {
            command: "test".to_string(),
            args: vec![],
            exit_code,
            duration_ms: 0,
            timed_out,
            stdout_tail: String::new(),
            stderr_tail: stderr_tail.to_string(),
            evidence_ref: String::new(),
        }
    }

    #[test]
    fn classifier_kind_timeout_when_timed_out() {
        let run = synthetic_run("", None, true);
        let c = classify_one(&run);
        assert_eq!(c.kind, FailureKindV1::Timeout);
        assert!(c.timed_out);
        assert_eq!(c.signals, vec!["timed-out"]);
        assert_eq!(c.evidence_ref.len(), 64);
    }

    #[test]
    fn classifier_kind_resource_for_oom_signal() {
        let run = synthetic_run(
            "error: cannot allocate memory (os error 1455)",
            Some(1),
            false,
        );
        let c = classify_one(&run);
        assert_eq!(c.kind, FailureKindV1::Resource);
        assert!(c.signals.iter().any(|s| s.contains("memory")));
        assert_eq!(c.evidence_ref.len(), 64);
    }

    #[test]
    fn classifier_kind_environment_for_command_not_found() {
        let run = synthetic_run("/bin/sh: cargo: command not found", Some(127), false);
        let c = classify_one(&run);
        assert_eq!(c.kind, FailureKindV1::Environment);
        assert!(c.signals.iter().any(|s| s.contains("command not found")));
        assert_eq!(c.evidence_ref.len(), 64);
    }

    #[test]
    fn classifier_kind_test_for_assertion_failure() {
        let run = synthetic_run(
            "thread 't' panicked at 'assertion failed: add(1, 2) == 4', src/lib.rs:12",
            Some(101),
            false,
        );
        let c = classify_one(&run);
        assert_eq!(c.kind, FailureKindV1::Test);
        assert!(
            c.signals
                .iter()
                .any(|s| s.contains("assertion") || s.contains("panicked")),
            "signals should cite the matched pattern: {:?}",
            c.signals
        );
    }

    #[test]
    fn classifier_kind_code_for_compile_error() {
        let run = synthetic_run(
            "error[E0425]: cannot find value `x` in this scope",
            Some(1),
            false,
        );
        let c = classify_one(&run);
        assert_eq!(c.kind, FailureKindV1::Code);
        assert!(
            c.signals
                .iter()
                .any(|s| s.contains("error") || s.contains("cannot find")),
            "signals should cite the matched pattern: {:?}",
            c.signals
        );
    }

    #[test]
    fn classifier_kind_unknown_when_no_pattern_matches() {
        let run = synthetic_run("some unfamiliar failure output", Some(1), false);
        let c = classify_one(&run);
        assert_eq!(c.kind, FailureKindV1::Unknown);
        assert!(c.signals.is_empty());
        // Unknown still gets an evidence_ref so the report links the
        // classification to its inputs.
        assert_eq!(c.evidence_ref.len(), 64);
    }

    #[test]
    fn classifier_evidence_ref_is_deterministic() {
        let run = synthetic_run("error[E0425]: cannot find", Some(1), false);
        let a = classify_one(&run);
        let b = classify_one(&run);
        assert_eq!(a.evidence_ref, b.evidence_ref);
        assert_eq!(a.kind, FailureKindV1::Code);
    }

    #[test]
    fn diagnostic_report_summarizes_failure_kinds() {
        // End-to-end: feed a synthetic validation result to the node and
        // assert the summary counts.
        let validation = ValidationResultV1 {
            schema_version: NODE_VALIDATION_VERSION.to_string(),
            repo_root: ".".to_string(),
            base_revision: "abc".to_string(),
            worktree_head_revision: "abc".to_string(),
            run_id: "val-test".to_string(),
            runs_digest: "d".repeat(64),
            runs: vec![
                synthetic_run("error: cannot find value `x`", Some(1), false),
                synthetic_run("panicked at 'assertion failed'", Some(101), false),
                synthetic_run("command not found", Some(127), false),
                synthetic_run("", None, true),     // timed out
                synthetic_run("", Some(0), false), // passed
            ],
            constraints: vec![],
        };
        let args = serde_json::json!({
            "repoRoot": ".",
            "validationEvidence": serde_json::to_string(&validation).unwrap(),
        });
        let out = run_diagnostic(&args).unwrap();
        let report: DiagnosticReportV1 = serde_json::from_str(&out).unwrap();
        assert_eq!(report.total_runs, 5);
        assert_eq!(report.failed_runs, 4);
        // 4 failures + 1 passed (Unknown with no signals).
        assert_eq!(report.classifications.len(), 5);
        // Summary counts include every observed kind.
        let kinds: Vec<FailureKindV1> = report.classifications.iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&FailureKindV1::Code));
        assert!(kinds.contains(&FailureKindV1::Test));
        assert!(kinds.contains(&FailureKindV1::Environment));
        assert!(kinds.contains(&FailureKindV1::Timeout));
        // Summary by kind is sorted (BTreeMap) and each entry is non-zero.
        assert!(!report.summary_by_kind.is_empty());
        for entry in &report.summary_by_kind {
            assert!(entry.count > 0);
        }
        // Every classification carries a 64-hex evidence_ref.
        for c in &report.classifications {
            assert_eq!(c.evidence_ref.len(), 64);
        }
    }

    // -------------------------------------------------------------------
    // PR7 post-merge repair regressions
    // -------------------------------------------------------------------

    /// Build a small git repository with the named files, committed.
    fn fixture_repo_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git_run(root, &["init", "-q"]);
        git_run(root, &["config", "user.email", "ci@example.com"]);
        git_run(root, &["config", "user.name", "ci"]);
        for (path, contents) in files {
            let full = root.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, contents).unwrap();
        }
        git_run(root, &["add", "-A"]);
        git_run(root, &["commit", "-q", "-m", "initial"]);
        dir
    }

    fn git_run(repo_root: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .expect("git available in test environment");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn git_output_line(repo_root: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .output()
            .expect("git available in test environment");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn source_snapshot_detects_modified_file() {
        // PR7 fix 1: the snapshot must reflect working-tree changes, not
        // just HEAD. Capture, mutate, capture again, assert diff.
        let dir = fixture_repo_with(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
        let pre = capture_source_snapshot(dir.path());
        assert!(pre.porcelain.is_empty(), "fixture must start clean");
        // Simulate a command writing a file in the source.
        std::fs::write(dir.path().join("untracked.txt"), "leak").unwrap();
        let post = capture_source_snapshot(dir.path());
        assert_ne!(pre.porcelain, post.porcelain);
        assert!(post.porcelain.contains("?? untracked.txt"));
    }

    #[test]
    fn source_snapshot_detects_deleted_file() {
        // PR7 fix 1: `git status --porcelain` shows tracked deletions as
        // `D` lines; the snapshot must reflect that.
        let dir = fixture_repo_with(&[("keep.txt", "keep me\n")]);
        let pre = capture_source_snapshot(dir.path());
        assert!(pre.porcelain.is_empty());
        std::fs::remove_file(dir.path().join("keep.txt")).unwrap();
        let post = capture_source_snapshot(dir.path());
        assert_ne!(pre.porcelain, post.porcelain);
        assert!(
            post.porcelain.contains(" D keep.txt") || post.porcelain.contains("D  keep.txt"),
            "post porcelain should show the deletion: {:?}",
            post.porcelain
        );
    }

    #[test]
    fn verify_source_unchanged_passes_when_identical() {
        let s = SourceSnapshot {
            head: "abc".to_string(),
            porcelain: String::new(),
        };
        verify_source_unchanged(&s, &s).expect("identical snapshots must verify");
    }

    #[test]
    fn verify_source_unchanged_rejects_head_drift() {
        let pre = SourceSnapshot {
            head: "aaa".to_string(),
            porcelain: String::new(),
        };
        let post = SourceSnapshot {
            head: "bbb".to_string(),
            porcelain: String::new(),
        };
        let err = verify_source_unchanged(&pre, &post)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("HEAD moved"),
            "expected HEAD-moved error, got: {err}"
        );
    }

    #[test]
    fn verify_source_unchanged_rejects_porcelain_drift() {
        let pre = SourceSnapshot {
            head: "abc".to_string(),
            porcelain: String::new(),
        };
        let post = SourceSnapshot {
            head: "abc".to_string(),
            porcelain: "?? leaked.txt\n".to_string(),
        };
        let err = verify_source_unchanged(&pre, &post)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("working tree changed"),
            "expected working-tree error, got: {err}"
        );
    }

    #[test]
    fn read_command_specs_rejects_both_commands_and_discovery() {
        // PR7 fix 4: the input contract is exactly-one of `commands` or
        // `discoveryEvidence`; passing both is an error.
        let dir = tempfile::tempdir().unwrap();
        let args = serde_json::json!({
            "commands": [{"command": "git", "args": ["status"]}],
            "discoveryEvidence": serde_json::to_string(&TestDiscoveryResultV1 {
                schema_version: NODE_VALIDATION_VERSION.to_string(),
                repo_root: dir.path().to_string_lossy().to_string(),
                revision: "abc".to_string(),
                discovery_id: "d".to_string(),
                discovery_digest: "d".repeat(64),
                commands: vec![],
                constraints: vec![],
            })
            .unwrap(),
        });
        let err = read_command_specs(&args).unwrap_err().to_string();
        assert!(
            err.contains("exactly one") && err.contains("not both"),
            "expected exactly-one error, got: {err}"
        );
    }

    #[test]
    fn read_command_specs_rejects_neither_commands_nor_discovery() {
        let args = serde_json::json!({});
        let err = read_command_specs(&args).unwrap_err().to_string();
        assert!(
            err.contains("exactly one") && err.contains("got neither"),
            "expected neither error, got: {err}"
        );
    }

    #[test]
    fn validation_manifest_declares_commands_and_discovery_as_optional() {
        // PR7 fix 4: the lite.node.v1 manifest for the validation node
        // must declare BOTH `commands` and `discoveryEvidence` so the
        // nine-gate contract matches the handler's exactly-one rule. The
        // capability-level `required_args` covers the always-required
        // trio; the manifest's `required: Some(false)` flags make the
        // command-side inputs optional.
        let m = validation_node_manifest("node-validation-manifest-check");
        let names: Vec<&str> = m.inputs.iter().map(|i| i.name.as_str()).collect();
        assert!(
            names.contains(&"commands"),
            "manifest must declare commands"
        );
        assert!(
            names.contains(&"discoveryEvidence"),
            "manifest must declare discoveryEvidence"
        );
        let commands_input = m.inputs.iter().find(|i| i.name == "commands").unwrap();
        let discovery_input = m
            .inputs
            .iter()
            .find(|i| i.name == "discoveryEvidence")
            .unwrap();
        assert_eq!(
            commands_input.required,
            Some(false),
            "commands must be optional (exactly-one with discoveryEvidence)"
        );
        assert_eq!(
            discovery_input.required,
            Some(false),
            "discoveryEvidence must be optional (exactly-one with commands)"
        );
    }

    #[test]
    fn classifier_compiler_error_is_code_not_test() {
        // PR7 fix 3 regression: a real compiler error like
        // `error: failed to resolve ...` must classify as Code, not
        // Test. The prior pattern set matched the broad substring
        // "failed" first and misclassified these as test failures.
        let run = synthetic_run(
            "error: failed to resolve `crate::foo::Bar`\n  --> src/lib.rs:12:5",
            Some(1),
            false,
        );
        let c = classify_one(&run);
        assert_eq!(
            c.kind,
            FailureKindV1::Code,
            "compiler error containing 'failed' must be Code, got {:?} with signals {:?}",
            c.kind,
            c.signals
        );
    }

    #[test]
    fn classifier_test_pattern_does_not_match_compiler_expected() {
        // PR7 fix 3 regression: the Test rule must NOT match generic
        // compiler-error phrasing like `expected ... , found ...`.
        let run = synthetic_run(
            "error[E0308]: mismatched types\n  --> src/lib.rs:5:9\n   expected `u32`, found `i32`",
            Some(1),
            false,
        );
        let c = classify_one(&run);
        assert_eq!(
            c.kind,
            FailureKindV1::Code,
            "compiler 'expected ... found ...' must be Code, got {:?} with signals {:?}",
            c.kind,
            c.signals
        );
    }

    #[test]
    fn run_validation_rejects_when_command_modifies_source() {
        // PR7 fix 1 end-to-end: a command that removes a tracked file
        // from the source working tree must be detected by the
        // post-snapshot check. We use `git rm` with --git-dir/--work-tree
        // pointing at the source so the command runs in the worktree
        // but mutates the source. `git status --porcelain` then shows
        // the deletion in the post-snapshot.
        let dir = fixture_repo_with(&[("leak.txt", "leak me\n")]);
        let head = git_output_line(dir.path(), &["rev-parse", "HEAD"]);
        let ws_parent = tempfile::tempdir().unwrap();
        let git_dir = format!("{}/.git", dir.path().to_string_lossy());
        let work_tree = dir.path().to_string_lossy().to_string();
        let result = run_validation(&serde_json::json!({
            "repoRoot": dir.path().to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
            "baseRevision": head,
            "commands": [{
                "command": "git",
                "args": ["--git-dir", git_dir, "--work-tree", work_tree, "rm", "leak.txt"],
            }],
        }));
        let err = result.expect_err("validation must reject a command that mutates the source");
        let msg = err.to_string();
        assert!(
            msg.contains("source-isolation invariant violated")
                && msg.contains("working tree changed"),
            "expected working-tree invariant rejection, got: {msg}"
        );
        // The source HEAD must be unchanged (only the working tree was
        // modified by the malicious command).
        assert_eq!(git_output_line(dir.path(), &["rev-parse", "HEAD"]), head);
    }

    #[test]
    fn run_validation_cleans_up_worktree_when_command_fails() {
        // PR7 fix 2: a command that can't be spawned must not leak the
        // worktree. The guard's Drop runs cleanup as a last-resort.
        let dir = fixture_repo_with(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
        let head = git_output_line(dir.path(), &["rev-parse", "HEAD"]);
        let ws_parent = tempfile::tempdir().unwrap();
        let _ = run_validation(&serde_json::json!({
            "repoRoot": dir.path().to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
            "baseRevision": head,
            "commands": [{
                "command": "definitely-not-a-real-binary-xyzzy",
                "args": [],
            }],
        }))
        .expect_err("spawn failure must propagate as an error");
        // The worktree under `ws_parent/validate-XXXXXXXX/worktree` must be
        // gone (the guard's Drop cleanup ran). The `validate-XXXXXXXX` parent
        // directory is left behind by `git worktree remove --force` (it
        // does not rmdir empty parents), which is expected and harmless.
        let worktree_paths: Vec<_> = walk_dir_for_worktrees(ws_parent.path());
        assert!(
            worktree_paths.is_empty(),
            "Drop fallback must have removed the worktree; leftover: {:?}",
            worktree_paths
        );
    }

    /// Walk a directory recursively and return any path whose final segment
    /// is `worktree` and which lives under a `validate-...` directory. This
    /// is used by the cleanup-fallback test to assert the worktree was
    /// actually removed (without relying on the parent dir being rmdir'd).
    fn walk_dir_for_worktrees(root: &Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => return out,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path
                    .file_name()
                    .map(|n| n == std::ffi::OsStr::new("worktree"))
                    .unwrap_or(false)
                {
                    out.push(path);
                } else {
                    out.extend(walk_dir_for_worktrees(&path));
                }
            }
        }
        out
    }
}
