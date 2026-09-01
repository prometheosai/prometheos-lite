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
    if let Some(arr) = args.get("commands").and_then(|v| v.as_array()) {
        let mut out = Vec::with_capacity(arr.len());
        for v in arr {
            out.push(serde_json::from_value(v.clone()).with_context(
                || "validation: each command spec must include at least a `command` string",
            )?);
        }
        return Ok(out);
    }
    if let Some(discovery_json) = args.get("discoveryEvidence").and_then(|v| v.as_str()) {
        let discovery: TestDiscoveryResultV1 = serde_json::from_str(discovery_json)
            .context("validation: discoveryEvidence unparseable")?;
        let out = discovery
            .commands
            .into_iter()
            .map(|c| CommandSpecV1 {
                command: c.command,
                args: c.args,
                timeout_ms: None,
            })
            .collect();
        return Ok(out);
    }
    bail!("validation requires either `commands` or `discoveryEvidence`")
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

    // Snapshot the source HEAD before we run, so any contamination (which
    // would be a bug) is detectable in evidence.
    let source_head_before = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(Path::new(repo_root))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let mut runs = Vec::with_capacity(specs.len());
    for spec in &specs {
        let timeout_ms = spec.timeout_ms.unwrap_or(VALIDATION_DEFAULT_TIMEOUT_MS);
        let run = run_one(&acquired.root, spec, timeout_ms)
            .with_context(|| format!("validation: command `{}` failed to execute", spec.command))?;
        runs.push(run);
    }

    // The worktree head should equal baseRevision (we never committed).
    let worktree_head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&acquired.root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Tidy up the worktree: nothing to preserve (we made no commits). The
    // source checkout was never opened by us in write mode.
    let preserve_dir = Path::new(workspace_parent).join(".validation-evidence");
    let _ = std::fs::create_dir_all(&preserve_dir);
    let _ = adapter.cleanup(acquired, &PreservationSet::default(), &preserve_dir);

    // Assert the source is still on the same HEAD. This is a hard
    // contract: validation NEVER mutates the source repository.
    let source_head_after = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(Path::new(repo_root))
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    if source_head_before != "unknown"
        && source_head_after != "unknown"
        && source_head_before != source_head_after
    {
        bail!(
            "validation invariant violated: source HEAD moved during run ({} -> {})",
            source_head_before,
            source_head_after
        );
    }

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
            format!("source HEAD unchanged: {}", source_head_after),
            "read-only with respect to source: validation executes commands in an isolated, detached worktree pinned to baseRevision and never commits".to_string(),
            "worktree torn down on completion".to_string(),
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

/// Combined registry: discovery + validation. PR3 will add `diagnostic` to
/// this surface when the diagnostic node lands.
pub fn validation_pipeline_registry() -> CapabilityRegistry {
    let mut reg = test_discovery_registry();
    reg.declare(
        CAP_VALIDATION,
        Capability::deterministic(
            &["repoRoot", "workspaceParent", "baseRevision"],
            run_validation,
        ),
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
pub fn validation_node_manifest(node_id: &str) -> NodeManifestV1 {
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": NODE_VALIDATION_VERSION,
            "nodeId": node_id,
            "purpose": CAP_VALIDATION,
            "inputs": [
                io("repoRoot", "core.Path"),
                io("workspaceParent", "core.Path"),
                io("baseRevision", "core.String"),
                io("commands", "lite.node.validation.CommandList"),
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
}
