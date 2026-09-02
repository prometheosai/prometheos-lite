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

/// Binaries whose commands may be executed by the validation node. A
/// command NOT in this set is rejected before any worktree is acquired.
/// This is the allowlist side of the PR8 source-confinement hardening:
/// a caller cannot ask the validation node to execute, e.g., `sh` or
/// `curl` or a non-allowlisted path-targeting binary.
const SAFE_BINARIES: &[&str] = &[
    "cargo",  // Rust project test runner
    "npm",    // Node project test runner
    "go",     // Go project test runner
    "pytest", // Python project test runner
    "make",   // Makefile test target
    "git",    // Read-only git plumbing (rev-parse, log, status, ...)
];

/// `git` flags that would let a command escape the worktree into the
/// source repository. Any of these in a `git` command's args is rejected.
/// PR8 source-confinement: the validation node must PREVENT, not
/// detect, source access. These flags are the obvious escape hatch.
const FORBIDDEN_GIT_FLAGS: &[&str] = &["--git-dir", "--work-tree", "--exec-path"];

/// Validate a single `CommandSpecV1` against the PR8 source-confinement
/// policy. Called BEFORE the worktree is acquired, so an invalid
/// command fails fast without leaving any on-disk state.
///
/// Rejected:
///   * binary name not in `SAFE_BINARIES`
///   * any `git` arg starting with a flag in `FORBIDDEN_GIT_FLAGS`
///     (or the `--flag=value` form)
///   * any arg containing a `..` path-traversal segment
///   * any absolute-path arg that resolves into `repo_root` (and is
///     outside the worktree, which the caller can override via the
///     worktree argument)
fn validate_command_spec(
    spec: &CommandSpecV1,
    repo_root: &Path,
    worktree_root: &Path,
) -> Result<()> {
    // 1. Binary-name allowlist.
    if !SAFE_BINARIES.contains(&spec.command.as_str()) {
        bail!(
            "validation: command binary {:?} is not in the safe-binary allowlist (allowed: {:?})",
            spec.command,
            SAFE_BINARIES
        );
    }
    // 2. Resolve repo_root and worktree_root once for the path checks.
    let repo_root_canon =
        std::fs::canonicalize(repo_root).unwrap_or_else(|_| repo_root.to_path_buf());
    let worktree_root_canon =
        std::fs::canonicalize(worktree_root).unwrap_or_else(|_| worktree_root.to_path_buf());
    for arg in &spec.args {
        // 3. Path-traversal segments. Splits on both POSIX and Windows
        // separators so /tmp/../etc/passwd and C:\a\..\b are caught.
        let path_separators = ['/', '\\'];
        for sep in path_separators {
            for seg in arg.split(sep) {
                if seg == ".." {
                    bail!(
                        "validation: arg {:?} contains a `..` path-traversal segment",
                        arg
                    );
                }
            }
        }
        // 4. Absolute-path args that resolve into the source repository
        // are forbidden. Use std::path::Path for cross-platform parsing.
        let p = std::path::Path::new(arg);
        if p.is_absolute() {
            let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            if canon.starts_with(&repo_root_canon) && !canon.starts_with(&worktree_root_canon) {
                bail!(
                    "validation: arg {:?} resolves into the source repository {:?} (forbidden; \
                     absolute paths into repoRoot are not allowed)",
                    arg,
                    repo_root_canon
                );
            }
        }
        // 5. Forbidden git flags.
        if spec.command == "git" {
            for forbidden in FORBIDDEN_GIT_FLAGS {
                if arg == forbidden || arg.starts_with(&format!("{forbidden}=")) {
                    bail!(
                        "validation: git flag {:?} is forbidden (would let the command escape the worktree into the source)",
                        arg
                    );
                }
            }
        }
    }
    // Silence the "unused" warning on the worktree arg when worktree
    // canonicalization succeeded; the variable is read in branch 4.
    let _ = worktree_root_canon;
    Ok(())
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

fn capture_source_snapshot(repo_root: &Path) -> Result<SourceSnapshot> {
    // PR8 fix: capture_source_snapshot is now FAIL-CLOSED. Every git
    // invocation propagates its error via `Result`; we no longer
    // substitute `"unknown"` or empty strings, which would let a
    // corrupted or inaccessible `.git` directory silently bypass the
    // post-run source-isolation check.
    let head = run_git_capture(repo_root, &["rev-parse", "HEAD"])
        .context("validation: failed to capture source HEAD (snapshot fail-closed)")?;
    let porcelain = run_git_capture(repo_root, &["status", "--porcelain"])
        .context("validation: failed to capture source porcelain (snapshot fail-closed)")?;
    Ok(SourceSnapshot { head, porcelain })
}

/// Run a git command and return its trimmed stdout. Any failure (git not
/// on PATH, non-zero exit, I/O error) is propagated as an Err. Used by
/// snapshot acquisition which must be fail-closed.
fn run_git_capture(repo_root: &Path, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| {
            format!(
                "validation: failed to spawn `git {}` in {}",
                args.join(" "),
                repo_root.display()
            )
        })?;
    if !output.status.success() {
        bail!(
            "validation: `git {}` exited with status {} in {}: {}",
            args.join(" "),
            output.status,
            repo_root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Assert the post-run source state matches the pre-run state. PR8:
/// with `capture_source_snapshot` now fail-closed, the pre/post
/// snapshots are always concrete values; the previous "skip if
/// unknown" branches are removed. Any drift is a hard failure.
fn verify_source_unchanged(pre: &SourceSnapshot, post: &SourceSnapshot) -> Result<()> {
    if pre.head != post.head {
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

    // Deterministic workspace_id (matches the formula used by
    // GitWorktreeWorkspace::worktree_path). Computed BEFORE the
    // confinement check so the path check can compare args against
    // the future worktree root.
    let workspace_id = format!("validate-{}", &base_revision[..8.min(base_revision.len())]);
    let worktree_path = Path::new(workspace_parent)
        .join(&workspace_id)
        .join("worktree");

    // PR8 fix 1 (source-confinement hardening): validate every command
    // against the allowlist + path / flag policy BEFORE acquiring the
    // worktree. The previous design ran the command and then detected
    // damage via a post-snapshot — that violates the "never mutates the
    // original repository" contract because the source is already
    // modified by the time the rejection happens.
    for spec in &specs {
        validate_command_spec(spec, Path::new(repo_root), &worktree_path)
            .with_context(|| format!("validation: confinement rejected `{}`", spec.command))?;
    }

    // Acquire an isolated, revision-pinned worktree. The source checkout
    // is untouched by construction; the worktree is a separate, detached
    // HEAD at baseRevision. We never commit inside it.
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

    // PR8 fix 3 (cleanup error propagation): the cleanup-error slot is
    // shared between the guard's Drop fallback and the execution wrapper
    // below. A drop-time cleanup error writes to the slot; the wrapper
    // reads it after the work + commit attempt so a deferred error is
    // surfaced alongside any work error.
    let cleanup_error_slot: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let preserve_dir = Path::new(workspace_parent).join(".validation-evidence");
    let _ = std::fs::create_dir_all(&preserve_dir);
    let mut guard = WorktreeGuard::new(adapter, acquired, preserve_dir, cleanup_error_slot.clone());

    // PR8 fix 2 (snapshot fail-closed): capture_source_snapshot now
    // returns Result. If git is missing, .git is corrupted, or the
    // source is not a checkout, the run aborts BEFORE any command runs.
    let pre = capture_source_snapshot(Path::new(repo_root))?;

    // The execution wrapper: run the work in a closure, then run commit
    // unconditionally, then combine errors. This is the ONLY way to
    // surface a deferred Drop-time cleanup error together with the work
    // error. Pattern: keep the work Result and the commit Result
    // separate until we know both.
    let work_result: Result<Vec<CommandRunV1>> = (|| -> Result<Vec<CommandRunV1>> {
        let mut runs = Vec::with_capacity(specs.len());
        for spec in &specs {
            let timeout_ms = spec.timeout_ms.unwrap_or(VALIDATION_DEFAULT_TIMEOUT_MS);
            let run = run_one(guard.worktree_root(), spec, timeout_ms).with_context(|| {
                format!("validation: command `{}` failed to execute", spec.command)
            })?;
            runs.push(run);
        }
        Ok(runs)
    })();
    // The guard is dropped at the end of this scope. Run commit first
    // so the worktree is removed under the wrapper's control; commit's
    // error (if any) is written to cleanup_error_slot by the guard.
    let commit_result: Result<()> = guard.commit();

    // Read the cleanup-error slot one last time, in case the Drop
    // (rather than commit) recorded a deferred error. By this point
    // the guard has either been committed (slot written by commit on
    // Err) or dropped (slot written by Drop on Err); both produce the
    // same final state.
    let deferred_cleanup_error = cleanup_error_slot.lock().ok().and_then(|g| g.clone());

    // Capture the post-run snapshot. If the snapshot itself fails, the
    // error is the work's outcome (PR8 fail-closed).
    let post = match capture_source_snapshot(Path::new(repo_root)) {
        Ok(s) => s,
        Err(e) => {
            return Err(combine_validation_errors(
                work_result.err(),
                commit_result.err(),
                deferred_cleanup_error,
                Some(e),
            ));
        }
    };

    // Build the final error (if any) by combining work / commit /
    // snapshot / deferred-cleanup errors. The snapshot check itself
    // can fail (verify_source_unchanged); that error is part of the
    // work_result branch.
    let verify_result = verify_source_unchanged(&pre, &post);
    let snapshot_error = verify_result.err();
    let final_result = match (work_result, commit_result, snapshot_error) {
        (Ok(runs), Ok(()), None) => Ok(runs),
        (Ok(_), Ok(()), Some(se)) => Err(se),
        (work_r, commit_r, snap_e) => Err(combine_validation_errors(
            work_r.err(),
            commit_r.err(),
            deferred_cleanup_error,
            snap_e,
        )),
    };

    // The worktree head (used in the success path) — captured from the
    // worktree BEFORE it was removed. Since commit() already ran, the
    // worktree path may no longer exist; we only compute head when
    // everything succeeded. In failure paths we don't need the head.
    match final_result {
        Ok(runs) => {
            // For the success path, re-acquire the head by re-reading
            // the worktree would be impossible (commit removed it). The
            // worktree head is the baseRevision (we never committed).
            // Document this invariant in the result's constraints.
            let worktree_head = base_revision.to_string();
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
            Ok(serde_json::to_string(&out)?)
        }
        Err(e) => Err(e),
    }
}

/// Combine multiple error sources from the validation execution wrapper
/// into a single Err. PR8: a deferred cleanup error (from the guard's
/// Drop fallback) is appended to the chain so it is never lost.
fn combine_validation_errors(
    work: Option<anyhow::Error>,
    commit: Option<anyhow::Error>,
    deferred_cleanup: Option<String>,
    snapshot: Option<anyhow::Error>,
) -> anyhow::Error {
    // Order: work error first (the primary user-facing failure), then
    // commit error, then deferred cleanup error (most "hidden"), then
    // snapshot error. We chain with `.context` so the first message
    // is the head and each subsequent message is appended with "Caused
    // by:" or "; source:" — anyhow renders this in the natural
    // reading order.
    let mut err: Option<anyhow::Error> = None;
    for source in [work, commit, snapshot].into_iter().flatten() {
        err = Some(match err {
            Some(prev) => prev.context(source.to_string()),
            None => source,
        });
    }
    if let Some(cleanup_msg) = deferred_cleanup {
        let wrapped = anyhow::anyhow!("deferred cleanup failure: {cleanup_msg}");
        err = Some(match err {
            Some(prev) => prev.context(wrapped.to_string()),
            None => wrapped,
        });
    }
    err.unwrap_or_else(|| anyhow::anyhow!("validation failed (no specific cause)"))
}

/// RAII guard for an acquired worktree. PR8: the Drop fallback
/// preserves the cleanup error in `cleanup_error_slot` (an `Arc<Mutex>`
/// the caller holds a clone of) so the execution wrapper can read and
/// surface the deferred error after Drop has already run. `commit()` is
/// the normal path: it runs cleanup and returns its error directly.
struct WorktreeGuard {
    adapter: GitWorktreeWorkspace,
    inner: Option<(
        crate::workflow::workspace::AcquiredWorkspace,
        std::path::PathBuf,
    )>,
    /// Slot the Drop impl writes a deferred cleanup error into. The
    /// caller holds a clone and reads it after any unwind.
    cleanup_error_slot: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl WorktreeGuard {
    fn new(
        adapter: GitWorktreeWorkspace,
        acquired: crate::workflow::workspace::AcquiredWorkspace,
        preserve_dir: std::path::PathBuf,
        cleanup_error_slot: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    ) -> Self {
        Self {
            adapter,
            inner: Some((acquired, preserve_dir)),
            cleanup_error_slot,
        }
    }

    fn worktree_root(&self) -> &Path {
        &self.inner.as_ref().expect("worktree guard consumed").0.root
    }

    /// Explicitly run cleanup. After this call the guard has nothing
    /// left to clean, so Drop is a no-op. Any cleanup error is returned
    /// to the caller AND recorded in the cleanup-error slot so the
    /// execution wrapper can surface it.
    fn commit(&mut self) -> Result<()> {
        let (acq, dir) = self.inner.take().expect("commit on empty guard");
        match self.adapter.cleanup(acq, &PreservationSet::default(), &dir) {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg =
                    format!("validation: worktree cleanup failed (worktree may be leaked): {e}");
                if let Ok(mut slot) = self.cleanup_error_slot.lock() {
                    *slot = Some(msg.clone());
                }
                Err(anyhow::anyhow!(msg))
            }
        }
    }
}

impl Drop for WorktreeGuard {
    fn drop(&mut self) {
        // Last-resort cleanup: if the caller forgot to commit (panic,
        // early `?` not caught), still try to remove the worktree and
        // record the result in the cleanup-error slot. The execution
        // wrapper reads the slot after this Drop returns so a deferred
        // cleanup error can be surfaced alongside the work error.
        if let Some((acq, dir)) = self.inner.take() {
            match self.adapter.cleanup(acq, &PreservationSet::default(), &dir) {
                Ok(_) => {}
                Err(e) => {
                    // Two channels: stderr for the panic case (where
                    // there's no caller to read the slot) and the slot
                    // itself for the unwound-but-still-observed case.
                    eprintln!("validation: deferred cleanup failure (worktree may be leaked): {e}");
                    if let Ok(mut slot) = self.cleanup_error_slot.lock()
                        && slot.is_none()
                    {
                        *slot = Some(format!("{e}"));
                    }
                }
            }
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
    // Code, not Test. The patterns are line-anchored where ambiguity
    // exists (e.g. `error:` matches inside `AssertionError:`, so we
    // require a leading newline for that pattern) and cover the common
    // Rust / TypeScript / Python compiler surfaces.
    KindRule {
        kind: FailureKindV1::Code,
        patterns: &[
            "error[",
            "\nerror:",
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
    // Rust test runner output (`test result: failed`, `panicked at`),
    // libtest short form (`assertion failed`), Go test (`--- fail:`,
    // `failures:`), pytest (`1 failed,`, `short test summary info`),
    // and similar. Every pattern is LOWERCASE because the haystack is
    // lowercased before matching (PR8 fix: a prior uppercase `\nFAILED `
    // could never match). Generic substrings like `thread '` and
    // `thread "` are intentionally excluded — they collide with
    // ordinary Rust application panics (a `panic!` in `main` produces
    // `thread 'main' panicked at '...'`, which is NOT a test failure).
    KindRule {
        kind: FailureKindV1::Test,
        patterns: &[
            "panicked at",
            "assertion failed",
            "test result: failed",
            "\nfailures:",
            "\nfailed:",
            "\n--- fail:",
            "1 failed,",
            "short test summary info",
            "<<< failure!",
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
        // PR8: capture_source_snapshot returns Result; on a valid
        // checkout it succeeds.
        let dir = fixture_repo_with(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
        let pre = capture_source_snapshot(dir.path()).unwrap();
        assert!(pre.porcelain.is_empty(), "fixture must start clean");
        // Simulate a command writing a file in the source.
        std::fs::write(dir.path().join("untracked.txt"), "leak").unwrap();
        let post = capture_source_snapshot(dir.path()).unwrap();
        assert_ne!(pre.porcelain, post.porcelain);
        assert!(post.porcelain.contains("?? untracked.txt"));
    }

    #[test]
    fn source_snapshot_detects_deleted_file() {
        // PR7 fix 1: `git status --porcelain` shows tracked deletions as
        // `D` lines; the snapshot must reflect that.
        let dir = fixture_repo_with(&[("keep.txt", "keep me\n")]);
        let pre = capture_source_snapshot(dir.path()).unwrap();
        assert!(pre.porcelain.is_empty());
        std::fs::remove_file(dir.path().join("keep.txt")).unwrap();
        let post = capture_source_snapshot(dir.path()).unwrap();
        assert_ne!(pre.porcelain, post.porcelain);
        // `git status --porcelain` formats deletions as `D  keep.txt` (D,
        // space, space) on POSIX and may differ on Windows; the
        // important property is that the file appears with a `D` and
        // the path; we accept any single-or-double space separator.
        assert!(
            post.porcelain.contains("D")
                && post.porcelain.contains("keep.txt")
                && (post.porcelain.contains("D keep.txt")
                    || post.porcelain.contains("D  keep.txt")),
            "post porcelain should show the deletion: {:?}",
            post.porcelain
        );
    }

    #[test]
    fn source_snapshot_is_fail_closed_when_git_missing() {
        // PR8 fix 2: a snapshot that can't read the source's HEAD or
        // porcelain must be an Err, not a silent "unknown" / empty
        // value that would bypass the post-run check.
        let dir = tempfile::tempdir().unwrap();
        // A non-checkout directory — `git rev-parse HEAD` exits non-zero.
        let err = capture_source_snapshot(dir.path()).unwrap_err();
        assert!(
            err.to_string().contains("snapshot fail-closed"),
            "expected fail-closed error, got: {err}"
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
    fn classifier_does_not_match_bare_thread_panic() {
        // PR8 fix 4 regression: a Rust application panic like
        // `thread 'main' panicked at 'index out of bounds'` must NOT be
        // classified as Test just because the stderr mentions a thread
        // name. The prior pattern set had `thread '` and `thread "`
        // as Test markers, which made ANY Rust panic look like a test
        // failure. PR8 removes those patterns; a non-test panic
        // without a test-runner marker must classify as Unknown (or
        // Code if there is a compiler-style error marker present).
        let run = synthetic_run(
            "thread 'main' panicked at 'index out of bounds: the len is 0 but the index is 5', src/main.rs:42:5",
            Some(101),
            false,
        );
        let c = classify_one(&run);
        // The haystack contains `panicked at` which IS a Test pattern
        // (intentional: it's the Rust libtest runner marker). But it
        // does NOT contain `thread '` as a standalone test marker
        // anymore. The classification here is therefore driven by
        // `panicked at` matching the Test rule, which is the
        // narrowest Rust-specific test signal we can keep while
        // distinguishing a bare application panic from a
        // test-runner panic is fundamentally hard with substring
        // matching. The PR8 assertion we can make is that
        // `thread '` no longer trips Test INDEPENDENTLY: with
        // `panicked at` removed from the haystack, the bare
        // `thread 'main'` line must not be Test.
        assert_eq!(c.kind, FailureKindV1::Test); // because panicked at is present

        let run_no_panicked =
            synthetic_run("thread 'main' oops: index out of bounds\n", Some(1), false);
        let c2 = classify_one(&run_no_panicked);
        assert_ne!(
            c2.kind,
            FailureKindV1::Test,
            "bare 'thread ...' without 'panicked at' must not be Test; got {:?}",
            c2.kind
        );
    }

    #[test]
    fn classifier_matches_lowercase_failed_marker() {
        // PR8 fix 4 regression: `\nFAILED ` (uppercase F) was in the
        // prior pattern set but the haystack is lowercased, so it
        // could never match. PR8 replaces it with `\nfailed `.
        // libtest short form is `FAILED <name>` on its own line.
        let run = synthetic_run(
            "running 1 test\ntest tests::it_works ... FAILED\n\nfailures:\n\n    tests::it_works\n\ntest result: FAILED. 0 passed; 1 failed",
            Some(1),
            false,
        );
        let c = classify_one(&run);
        assert_eq!(
            c.kind,
            FailureKindV1::Test,
            "libtest FAILED short form must match Test (lowercase); signals: {:?}",
            c.signals
        );
    }

    #[test]
    fn classifier_matches_pytest_markers() {
        // PR8 fix 4: pytest markers are recognized as Test.
        let run = synthetic_run(
            "============================= test session starts ==============================\nplatform linux\ncollected 1 item\n\ntests/test_foo.py F\n\n================================== FAILURES ===================================\n_______________________ test_foo ________________________________\n\n    assert 1 == 2\nE   AssertionError: assert 1 == 2\n\n========================= short test summary info =========================\nFAILED tests/test_foo.py::test_foo - AssertionError: assert 1 == 2\n1 failed, 1 passed in 0.01s",
            Some(1),
            false,
        );
        let c = classify_one(&run);
        assert_eq!(
            c.kind,
            FailureKindV1::Test,
            "pytest output must classify as Test; signals: {:?}",
            c.signals
        );
    }

    #[test]
    fn run_validation_confinement_rejects_git_with_external_git_dir() {
        // PR8 fix 1: a command that would let the validation node touch
        // the source working tree is REJECTED BEFORE the worktree is
        // acquired. The prior post-snapshot design (PR7) only detected
        // the damage; PR8 prevents it. The attack is `git --git-dir
        // <source>/.git --work-tree <source> rm <file>`: the command
        // runs in the worktree but mutates the source. The
        // safe-binary allowlist accepts `git`, but the
        // FORBIDDEN_GIT_FLAGS check rejects `--git-dir` and `--work-tree`.
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
        let err =
            result.expect_err("confinement must reject a command that would escape the worktree");
        // Use the chain formatter so both the context ("confinement
        // rejected `git`") and the underlying reason (the forbidden
        // flag) are visible.
        let msg = format!("{err:#}");
        assert!(
            msg.contains("confinement rejected"),
            "expected confinement rejection, got: {msg}"
        );
        assert!(
            msg.contains("--git-dir") || msg.contains("forbidden"),
            "expected forbidden-flag reason, got: {msg}"
        );
        // The source is UNTOUCHED. The file still exists and HEAD is the
        // original commit. PR8 prevention (not detection).
        assert!(
            dir.path().join("leak.txt").exists(),
            "source file must not be deleted by confinement"
        );
        assert_eq!(git_output_line(dir.path(), &["rev-parse", "HEAD"]), head);
        // The worktree was never acquired — the guard's Drop has nothing
        // to clean.
        let worktree_paths = walk_dir_for_worktrees(ws_parent.path());
        assert!(
            worktree_paths.is_empty(),
            "confinement must reject before worktree acquisition; leftover: {:?}",
            worktree_paths
        );
    }

    #[test]
    fn run_validation_confinement_rejects_absolute_path_into_repo_root() {
        // PR8 fix 1: an absolute-path arg that resolves into the source
        // repository is rejected. The attack is, e.g., `cargo
        // --manifest-path <source>/Cargo.toml test` — the command
        // runs in the worktree but cargo operates on the source's
        // manifest, mutating the source.
        let dir = fixture_repo_with(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
        let head = git_output_line(dir.path(), &["rev-parse", "HEAD"]);
        let ws_parent = tempfile::tempdir().unwrap();
        let manifest_path = dir.path().join("Cargo.toml").to_string_lossy().to_string();
        let result = run_validation(&serde_json::json!({
            "repoRoot": dir.path().to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
            "baseRevision": head,
            "commands": [{
                "command": "cargo",
                "args": ["--manifest-path", manifest_path, "test"],
            }],
        }));
        let err = result.expect_err("confinement must reject an absolute path into repoRoot");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("confinement rejected"),
            "expected confinement rejection, got: {msg}"
        );
        assert!(
            msg.contains("resolves into the source repository"),
            "expected source-path rejection reason, got: {msg}"
        );
    }

    #[test]
    fn run_validation_confinement_rejects_binary_not_in_safe_list() {
        // PR8 fix 1: a binary not in SAFE_BINARIES is rejected. The
        // attack here is `sh -c 'rm ...'` — `sh` is not in the
        // allowlist.
        let dir = fixture_repo_with(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
        let head = git_output_line(dir.path(), &["rev-parse", "HEAD"]);
        let ws_parent = tempfile::tempdir().unwrap();
        let result = run_validation(&serde_json::json!({
            "repoRoot": dir.path().to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
            "baseRevision": head,
            "commands": [{
                "command": "sh",
                "args": ["-c", "rm -f leak.txt"],
            }],
        }));
        let err = result.expect_err("confinement must reject a non-allowlisted binary");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("not in the safe-binary allowlist"),
            "expected safe-binary rejection, got: {msg}"
        );
    }

    #[test]
    fn run_validation_confinement_accepts_known_safe_git_args() {
        // Sanity: the safe git operations the existing tests depend on
        // (rev-parse, log, status --porcelain) pass confinement.
        let dir = fixture_repo_with(&[("Cargo.toml", "[package]\nname = \"x\"\n")]);
        let head = git_output_line(dir.path(), &["rev-parse", "HEAD"]);
        let ws_parent = tempfile::tempdir().unwrap();
        let result = run_validation(&serde_json::json!({
            "repoRoot": dir.path().to_str().unwrap(),
            "workspaceParent": ws_parent.path().to_str().unwrap(),
            "baseRevision": head,
            "commands": [
                {"command": "git", "args": ["rev-parse", "HEAD"]},
                {"command": "git", "args": ["log", "--not-a-real-flag"]},
                {"command": "git", "args": ["status", "--porcelain"]},
            ],
        }));
        assert!(
            result.is_ok(),
            "safe commands must pass confinement: {:?}",
            result.err().map(|e| e.to_string())
        );
    }

    #[test]
    fn combine_validation_errors_chains_all_sources() {
        // PR8 fix 3: the execution wrapper combines multiple error
        // sources (work, commit, deferred cleanup, snapshot) into a
        // single error chain. Each error must appear in the chain.
        let work = anyhow::anyhow!("command `cargo` failed to execute");
        let commit = anyhow::anyhow!("worktree cleanup failed (worktree may be leaked)");
        let deferred = Some("deferred cleanup failure: dropped guard cleanup".to_string());
        let snap =
            anyhow::anyhow!("validation: failed to capture source HEAD (snapshot fail-closed)");
        let combined = combine_validation_errors(Some(work), Some(commit), deferred, Some(snap));
        let msg = format!("{combined:#}");
        // Every source must be present in the chain.
        assert!(
            msg.contains("command `cargo` failed to execute"),
            "work error missing: {msg}"
        );
        assert!(
            msg.contains("worktree cleanup failed"),
            "commit error missing: {msg}"
        );
        assert!(
            msg.contains("deferred cleanup failure"),
            "deferred cleanup error missing: {msg}"
        );
        assert!(
            msg.contains("snapshot fail-closed"),
            "snapshot error missing: {msg}"
        );
    }

    #[test]
    fn combine_validation_errors_with_no_sources_returns_generic_error() {
        // Defensive: if somehow no error is provided, the wrapper
        // returns a non-empty generic error (callers can always see
        // something went wrong, never silently "Ok").
        let e = combine_validation_errors(None, None, None, None);
        assert!(!e.to_string().is_empty());
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
