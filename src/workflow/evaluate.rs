//! Fast Governed Loop V1 — automated evaluation pipeline.
//!
//! Takes a task from definition through `REVIEW_GATE`, producing a trustworthy
//! evidence bundle. The human still makes the final correctness decision.
//!
//! ```text
//! preflight
//! → generate exactly once
//! → inspect governance
//! → isolated dry-run
//! → classify infrastructure vs patch failures
//! → verify repository integrity
//! → write structured result
//! → stop at REVIEW_GATE
//! ```
//!
//! No automatic approval, patch application, commit creation, push, or
//! pull-request creation.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
extern crate libc;
#[cfg(windows)]
extern crate winapi;

use crate::harness::patch_provider::{PatchProvider, PatchProviderContext};
use crate::workflow::{
    AuthorityLevel, GenerateScope, ProposalArtifact, ProviderRouteInfo, is_git_repo,
    sanitize_provider_route,
};

// ---------------------------------------------------------------------------
// Schema version
// ---------------------------------------------------------------------------

/// Current evidence-bundle schema version.
const SCHEMA_VERSION: &str = "1.0.0";

// ---------------------------------------------------------------------------
// Task manifest
// ---------------------------------------------------------------------------

/// Machine-readable task definition for the evaluation pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManifest {
    /// Stable task identifier (caller-supplied or auto-generated).
    pub task_id: String,
    /// Human-readable goal.
    pub goal: String,
    /// Repository root to evaluate against.
    pub repo: PathBuf,
    /// Allowed repo-relative path prefixes.
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    /// Forbidden repo-relative path prefixes.
    #[serde(default)]
    pub forbidden_paths: Vec<String>,
    /// Whether dependency-manifest changes are permitted.
    #[serde(default)]
    pub allow_dependency_changes: bool,
    /// Maximum changed files before blocking.
    pub max_files_changed: Option<usize>,
    /// Maximum total changed lines before blocking.
    pub max_lines_changed: Option<usize>,
    /// Validation command (run in the isolated worktree).
    pub validation_command: Option<String>,
    /// Provider source: "config" or "mock".
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Authority level.
    #[serde(default = "default_authority")]
    pub authority: String,
    /// Minimum free disk space in bytes required for the worktree + target dir.
    #[serde(default = "default_min_disk_bytes")]
    pub min_disk_bytes: u64,
    /// Evidence output directory (default: `<repo>/.prometheos/evidence/<run_id>`).
    pub evidence_dir: Option<PathBuf>,
}

fn default_provider() -> String {
    "mock".to_string()
}
fn default_authority() -> String {
    "propose".to_string()
}
fn default_min_disk_bytes() -> u64 {
    100 * 1024 * 1024 // 100 MB
}

// ---------------------------------------------------------------------------
// Execution identity — persisted before any model call
// ---------------------------------------------------------------------------

/// Unique identity for a single evaluation run. Persisted to disk before the
/// model is invoked so that a process restart can detect an existing proposal
/// and reuse it rather than generating another candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionIdentity {
    /// Monotonically increasing run id (UUID).
    pub run_id: String,
    /// Caller-supplied task id.
    pub task_id: String,
    /// Repository path.
    pub repo: String,
    /// Repository HEAD at creation time.
    pub repo_pin: String,
    /// Model used for generation (may be "mock" or "none").
    pub model: String,
    /// Provider implementation name.
    pub provider: String,
    /// Governance scope effective for this run.
    pub governance_scope: GovernanceScopeSnapshot,
    /// RFC3339 creation timestamp.
    pub created_at: String,
    /// Current execution state.
    pub state: EvaluationState,
}

/// Snapshot of the governance scope at evaluation start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceScopeSnapshot {
    pub allowed_paths: Vec<String>,
    pub forbidden_paths: Vec<String>,
    pub allow_dependency_changes: bool,
    pub max_files_changed: Option<usize>,
    pub max_lines_changed: Option<usize>,
    pub authority: String,
    pub validation_command: Option<String>,
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/// Evaluation pipeline state. Every transition is append-only or recoverable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationState {
    /// Identity persisted, preflight not yet run.
    Created,
    /// Preflight checks passed.
    PreflightPassed,
    /// Generation in progress (exactly-once gate held).
    Generating,
    /// Proposal generated and persisted.
    ProposalGenerated,
    /// Governance checks passed.
    GovernancePassed,
    /// Validation in isolated worktree.
    Validating,
    /// Validation finished (pass or fail).
    ValidationComplete,
    /// Repository integrity verified.
    IntegrityVerified,
    /// Terminal: awaiting human correctness review.
    ReviewGate,
    // --- terminal failures ---
    /// Preflight failed (disk, git, credential, governance, writable dir).
    PreflightBlocked,
    /// Model/provider returned no usable proposal.
    GenerationFailed,
    /// Proposal violated governance constraints.
    GovernanceRejected,
    /// Candidate failed to compile or apply in the worktree.
    CandidateCompileFailed,
    /// Candidate compiled but tests failed.
    CandidateTestFailed,
    /// Validation command failed for non-infrastructure reasons.
    ValidationFailed,
    /// Infrastructure problem prevented validation (disk full, missing compiler, etc.).
    InfraBlocked,
    /// Original repository was modified during evaluation.
    IntegrityFailed,
    /// Internal error (should never happen).
    InternalError,
}

impl EvaluationState {
    /// True if this is a terminal state.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::ReviewGate
                | Self::PreflightBlocked
                | Self::GenerationFailed
                | Self::GovernanceRejected
                | Self::CandidateCompileFailed
                | Self::CandidateTestFailed
                | Self::ValidationFailed
                | Self::InfraBlocked
                | Self::IntegrityFailed
                | Self::InternalError
        )
    }

    /// Human-readable outcome label.
    pub fn outcome_label(self) -> &'static str {
        match self {
            Self::ReviewGate => "REVIEW_REQUIRED",
            Self::PreflightBlocked => "PREFLIGHT_BLOCKED",
            Self::GenerationFailed => "GENERATION_FAILED",
            Self::GovernanceRejected => "GOVERNANCE_REJECTED",
            Self::CandidateCompileFailed => "CANDIDATE_COMPILE_FAILED",
            Self::CandidateTestFailed => "CANDIDATE_TEST_FAILED",
            Self::ValidationFailed => "VALIDATION_FAILED",
            Self::InfraBlocked => "INFRA_BLOCKED",
            Self::IntegrityFailed => "INTEGRITY_FAILED",
            Self::InternalError => "INTERNAL_ERROR",
            Self::Created
            | Self::PreflightPassed
            | Self::Generating
            | Self::ProposalGenerated
            | Self::GovernancePassed
            | Self::Validating
            | Self::ValidationComplete
            | Self::IntegrityVerified => "in_progress",
        }
    }
}

// ---------------------------------------------------------------------------
// Deterministic identity registry (stateful, atomic reservation)
// ---------------------------------------------------------------------------

/// Registry entry tracking the full lifecycle of a proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// Current pipeline state for this identity key.
    pub state: ProposalState,
    /// Proposal ID (set after generation completes).
    pub proposal_id: Option<String>,
    /// Run ID of the process that holds the reservation.
    pub run_id: String,
    /// RFC3339 timestamp when the reservation was created.
    pub reserved_at: String,
    /// RFC3339 timestamp of the last state transition.
    pub updated_at: String,
}

/// Lifecycle state of a registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    /// Identity reserved; generation not yet started.
    Reserved,
    /// Generation in progress (model call running).
    Generating,
    /// Proposal generated and persisted; validation pending or in progress.
    ProposalGenerated,
    /// Validation complete; evidence bundle finalized.
    ValidationComplete,
}

/// Registry mapping identity keys to their lifecycle entries.
/// Stored at `<repo>/.prometheos/workflow/proposal_registry.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProposalRegistry {
    /// Maps identity_key → RegistryEntry.
    pub entries: std::collections::HashMap<String, RegistryEntry>,
}

/// Compute a deterministic identity key for a proposal lookup.
///
/// The key is a SHA-256 hash of the inputs that uniquely identify a
/// task+repository+governance+provider combination. Two evaluations with
/// identical keys should produce the same proposal (exactly-once).
pub fn compute_identity_key(
    task_id: &str,
    repo: &Path,
    base_commit: &str,
    provider: &str,
    model: &str,
    governance_scope: &GovernanceScopeSnapshot,
    validation_command: &Option<String>,
) -> String {
    let scope_hash = hash_str(&serde_json::to_string(governance_scope).unwrap_or_default());
    let validation_hash = hash_str(validation_command.as_deref().unwrap_or(""));
    let repo_canonical = repo
        .canonicalize()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| repo.display().to_string());

    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        task_id, repo_canonical, base_commit, provider, model, scope_hash, validation_hash
    );
    hash_str(&input)
}

fn registry_path(repo: &Path) -> PathBuf {
    repo.join(".prometheos")
        .join("workflow")
        .join("proposal_registry.json")
}

fn registry_lock_path(repo: &Path) -> PathBuf {
    repo.join(".prometheos")
        .join("workflow")
        .join("proposal_registry.lock")
}

fn load_registry(repo: &Path) -> ProposalRegistry {
    let path = registry_path(repo);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_registry(repo: &Path, registry: &ProposalRegistry) -> Result<()> {
    let path = registry_path(repo);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("failed to create workflow dir for registry")?;
    }
    // Write to a temp file, then atomically rename to avoid partial writes.
    let tmp = path.with_extension("json.tmp");
    let json =
        serde_json::to_string_pretty(registry).context("failed to serialize proposal registry")?;
    std::fs::write(&tmp, &json).context("failed to write proposal registry temp file")?;
    std::fs::rename(&tmp, &path).context("failed to atomically rename proposal registry")?;
    Ok(())
}

/// Try to acquire an atomic reservation for an identity key.
///
/// Returns `Ok(true)` if the reservation was acquired (new entry).
/// Returns `Ok(false)` if the entry already exists (caller should reuse or wait).
/// Returns `Err` on I/O failure.
fn try_reserve(repo: &Path, identity_key: &str, run_id: &str) -> Result<bool> {
    let lock_path = registry_lock_path(repo);

    // Ensure the workflow directory exists before creating the lock file.
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create workflow dir for lock file")?;
    }

    // Acquire an exclusive lock file.
    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_path)
        .context("failed to create registry lock file")?;

    // Use platform-exclusive lock (flock on Unix, LockFileEx on Windows).
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        unsafe {
            libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::LockFileEx;
        use winapi::um::minwinbase::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, OVERLAPPED,
        };
        let handle = lock_file.as_raw_handle();
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let result = unsafe {
            LockFileEx(
                handle as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            )
        };
        if result == 0 {
            // Lock held by another process.
            return Ok(false);
        }
    }

    // Now read the registry under the lock.
    let mut registry = load_registry(repo);
    if registry.entries.contains_key(identity_key) {
        // Another process reserved it first.
        drop(lock_file);
        let _ = std::fs::remove_file(&lock_path);
        return Ok(false);
    }

    // Reserve the identity.
    let now = now_iso();
    registry.entries.insert(
        identity_key.to_string(),
        RegistryEntry {
            state: ProposalState::Reserved,
            proposal_id: None,
            run_id: run_id.to_string(),
            reserved_at: now.clone(),
            updated_at: now,
        },
    );
    save_registry(repo, &registry)?;

    // Release lock and remove lock file.
    drop(lock_file);
    let _ = std::fs::remove_file(&lock_path);
    Ok(true)
}

/// Look up the registry entry for an identity key.
fn lookup_entry(repo: &Path, identity_key: &str) -> Option<RegistryEntry> {
    let registry = load_registry(repo);
    registry.entries.get(identity_key).cloned()
}

/// Transition the state of a registry entry.
fn transition_entry(
    repo: &Path,
    identity_key: &str,
    new_state: ProposalState,
    proposal_id: Option<&str>,
) -> Result<()> {
    let lock_path = registry_lock_path(repo);

    // Ensure the workflow directory exists before creating the lock file.
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create workflow dir for lock file")?;
    }

    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_path)
        .context("failed to create registry lock file for transition")?;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        unsafe {
            libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::LockFileEx;
        use winapi::um::minwinbase::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, OVERLAPPED,
        };
        let handle = lock_file.as_raw_handle();
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let result = unsafe {
            LockFileEx(
                handle as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            )
        };
        if result == 0 {
            bail!("failed to acquire lock for registry transition");
        }
    }

    let mut registry = load_registry(repo);
    let entry = registry
        .entries
        .get_mut(identity_key)
        .context("registry entry not found during transition")?;

    entry.state = new_state;
    entry.updated_at = now_iso();
    if let Some(pid) = proposal_id {
        entry.proposal_id = Some(pid.to_string());
    }
    save_registry(repo, &registry)?;

    drop(lock_file);
    let _ = std::fs::remove_file(&lock_path);
    Ok(())
}

/// Release a reservation (remove the entry from the registry).
/// Called when generation fails so another process can retry.
fn release_reservation(repo: &Path, identity_key: &str) -> Result<()> {
    let lock_path = registry_lock_path(repo);

    // Ensure the workflow directory exists before creating the lock file.
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).context("failed to create workflow dir for lock file")?;
    }

    let lock_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_path)
        .context("failed to create registry lock file for release")?;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        unsafe {
            libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::LockFileEx;
        use winapi::um::minwinbase::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, OVERLAPPED,
        };
        let handle = lock_file.as_raw_handle();
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let result = unsafe {
            LockFileEx(
                handle as _,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                1,
                0,
                &mut overlapped,
            )
        };
        if result == 0 {
            bail!("failed to acquire lock for reservation release");
        }
    }

    let mut registry = load_registry(repo);
    registry.entries.remove(identity_key);
    save_registry(repo, &registry)?;

    drop(lock_file);
    let _ = std::fs::remove_file(&lock_path);
    Ok(())
}

// ---------------------------------------------------------------------------
// Preflight
// ---------------------------------------------------------------------------

/// Disk space detection result. Fails closed: unknown disk space blocks preflight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiskSpaceStatus {
    /// Disk space successfully measured.
    Available(u64),
    /// Disk space measurement not supported on this platform.
    Unsupported,
    /// Disk space measurement failed.
    Failed(String),
}

/// Record of all preflight checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightResult {
    pub is_git_repo: bool,
    pub commit_at_start: String,
    pub working_tree_clean: bool,
    pub disk_space: DiskSpaceStatus,
    pub disk_space_sufficient: bool,
    pub credential_available: bool,
    pub validation_command_available: bool,
    pub governance_scope_valid: bool,
    pub evidence_dir_writable: bool,
}

// ---------------------------------------------------------------------------
// Evidence bundle (JSON)
// ---------------------------------------------------------------------------

/// Machine-readable evidence bundle produced by the evaluation pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceBundle {
    pub schema_version: String,
    pub run_id: String,
    pub task_id: String,
    pub repo: String,
    pub repo_pin_before: String,
    pub repo_pin_after: String,
    pub provider_provenance: ProviderProvenanceRecord,
    pub effective_governance: GovernanceScopeSnapshot,
    pub proposal: Option<ProposalRecord>,
    pub validation: Option<ValidationRecord>,
    pub failure_classification: Option<String>,
    pub integrity: Option<IntegrityRecord>,
    pub cleanup: Option<CleanupRecord>,
    pub raw_logs: RawLogPaths,
    pub final_state: String,
    pub completed_at: String,
}

/// Non-secret provider provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProvenanceRecord {
    pub implementation: String,
    pub model: Option<String>,
    pub route: Option<String>,
    pub generated_at: Option<String>,
    pub input_digest: Option<String>,
    pub patch_hash: Option<String>,
}

/// Proposal metadata recorded in the evidence bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalRecord {
    pub id: String,
    pub patch_hash: String,
    pub changed_files: Vec<String>,
    pub added_lines: usize,
    pub removed_lines: usize,
    pub base_sha: String,
}

/// Validation execution record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRecord {
    pub validation_command: Option<String>,
    pub exit_code: Option<i32>,
    pub stdout_preview: String,
    pub stderr_preview: String,
    pub start_time: String,
    pub completion_time: String,
    pub test_discovered: bool,
    pub test_executed: bool,
    pub test_names_found: Vec<String>,
    pub test_count: usize,
    pub warnings: Vec<String>,
    pub failures: Vec<String>,
    pub patch_applies_cleanly: bool,
    pub validation_passed: bool,
}

/// Repository integrity verification record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityRecord {
    pub original_commit_unchanged: bool,
    pub no_tracked_modifications: bool,
    pub no_staged_modifications: bool,
    pub candidate_changes_confined: bool,
    pub proposal_not_applied: bool,
}

/// Worktree cleanup record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupRecord {
    pub worktree_removed: bool,
    pub evidence_preserved: bool,
}

/// Paths to raw log files in the evidence directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawLogPaths {
    pub stdout: Option<PathBuf>,
    pub stderr: Option<PathBuf>,
    pub validation_output: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Pipeline orchestration
// ---------------------------------------------------------------------------

/// Configuration for a single evaluation run.
pub struct EvaluationConfig {
    pub manifest: TaskManifest,
    pub provider: Box<dyn PatchProvider>,
    pub route_info: Option<ProviderRouteInfo>,
}

/// Run the full evaluation pipeline and return the evidence bundle.
///
/// This is the primary entry point for the `workflow evaluate` command.
pub async fn evaluate(config: EvaluationConfig) -> Result<EvidenceBundle> {
    let repo = config.manifest.repo.clone();

    if !is_git_repo(&repo) {
        bail!("not a git repository: {}", repo.display());
    }

    let commit_at_start = git_rev_parse_head(&repo)?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let evidence_dir = config
        .manifest
        .evidence_dir
        .clone()
        .unwrap_or_else(|| evidence_dir_for(&repo, &run_id));
    std::fs::create_dir_all(&evidence_dir)
        .with_context(|| format!("failed to create evidence dir: {}", evidence_dir.display()))?;

    let governance_scope = GovernanceScopeSnapshot {
        allowed_paths: config.manifest.allowed_paths.clone(),
        forbidden_paths: config.manifest.forbidden_paths.clone(),
        allow_dependency_changes: config.manifest.allow_dependency_changes,
        max_files_changed: config.manifest.max_files_changed,
        max_lines_changed: config.manifest.max_lines_changed,
        authority: config.manifest.authority.clone(),
        validation_command: config.manifest.validation_command.clone(),
    };

    let identity = ExecutionIdentity {
        run_id: run_id.clone(),
        task_id: config.manifest.task_id.clone(),
        repo: repo.display().to_string(),
        repo_pin: commit_at_start.clone(),
        model: config
            .route_info
            .as_ref()
            .and_then(|r| r.model.clone())
            .unwrap_or_else(|| "mock".to_string()),
        provider: config.provider.name().to_string(),
        governance_scope: governance_scope.clone(),
        created_at: now_iso(),
        state: EvaluationState::Created,
    };

    // Persist identity before any model call (exactly-once gate).
    let identity_path = evidence_dir.join("execution_identity.json");
    std::fs::write(
        &identity_path,
        serde_json::to_string_pretty(&identity).context("failed to serialize identity")?,
    )
    .context("failed to persist execution identity")?;

    // Compute deterministic identity key for resume lookup.
    let identity_key = compute_identity_key(
        &config.manifest.task_id,
        &repo,
        &commit_at_start,
        config.provider.name(),
        identity.model.as_str(),
        &governance_scope,
        &config.manifest.validation_command,
    );

    // ---- Atomic reservation gate ----
    // Try to reserve the identity. If another process holds it, wait/reuse.
    let reserved = try_reserve(&repo, &identity_key, &run_id)
        .context("failed to attempt identity reservation")?;
    if !reserved {
        // Another process reserved this identity. Wait for it to complete,
        // then reuse the existing proposal.
        return wait_and_reuse(
            &repo,
            &commit_at_start,
            &run_id,
            &config.manifest,
            &config,
            &evidence_dir,
            &governance_scope,
            &identity_key,
        )
        .await;
    }

    // ---- Stage: Preflight ----
    let preflight = run_preflight(&repo, &commit_at_start, &config.manifest, &evidence_dir);
    let mut bundle = new_bundle(&identity, &commit_at_start, &repo, &evidence_dir);

    if let Err(_e) = &preflight {
        bundle.failure_classification = Some("preflight_blocked".to_string());
        bundle.final_state = EvaluationState::PreflightBlocked
            .outcome_label()
            .to_string();
        bundle.completed_at = now_iso();
        write_bundle(&evidence_dir, &bundle)?;
        return Ok(bundle);
    }
    let _preflight = preflight.unwrap();
    update_identity_state(&identity_path, EvaluationState::PreflightPassed);

    // ---- Stage: Generate ----
    transition_entry(&repo, &identity_key, ProposalState::Generating, None)
        .context("failed to transition to Generating state")?;
    update_identity_state(&identity_path, EvaluationState::Generating);
    let scope = GenerateScope {
        allowed_paths: config.manifest.allowed_paths.clone(),
        forbidden_paths: config.manifest.forbidden_paths.clone(),
        allow_dependency_changes: config.manifest.allow_dependency_changes,
        max_files_changed: config.manifest.max_files_changed,
        max_lines_changed: config.manifest.max_lines_changed,
    };
    let patch_context = PatchProviderContext {
        task: config.manifest.goal.clone(),
        ..Default::default()
    };

    let gen_result = match crate::workflow::generate_proposal(
        &repo,
        &config.manifest.goal,
        AuthorityLevel::from_str(&config.manifest.authority)?,
        config.provider.as_ref(),
        patch_context,
        &scope,
        config.route_info.clone(),
        config.manifest.validation_command.clone(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            let classification = classify_generation_error(&msg);
            bundle.failure_classification = Some(classification);
            bundle.final_state = EvaluationState::GenerationFailed
                .outcome_label()
                .to_string();
            bundle.completed_at = now_iso();
            // Release the reservation so another process can retry.
            let _ = release_reservation(&repo, &identity_key);
            write_bundle(&evidence_dir, &bundle)?;
            return Ok(bundle);
        }
    };
    update_identity_state(&identity_path, EvaluationState::ProposalGenerated);

    // Register the proposal in the registry.
    if let Err(e) = transition_entry(
        &repo,
        &identity_key,
        ProposalState::ProposalGenerated,
        Some(&gen_result.id),
    ) {
        eprintln!("warning: failed to transition registry to ProposalGenerated: {e}");
    }

    let proposal = load_proposal_from_repo(&repo, &gen_result.id)?;
    bundle.proposal = Some(ProposalRecord {
        id: proposal.id.clone(),
        patch_hash: gen_result.patch_hash.clone(),
        changed_files: proposal.changed_files.clone(),
        added_lines: proposal.added_lines,
        removed_lines: proposal.removed_lines,
        base_sha: proposal.base_sha.clone(),
    });
    bundle.provider_provenance = ProviderProvenanceRecord {
        implementation: config.provider.name().to_string(),
        model: config.route_info.as_ref().and_then(|r| r.model.clone()),
        route: config
            .route_info
            .as_ref()
            .and_then(|r| r.route.clone())
            .and_then(|u| sanitize_provider_route(&u)),
        generated_at: Some(now_iso()),
        input_digest: Some(hash_str(&config.manifest.goal)),
        patch_hash: Some(gen_result.patch_hash.clone()),
    };

    // ---- Stage: Governance verification ----
    // Governance is already enforced by `generate_proposal` → `propose_with_meta`.
    // Record that it passed.
    update_identity_state(&identity_path, EvaluationState::GovernancePassed);

    // ---- Stage: Isolated dry-run validation ----
    update_identity_state(&identity_path, EvaluationState::Validating);
    let validation_result = run_isolated_validation(
        &repo,
        &gen_result.id,
        config.manifest.validation_command.as_deref(),
        &evidence_dir,
    );
    update_identity_state(&identity_path, EvaluationState::ValidationComplete);

    // Transition registry to ValidationComplete.
    if let Err(e) = transition_entry(
        &repo,
        &identity_key,
        ProposalState::ValidationComplete,
        Some(&gen_result.id),
    ) {
        eprintln!("warning: failed to transition registry to ValidationComplete: {e}");
    }

    match &validation_result {
        Ok(vr) => {
            bundle.validation = Some(vr.clone());
            if vr.validation_passed {
                bundle.failure_classification =
                    Some("validation_passed_review_required".to_string());
            } else {
                let class = classify_validation_failure(vr);
                bundle.failure_classification = Some(class);
            }
        }
        Err(e) => {
            let msg = e.to_string();
            let classification = classify_dry_run_error(&msg);
            bundle.failure_classification = Some(classification);
        }
    }

    // ---- Stage: Repository integrity ----
    update_identity_state(&identity_path, EvaluationState::IntegrityVerified);
    let integrity = verify_repo_integrity(&repo, &commit_at_start, &gen_result.id);
    bundle.integrity = Some(integrity.clone());

    if !integrity.original_commit_unchanged
        || !integrity.no_tracked_modifications
        || !integrity.no_staged_modifications
    {
        bundle.failure_classification = Some("integrity_failed".to_string());
        bundle.final_state = EvaluationState::IntegrityFailed.outcome_label().to_string();
    } else if let Some(ref fc) = bundle.failure_classification {
        if fc == "validation_passed_review_required" {
            bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
        } else {
            // Map the failure classification to a terminal state.
            bundle.final_state = failure_to_terminal_state(fc).outcome_label().to_string();
        }
    } else {
        bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
    }

    // ---- Stage: Cleanup ----
    let cleanup = cleanup_worktree(&repo, &gen_result.id);
    bundle.cleanup = Some(cleanup);
    bundle.completed_at = now_iso();
    // Fill repo_pin_after on the returned bundle.
    if let Ok(head) = git_rev_parse_head(&repo) {
        bundle.repo_pin_after = head;
    }
    write_bundle(&evidence_dir, &bundle)?;

    Ok(bundle)
}

// ---------------------------------------------------------------------------
// Wait and reuse (exactly-once resume after concurrent or restart)
// ---------------------------------------------------------------------------

/// Wait for another process to complete its reservation, then reuse the result.
///
/// This handles three cases:
/// 1. The other process completed validation → return preserved evidence.
/// 2. The other process completed generation but not validation → resume validation.
/// 3. The other process is still in progress → wait and retry.
async fn wait_and_reuse(
    repo: &Path,
    commit_at_start: &str,
    run_id: &str,
    manifest: &TaskManifest,
    config: &EvaluationConfig,
    evidence_dir: &Path,
    governance_scope: &GovernanceScopeSnapshot,
    identity_key: &str,
) -> Result<EvidenceBundle> {
    // Run validation-specific preflight first.
    run_validation_preflight(repo, commit_at_start, manifest, evidence_dir)?;

    let max_wait = std::time::Duration::from_secs(300); // 5 minutes
    let poll_interval = std::time::Duration::from_millis(500);
    let mut elapsed = std::time::Duration::ZERO;

    loop {
        let entry = lookup_entry(repo, identity_key);
        match entry {
            Some(e) if e.state == ProposalState::ValidationComplete => {
                // The other process finished validation. Return the preserved evidence.
                let proposal_id = e
                    .proposal_id
                    .as_deref()
                    .context("ValidationComplete entry missing proposal_id")?;
                let proposal = load_proposal_from_repo(repo, proposal_id)?;
                return return_completed_evidence(
                    repo,
                    commit_at_start,
                    run_id,
                    manifest,
                    config,
                    evidence_dir,
                    governance_scope,
                    &proposal,
                    proposal_id,
                    identity_key,
                )
                .await;
            }
            Some(e) if e.state == ProposalState::ProposalGenerated => {
                // The other process finished generation but not validation.
                // Resume validation from this process.
                let proposal_id = e
                    .proposal_id
                    .as_deref()
                    .context("ProposalGenerated entry missing proposal_id")?;
                let proposal = load_proposal_from_repo(repo, proposal_id)?;
                return resume_validation(
                    repo,
                    commit_at_start,
                    run_id,
                    manifest,
                    config,
                    evidence_dir,
                    governance_scope,
                    &proposal,
                    identity_key,
                )
                .await;
            }
            Some(e) => {
                // Still in Reserved or Generating state. Wait and retry.
                // But first check if the reservation is stale (crashed process).
                let stale_threshold = std::time::Duration::from_secs(120); // 2 minutes
                if let Ok(reserved_time) = chrono::DateTime::parse_from_rfc3339(&e.reserved_at) {
                    let age = chrono::Utc::now()
                        .signed_duration_since(reserved_time)
                        .to_std()
                        .unwrap_or(std::time::Duration::ZERO);
                    if age > stale_threshold {
                        // Stale reservation — release it and retry from scratch.
                        let _ = release_reservation(repo, identity_key);
                        bail!(
                            "stale identity reservation detected (age: {}s); \
                             reservation released, caller should retry",
                            age.as_secs()
                        );
                    }
                }
                if elapsed >= max_wait {
                    bail!(
                        "timed out waiting for another process to complete \
                         identity reservation after {} seconds",
                        max_wait.as_secs()
                    );
                }
                tokio::time::sleep(poll_interval).await;
                elapsed += poll_interval;
            }
            None => {
                // Entry was removed (generation failed and reservation released).
                // Return an error so the caller can retry from scratch.
                bail!(
                    "identity reservation was released by another process \
                     (generation likely failed); caller should retry"
                );
            }
        }
    }
}

/// Return preserved evidence from a completed validation.
async fn return_completed_evidence(
    repo: &Path,
    commit_at_start: &str,
    run_id: &str,
    manifest: &TaskManifest,
    config: &EvaluationConfig,
    evidence_dir: &Path,
    governance_scope: &GovernanceScopeSnapshot,
    proposal: &ProposalArtifact,
    proposal_id: &str,
    identity_key: &str,
) -> Result<EvidenceBundle> {
    let mut bundle = new_bundle_from_identity(
        run_id,
        &manifest.task_id,
        repo,
        commit_at_start,
        governance_scope,
        evidence_dir,
    );

    bundle.proposal = Some(ProposalRecord {
        id: proposal.id.clone(),
        patch_hash: proposal.patch_hash.clone(),
        changed_files: proposal.changed_files.clone(),
        added_lines: proposal.added_lines,
        removed_lines: proposal.removed_lines,
        base_sha: proposal.base_sha.clone(),
    });
    bundle.provider_provenance = ProviderProvenanceRecord {
        implementation: config.provider.name().to_string(),
        model: config.route_info.as_ref().and_then(|r| r.model.clone()),
        route: config
            .route_info
            .as_ref()
            .and_then(|r| r.route.clone())
            .and_then(|u| sanitize_provider_route(&u)),
        generated_at: None,
        input_digest: Some(hash_str(&manifest.goal)),
        patch_hash: Some(proposal.patch_hash.clone()),
    };

    // Check if there's an existing evidence bundle from the previous run.
    // If so, load it instead of re-running validation.
    let existing_bundle = find_existing_evidence(evidence_dir, proposal_id);
    if let Some(mut eb) = existing_bundle {
        // Reuse the preserved validation result.
        eb.run_id = run_id.to_string();
        eb.completed_at = now_iso();
        if let Ok(head) = git_rev_parse_head(repo) {
            eb.repo_pin_after = head;
        }
        write_bundle(evidence_dir, &eb)?;
        return Ok(eb);
    }

    // No existing evidence — this shouldn't happen for ValidationComplete,
    // but handle it gracefully by running validation.
    resume_validation(
        repo,
        commit_at_start,
        run_id,
        manifest,
        config,
        evidence_dir,
        governance_scope,
        proposal,
        identity_key,
    )
    .await
}

/// Find an existing evidence bundle for a proposal.
fn find_existing_evidence(evidence_dir: &Path, proposal_id: &str) -> Option<EvidenceBundle> {
    // Look for bundle.json in the evidence directory.
    let bundle_path = evidence_dir.join("bundle.json");
    if bundle_path.exists() {
        let text = std::fs::read_to_string(&bundle_path).ok()?;
        let bundle: EvidenceBundle = serde_json::from_str(&text).ok()?;
        if bundle.proposal.as_ref().map(|p| p.id.as_str()) == Some(proposal_id) {
            return Some(bundle);
        }
    }
    None
}

/// Resume validation from the ProposalGenerated state.
async fn resume_validation(
    repo: &Path,
    commit_at_start: &str,
    run_id: &str,
    manifest: &TaskManifest,
    config: &EvaluationConfig,
    evidence_dir: &Path,
    governance_scope: &GovernanceScopeSnapshot,
    proposal: &ProposalArtifact,
    identity_key: &str,
) -> Result<EvidenceBundle> {
    let mut bundle = new_bundle_from_identity(
        run_id,
        &manifest.task_id,
        repo,
        commit_at_start,
        governance_scope,
        evidence_dir,
    );

    bundle.proposal = Some(ProposalRecord {
        id: proposal.id.clone(),
        patch_hash: proposal.patch_hash.clone(),
        changed_files: proposal.changed_files.clone(),
        added_lines: proposal.added_lines,
        removed_lines: proposal.removed_lines,
        base_sha: proposal.base_sha.clone(),
    });
    bundle.provider_provenance = ProviderProvenanceRecord {
        implementation: config.provider.name().to_string(),
        model: config.route_info.as_ref().and_then(|r| r.model.clone()),
        route: config
            .route_info
            .as_ref()
            .and_then(|r| r.route.clone())
            .and_then(|u| sanitize_provider_route(&u)),
        generated_at: None,
        input_digest: Some(hash_str(&manifest.goal)),
        patch_hash: Some(proposal.patch_hash.clone()),
    };

    // Run validation on the existing proposal.
    let validation_result = run_isolated_validation(
        repo,
        &proposal.id,
        manifest.validation_command.as_deref(),
        evidence_dir,
    );

    match &validation_result {
        Ok(vr) => {
            bundle.validation = Some(vr.clone());
            if vr.validation_passed {
                bundle.failure_classification =
                    Some("validation_passed_review_required".to_string());
            } else {
                let class = classify_validation_failure(vr);
                bundle.failure_classification = Some(class);
            }
        }
        Err(e) => {
            let msg = e.to_string();
            let classification = classify_dry_run_error(&msg);
            bundle.failure_classification = Some(classification);
        }
    }

    let integrity = verify_repo_integrity(repo, commit_at_start, &proposal.id);
    bundle.integrity = Some(integrity.clone());

    if !integrity.original_commit_unchanged
        || !integrity.no_tracked_modifications
        || !integrity.no_staged_modifications
    {
        bundle.failure_classification = Some("integrity_failed".to_string());
        bundle.final_state = EvaluationState::IntegrityFailed.outcome_label().to_string();
    } else if let Some(ref fc) = bundle.failure_classification {
        if fc == "validation_passed_review_required" {
            bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
        } else {
            bundle.final_state = failure_to_terminal_state(fc).outcome_label().to_string();
        }
    } else {
        bundle.final_state = EvaluationState::ReviewGate.outcome_label().to_string();
    }

    // Transition registry to ValidationComplete.
    if let Err(e) = transition_entry(
        repo,
        identity_key,
        ProposalState::ValidationComplete,
        Some(&proposal.id),
    ) {
        eprintln!("warning: failed to transition registry to ValidationComplete: {e}");
    }

    let cleanup = cleanup_worktree(repo, &proposal.id);
    bundle.cleanup = Some(cleanup);
    bundle.completed_at = now_iso();
    if let Ok(head) = git_rev_parse_head(repo) {
        bundle.repo_pin_after = head;
    }
    write_bundle(evidence_dir, &bundle)?;

    Ok(bundle)
}

// ---------------------------------------------------------------------------
// Preflight
// ---------------------------------------------------------------------------

fn run_preflight(
    repo: &Path,
    commit: &str,
    manifest: &TaskManifest,
    evidence_dir: &Path,
) -> Result<PreflightResult> {
    let is_git = is_git_repo(repo);
    let working_tree_clean = is_repo_clean(repo);
    let disk_space = available_disk_bytes(repo);
    let disk_sufficient = match &disk_space {
        DiskSpaceStatus::Available(bytes) => *bytes >= manifest.min_disk_bytes,
        // Fail closed: unknown disk space is treated as insufficient.
        DiskSpaceStatus::Unsupported | DiskSpaceStatus::Failed(_) => false,
    };
    let credential_available = check_credential_available(&manifest.provider);
    let validation_available = manifest
        .validation_command
        .as_ref()
        .map(|cmd| check_command_available(cmd))
        .unwrap_or(true);
    let governance_valid = !manifest.authority.is_empty();
    let evidence_writable = evidence_dir
        .join("test_write_probe")
        .to_path_buf()
        .as_path()
        .parent()
        .map(|d| {
            std::fs::write(d.join(".prometheos_write_probe"), "ok").is_ok()
                && std::fs::remove_file(d.join(".prometheos_write_probe")).is_ok()
        })
        .unwrap_or(false);

    // Clean up the probe file
    let _ = std::fs::remove_file(evidence_dir.join(".prometheos_write_probe"));

    let result = PreflightResult {
        is_git_repo: is_git,
        commit_at_start: commit.to_string(),
        working_tree_clean,
        disk_space: disk_space.clone(),
        disk_space_sufficient: disk_sufficient,
        credential_available,
        validation_command_available: validation_available,
        governance_scope_valid: governance_valid,
        evidence_dir_writable: evidence_writable,
    };

    let mut errors = Vec::new();
    if !result.is_git_repo {
        errors.push("not a git repository".to_string());
    }
    if !result.disk_space_sufficient {
        let detail = match &result.disk_space {
            DiskSpaceStatus::Available(bytes) => {
                format!(
                    "{} bytes available, {} required",
                    bytes, manifest.min_disk_bytes
                )
            }
            DiskSpaceStatus::Unsupported => {
                "disk space measurement not supported on this platform".to_string()
            }
            DiskSpaceStatus::Failed(msg) => {
                format!("disk space measurement failed: {msg}")
            }
        };
        errors.push(format!("insufficient or unmeasurable disk space: {detail}"));
    }
    if !result.credential_available {
        errors.push("provider credential not available".to_string());
    }
    if !result.validation_command_available {
        errors.push("validation command not available".to_string());
    }
    if !result.governance_scope_valid {
        errors.push("governance scope invalid (empty authority)".to_string());
    }
    if !result.evidence_dir_writable {
        errors.push("evidence directory not writable".to_string());
    }

    if !errors.is_empty() {
        bail!("preflight failed:\n- {}", errors.join("\n- "));
    }

    Ok(result)
}

/// Validation-specific preflight checks. Used when resuming validation on an
/// existing proposal. Does NOT require provider credentials (generation already
/// happened), but does require disk space, validation command, and evidence
/// writability.
fn run_validation_preflight(
    repo: &Path,
    commit: &str,
    manifest: &TaskManifest,
    evidence_dir: &Path,
) -> Result<PreflightResult> {
    let is_git = is_git_repo(repo);
    let working_tree_clean = is_repo_clean(repo);
    let disk_space = available_disk_bytes(repo);
    let disk_sufficient = match &disk_space {
        DiskSpaceStatus::Available(bytes) => *bytes >= manifest.min_disk_bytes,
        DiskSpaceStatus::Unsupported | DiskSpaceStatus::Failed(_) => false,
    };
    // Validation does NOT require credentials — generation already happened.
    let credential_available = true;
    let validation_available = manifest
        .validation_command
        .as_ref()
        .map(|cmd| check_command_available(cmd))
        .unwrap_or(true);
    let governance_valid = !manifest.authority.is_empty();
    let evidence_writable = evidence_dir
        .join("test_write_probe")
        .to_path_buf()
        .as_path()
        .parent()
        .map(|d| {
            std::fs::write(d.join(".prometheos_write_probe"), "ok").is_ok()
                && std::fs::remove_file(d.join(".prometheos_write_probe")).is_ok()
        })
        .unwrap_or(false);

    let _ = std::fs::remove_file(evidence_dir.join(".prometheos_write_probe"));

    let result = PreflightResult {
        is_git_repo: is_git,
        commit_at_start: commit.to_string(),
        working_tree_clean,
        disk_space: disk_space.clone(),
        disk_space_sufficient: disk_sufficient,
        credential_available,
        validation_command_available: validation_available,
        governance_scope_valid: governance_valid,
        evidence_dir_writable: evidence_writable,
    };

    let mut errors = Vec::new();
    if !result.is_git_repo {
        errors.push("not a git repository".to_string());
    }
    if !result.disk_space_sufficient {
        let detail = match &result.disk_space {
            DiskSpaceStatus::Available(bytes) => {
                format!(
                    "{} bytes available, {} required",
                    bytes, manifest.min_disk_bytes
                )
            }
            DiskSpaceStatus::Unsupported => {
                "disk space measurement not supported on this platform".to_string()
            }
            DiskSpaceStatus::Failed(msg) => {
                format!("disk space measurement failed: {msg}")
            }
        };
        errors.push(format!("insufficient or unmeasurable disk space: {detail}"));
    }
    if !result.validation_command_available {
        errors.push("validation command not available".to_string());
    }
    if !result.evidence_dir_writable {
        errors.push("evidence directory not writable".to_string());
    }

    if !errors.is_empty() {
        bail!("validation preflight failed:\n- {}", errors.join("\n- "));
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Validation (isolated worktree)
// ---------------------------------------------------------------------------

fn run_isolated_validation(
    repo: &Path,
    proposal_id: &str,
    validation_command: Option<&str>,
    evidence_dir: &Path,
) -> Result<ValidationRecord> {
    let proposal = load_proposal_from_repo(repo, proposal_id)?;
    let start_time = now_iso();

    let wt_root = std::env::temp_dir().join(format!("prometheos-eval-{proposal_id}"));
    // Clean any stale state.
    let _ = run_git_cmd(
        repo,
        &["worktree", "remove", "--force", wt_root.to_str().unwrap()],
    );
    let _ = std::fs::remove_dir_all(&wt_root);
    let _ = run_git_cmd(repo, &["worktree", "prune"]);

    let patch_file =
        std::env::temp_dir().join(format!("prometheos-eval-patch-{proposal_id}.patch"));
    std::fs::write(&patch_file, &proposal.patch)
        .context("failed to write patch file for validation")?;

    // Create detached worktree at base sha.
    run_git_cmd(
        repo,
        &[
            "worktree",
            "add",
            "--detach",
            wt_root.to_str().unwrap(),
            &proposal.base_sha,
        ],
    )
    .context("failed to create validation worktree")?;

    // Step 1: Check if patch applies cleanly.
    let patch_applies = run_git_cmd(
        &wt_root,
        &["apply", "--check", patch_file.to_str().unwrap()],
    )
    .is_ok();

    if !patch_applies {
        // Patch doesn't apply — record and clean up.
        let _ = run_git_cmd(
            repo,
            &["worktree", "remove", "--force", wt_root.to_str().unwrap()],
        );
        let _ = std::fs::remove_dir_all(&wt_root);
        let _ = std::fs::remove_file(&patch_file);

        let completion_time = now_iso();
        return Ok(ValidationRecord {
            validation_command: validation_command.map(|s| s.to_string()),
            exit_code: None,
            stdout_preview: String::new(),
            stderr_preview: "patch does not apply cleanly".to_string(),
            start_time,
            completion_time,
            test_discovered: false,
            test_executed: false,
            test_names_found: Vec::new(),
            test_count: 0,
            warnings: Vec::new(),
            failures: vec!["patch apply check failed".to_string()],
            patch_applies_cleanly: false,
            validation_passed: false,
        });
    }

    // Apply the patch.
    let _ = run_git_cmd(&wt_root, &["apply", patch_file.to_str().unwrap()]);

    // Step 2: Run validation command if present.
    let (exit_code, stdout, stderr) = match validation_command {
        Some(cmd) => {
            let output = validation_shell(cmd)
                .current_dir(&wt_root)
                .output()
                .context("failed to execute validation command")?;
            (
                Some(output.status.code().unwrap_or(-1)),
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
            )
        }
        None => (None, String::new(), String::new()),
    };

    let completion_time = now_iso();

    // Discover tests from output.
    let (test_discovered, test_executed, test_names, test_count, warnings, failures) =
        parse_test_evidence(&stdout, &stderr, exit_code);

    let validation_passed = exit_code.map(|c| c == 0).unwrap_or(true) && patch_applies;

    // Save raw logs.
    let stdout_path = evidence_dir.join("validation_stdout.log");
    let stderr_path = evidence_dir.join("validation_stderr.log");
    let _ = std::fs::write(&stdout_path, &stdout);
    let _ = std::fs::write(&stderr_path, &stderr);

    // Clean up worktree.
    let _ = run_git_cmd(
        repo,
        &["worktree", "remove", "--force", wt_root.to_str().unwrap()],
    );
    let _ = std::fs::remove_dir_all(&wt_root);
    let _ = std::fs::remove_file(&patch_file);

    Ok(ValidationRecord {
        validation_command: validation_command.map(|s| s.to_string()),
        exit_code,
        stdout_preview: truncate(&stdout, 4096),
        stderr_preview: truncate(&stderr, 4096),
        start_time,
        completion_time,
        test_discovered,
        test_executed,
        test_names_found: test_names,
        test_count,
        warnings,
        failures,
        patch_applies_cleanly: patch_applies,
        validation_passed,
    })
}

// ---------------------------------------------------------------------------
// Test evidence parsing
// ---------------------------------------------------------------------------

fn parse_test_evidence(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
) -> (bool, bool, Vec<String>, usize, Vec<String>, Vec<String>) {
    let combined = format!("{stdout}\n{stderr}");

    // Discover test binary names.
    let test_names = extract_test_names(&combined);
    let test_discovered = !test_names.is_empty();

    // Detect test execution markers.
    let test_executed = combined.contains("test result:")
        || combined.contains("running")
        || combined.contains("test .")
        || combined.contains("FAILED")
        || combined.contains("ok")
        || combined.contains(".test.");

    // Count tests from "test result: ok. N passed" lines.
    let test_count = count_tests_from_output(&combined);

    // Extract warnings.
    let warnings = extract_patterns(&combined, &["warning:", "WARNING:", "warn:"]);

    // Extract failures.
    let failures = extract_patterns(
        &combined,
        &["FAILED", "error:", "ERROR:", "panicked", "failures:"],
    );

    // If exit code is non-zero and no specific failures found, add generic failure.
    if exit_code.map(|c| c != 0).unwrap_or(false) && failures.is_empty() {
        let mut f = Vec::new();
        f.push(format!(
            "validation exited with code {}",
            exit_code.unwrap()
        ));
        (
            test_discovered,
            test_executed,
            test_names,
            test_count,
            warnings,
            f,
        )
    } else {
        (
            test_discovered,
            test_executed,
            test_names,
            test_count,
            warnings,
            failures,
        )
    }
}

fn extract_test_names(output: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in output.lines() {
        // Rust test output: "test module::test_name ... ok"
        if line.starts_with("test ")
            && let Some(name) = line.split_whitespace().nth(1)
            && name != "result"
            && !names.contains(&name.to_string())
        {
            names.push(name.to_string());
        }
        // cargo test output: "Running target/..."
        if line.starts_with("Running ")
            && let Some(path) = line.strip_prefix("Running ")
        {
            let name = path.split('/').next_back().unwrap_or(path).to_string();
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn count_tests_from_output(output: &str) -> usize {
    let mut count = 0usize;
    for line in output.lines() {
        // "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"
        if let Some(rest) = line.strip_prefix("test result:") {
            for part in rest.split(';') {
                let part = part.trim();
                if let Some(passed_part) = part.strip_suffix("passed") {
                    // Format: "N passed" or "ok. N passed"
                    let passed_part = passed_part.trim();
                    // Extract the number: could be "5" or "ok. 5"
                    if let Some(n_str) = passed_part.split_whitespace().last()
                        && let Ok(v) = n_str.parse::<usize>()
                    {
                        count += v;
                    }
                }
            }
        }
    }
    count
}

fn extract_patterns(output: &str, patterns: &[&str]) -> Vec<String> {
    let mut results = Vec::new();
    for line in output.lines() {
        for pat in patterns {
            if line.contains(pat) {
                let trimmed = line.trim().to_string();
                if !trimmed.is_empty() && !results.contains(&trimmed) {
                    results.push(trimmed);
                }
                break;
            }
        }
    }
    results
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}…[truncated, {} bytes total]", &s[..max_chars], s.len())
    }
}

// ---------------------------------------------------------------------------
// Failure classification
// ---------------------------------------------------------------------------

fn classify_generation_error(msg: &str) -> String {
    if msg.contains("disk")
        || msg.contains("ENOSPC")
        || msg.contains("credential")
        || msg.contains("API key")
        || msg.contains("401")
        || msg.contains("network")
        || msg.contains("timeout")
        || msg.contains("ECONNREFUSED")
    {
        "infra_blocked".to_string()
    } else {
        "generation_failed".to_string()
    }
}

fn classify_dry_run_error(msg: &str) -> String {
    if msg.contains("disk") || msg.contains("ENOSPC") {
        "infra_blocked".to_string()
    } else if msg.contains("compiler") || msg.contains("cargo") || msg.contains("rustc") {
        "candidate_compile_failed".to_string()
    } else if msg.contains("worktree") || msg.contains("git") {
        "infra_blocked".to_string()
    } else if msg.contains("validation command failed") {
        "validation_failed".to_string()
    } else if msg.contains("patch does not apply") {
        "candidate_compile_failed".to_string()
    } else {
        "validation_failed".to_string()
    }
}

pub fn classify_validation_failure(vr: &ValidationRecord) -> String {
    // Infrastructure classification must be supported by concrete evidence.
    let stderr = &vr.stderr_preview;
    let lower = stderr.to_lowercase();
    if lower.contains("disk full") || lower.contains("enospc") {
        return "infra_blocked".to_string();
    }
    if lower.contains("no space left") {
        return "infra_blocked".to_string();
    }
    if lower.contains("compiler not found") || lower.contains("cargo: not found") {
        return "infra_blocked".to_string();
    }

    // Compilation failures are NOT infrastructure.
    if stderr.contains("error[") || stderr.contains("could not compile") {
        return "candidate_compile_failed".to_string();
    }

    // Test failures.
    if !vr.failures.is_empty() {
        return "candidate_test_failed".to_string();
    }

    // Validation command failure (non-zero exit, no specific classification).
    if vr.exit_code.map(|c| c != 0).unwrap_or(false) {
        return "validation_failed".to_string();
    }

    "validation_failed".to_string()
}

fn failure_to_terminal_state(classification: &str) -> EvaluationState {
    match classification {
        "preflight_blocked" => EvaluationState::PreflightBlocked,
        "generation_failed" => EvaluationState::GenerationFailed,
        "governance_rejected" => EvaluationState::GovernanceRejected,
        "candidate_compile_failed" => EvaluationState::CandidateCompileFailed,
        "candidate_test_failed" => EvaluationState::CandidateTestFailed,
        "validation_failed" => EvaluationState::ValidationFailed,
        "infra_blocked" => EvaluationState::InfraBlocked,
        "integrity_failed" => EvaluationState::IntegrityFailed,
        "validation_passed_review_required" => EvaluationState::ReviewGate,
        _ => EvaluationState::InternalError,
    }
}

// ---------------------------------------------------------------------------
// Repository integrity
// ---------------------------------------------------------------------------

pub fn verify_repo_integrity(
    repo: &Path,
    expected_commit: &str,
    proposal_id: &str,
) -> IntegrityRecord {
    let current_commit = git_rev_parse_head(repo).unwrap_or_default();
    let original_commit_unchanged = current_commit == expected_commit;

    let status = run_git_cmd(repo, &["status", "--porcelain"]).unwrap_or_default();
    let no_tracked_modifications = status.lines().all(|line| {
        let path = line.get(3..).unwrap_or("").trim();
        path.starts_with(".prometheos/")
    });

    let staged = run_git_cmd(repo, &["diff", "--cached", "--name-only"]).unwrap_or_default();
    let no_staged_modifications = staged.trim().is_empty();

    // Check that the proposal was not applied.
    let proposal = load_proposal_from_repo(repo, proposal_id).ok();
    let proposal_not_applied = proposal.map(|p| p.applied != Some(true)).unwrap_or(true);

    // Candidate changes confined: no untracked files outside .prometheos/.
    let candidate_changes_confined = status.lines().all(|line| {
        let path = line.get(3..).unwrap_or("").trim();
        path.starts_with(".prometheos/")
    });

    IntegrityRecord {
        original_commit_unchanged,
        no_tracked_modifications,
        no_staged_modifications,
        candidate_changes_confined,
        proposal_not_applied,
    }
}

// ---------------------------------------------------------------------------
// Cleanup
// ---------------------------------------------------------------------------

fn cleanup_worktree(repo: &Path, proposal_id: &str) -> CleanupRecord {
    let wt_root = std::env::temp_dir().join(format!("prometheos-eval-{proposal_id}"));
    let patch_file =
        std::env::temp_dir().join(format!("prometheos-eval-patch-{proposal_id}.patch"));

    let worktree_removed = run_git_cmd(
        repo,
        &["worktree", "remove", "--force", wt_root.to_str().unwrap()],
    )
    .is_ok()
        || !wt_root.exists();

    let _ = std::fs::remove_dir_all(&wt_root);
    let _ = std::fs::remove_file(&patch_file);

    // Evidence is preserved in the evidence directory, not in the worktree.
    CleanupRecord {
        worktree_removed,
        evidence_preserved: true,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn git_rev_parse_head(repo: &Path) -> Result<String> {
    let out = run_git_cmd(repo, &["rev-parse", "HEAD"])?;
    Ok(out.trim().to_string())
}

fn run_git_cmd(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .context("failed to execute git")?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim())
    }
}

fn is_repo_clean(repo: &Path) -> bool {
    run_git_cmd(repo, &["status", "--porcelain"])
        .map(|s| s.trim().is_empty())
        .unwrap_or(false)
}

/// Detect available disk space for the filesystem containing `path`.
///
/// Returns `DiskSpaceStatus::Available(bytes)` on success, or
/// `Unsupported`/`Failed` when measurement is impossible. The caller must
/// fail closed on unknown disk space — never assume infinite capacity.
pub fn available_disk_bytes(path: &Path) -> DiskSpaceStatus {
    // Resolve to an existing ancestor directory.
    let dir = path.ancestors().find(|a| a.exists()).unwrap_or(path);

    // Try sysinfo first (cross-platform, already a dependency).
    if let Some(bytes) = sysinfo_disk_available(dir) {
        return DiskSpaceStatus::Available(bytes);
    }

    // Platform-specific fallbacks.
    #[cfg(target_os = "windows")]
    {
        windows_disk_available(dir)
    }
    #[cfg(not(target_os = "windows"))]
    {
        DiskSpaceStatus::Unsupported
    }
}

/// Use sysinfo to find available disk space for the filesystem containing `dir`.
fn sysinfo_disk_available(dir: &Path) -> Option<u64> {
    use sysinfo::Disks;
    let disks = Disks::new_with_refreshed_list();
    let dir_str = dir.to_string_lossy();
    // Find the disk whose mount point is a prefix of the directory path.
    // Sort by mount point length descending so longest (most specific) match wins.
    let mut candidates: Vec<_> = disks.iter().collect();
    candidates.sort_by(|a, b| b.mount_point().cmp(a.mount_point()));
    for disk in candidates {
        let mount = disk.mount_point().to_string_lossy();
        if dir_str.starts_with(mount.as_ref()) || mount.starts_with(&dir_str[..]) {
            return Some(disk.available_space());
        }
    }
    None
}

/// Windows-specific disk space detection via Win32 API.
#[cfg(target_os = "windows")]
fn windows_disk_available(dir: &Path) -> DiskSpaceStatus {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut i64,
            lpTotalNumberOfBytes: *mut i64,
            lpTotalNumberOfFreeBytes: *mut i64,
        ) -> i32;
    }

    let wide: Vec<u16> = dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free_bytes: i64 = 0;
    let success = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_bytes,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if success != 0 {
        DiskSpaceStatus::Available(free_bytes as u64)
    } else {
        DiskSpaceStatus::Failed("GetDiskFreeSpaceExW failed".to_string())
    }
}

fn check_credential_available(provider: &str) -> bool {
    if provider == "mock" {
        return true;
    }
    // Check for common provider environment variables without exposing values.
    if std::env::var("PROMETHEOS_API_KEY").is_ok() || std::env::var("OPENAI_API_KEY").is_ok() {
        return true;
    }
    // If the provider is "config", check if the config file has credentials.
    if provider == "config" {
        return crate::config::AppConfig::load().is_ok();
    }
    false
}

fn check_command_available(cmd: &str) -> bool {
    // Extract the first token (the program name).
    let program = cmd.split_whitespace().next().unwrap_or(cmd);
    // On Windows, try `where`; on Unix, try `which`.
    #[cfg(windows)]
    let result = Command::new("where")
        .arg(program)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    #[cfg(not(windows))]
    let result = Command::new("which")
        .arg(program)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    result
}

/// Platform-aware shell for validation commands.
fn validation_shell(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

fn load_proposal_from_repo(repo: &Path, id: &str) -> Result<ProposalArtifact> {
    let path = repo
        .join(".prometheos")
        .join("workflow")
        .join(id)
        .join("proposal.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read proposal {id} at {}", path.display()))?;
    serde_json::from_str(&text).context("failed to parse proposal artifact")
}

fn update_identity_state(path: &Path, state: EvaluationState) {
    if let Ok(text) = std::fs::read_to_string(path)
        && let Ok(mut identity) = serde_json::from_str::<ExecutionIdentity>(&text)
    {
        identity.state = state;
        if let Ok(json) = serde_json::to_string_pretty(&identity) {
            let _ = std::fs::write(path, json);
        }
    }
}

fn evidence_dir_for(repo: &Path, run_id: &str) -> PathBuf {
    repo.join(".prometheos").join("evidence").join(run_id)
}

fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| secs.to_string())
}

fn hash_str(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn new_bundle(
    identity: &ExecutionIdentity,
    commit_at_start: &str,
    repo: &Path,
    _evidence_dir: &Path,
) -> EvidenceBundle {
    EvidenceBundle {
        schema_version: SCHEMA_VERSION.to_string(),
        run_id: identity.run_id.clone(),
        task_id: identity.task_id.clone(),
        repo: repo.display().to_string(),
        repo_pin_before: commit_at_start.to_string(),
        repo_pin_after: String::new(), // filled at end
        provider_provenance: ProviderProvenanceRecord {
            implementation: identity.provider.clone(),
            model: Some(identity.model.clone()),
            route: None,
            generated_at: None,
            input_digest: None,
            patch_hash: None,
        },
        effective_governance: identity.governance_scope.clone(),
        proposal: None,
        validation: None,
        failure_classification: None,
        integrity: None,
        cleanup: None,
        raw_logs: RawLogPaths {
            stdout: None,
            stderr: None,
            validation_output: None,
        },
        final_state: "in_progress".to_string(),
        completed_at: String::new(),
    }
}

fn new_bundle_from_identity(
    run_id: &str,
    task_id: &str,
    repo: &Path,
    commit_at_start: &str,
    governance_scope: &GovernanceScopeSnapshot,
    _evidence_dir: &Path,
) -> EvidenceBundle {
    EvidenceBundle {
        schema_version: SCHEMA_VERSION.to_string(),
        run_id: run_id.to_string(),
        task_id: task_id.to_string(),
        repo: repo.display().to_string(),
        repo_pin_before: commit_at_start.to_string(),
        repo_pin_after: String::new(),
        provider_provenance: ProviderProvenanceRecord {
            implementation: "unknown".to_string(),
            model: None,
            route: None,
            generated_at: None,
            input_digest: None,
            patch_hash: None,
        },
        effective_governance: governance_scope.clone(),
        proposal: None,
        validation: None,
        failure_classification: None,
        integrity: None,
        cleanup: None,
        raw_logs: RawLogPaths {
            stdout: None,
            stderr: None,
            validation_output: None,
        },
        final_state: "in_progress".to_string(),
        completed_at: String::new(),
    }
}

fn write_bundle(evidence_dir: &Path, bundle: &EvidenceBundle) -> Result<()> {
    // Fill repo_pin_after.
    let mut bundle = bundle.clone();
    if let Ok(head) = git_rev_parse_head(Path::new(&bundle.repo)) {
        bundle.repo_pin_after = head;
    }

    let json_path = evidence_dir.join("evidence.json");
    let json =
        serde_json::to_string_pretty(&bundle).context("failed to serialize evidence bundle")?;
    std::fs::write(&json_path, &json).context("failed to write evidence.json")?;

    // Write Markdown report.
    let md_path = evidence_dir.join("evidence.md");
    let md = render_markdown_report(&bundle);
    std::fs::write(&md_path, &md).context("failed to write evidence.md")?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Markdown report
// ---------------------------------------------------------------------------

fn render_markdown_report(bundle: &EvidenceBundle) -> String {
    let mut md = String::new();
    md.push_str(&format!("# Evaluation Evidence — {}\n\n", bundle.task_id));
    md.push_str(&format!("**Schema:** `{}`\n", bundle.schema_version));
    md.push_str(&format!("**Run:** `{}`\n", bundle.run_id));
    md.push_str(&format!("**Repository:** `{}`\n", bundle.repo));
    md.push_str(&format!("**Pin before:** `{}`\n", bundle.repo_pin_before));
    md.push_str(&format!("**Pin after:** `{}`\n", bundle.repo_pin_after));
    md.push_str(&format!("**Completed:** {}\n\n", bundle.completed_at));

    md.push_str("## Outcome\n\n");
    md.push_str(&format!("**Result:** `{}`\n\n", bundle.final_state));

    if let Some(ref fc) = bundle.failure_classification {
        md.push_str(&format!("**Classification:** `{fc}`\n\n"));
    }

    md.push_str("## Provider\n\n");
    md.push_str(&format!(
        "- Implementation: `{}`\n",
        bundle.provider_provenance.implementation
    ));
    if let Some(ref model) = bundle.provider_provenance.model {
        md.push_str(&format!("- Model: `{model}`\n"));
    }
    if let Some(ref route) = bundle.provider_provenance.route {
        md.push_str(&format!("- Route: `{route}`\n"));
    }

    if let Some(ref proposal) = bundle.proposal {
        md.push_str("\n## Proposal\n\n");
        md.push_str(&format!("- ID: `{}`\n", proposal.id));
        md.push_str(&format!("- Patch hash: `{}`\n", proposal.patch_hash));
        md.push_str(&format!("- Base SHA: `{}`\n", proposal.base_sha));
        md.push_str(&format!(
            "- Changed files: {}\n",
            proposal.changed_files.len()
        ));
        md.push_str(&format!(
            "- Lines: +{} / -{}\n",
            proposal.added_lines, proposal.removed_lines
        ));
        md.push_str(&format!("- Paths: {}\n", proposal.changed_files.join(", ")));
    }

    if let Some(ref validation) = bundle.validation {
        md.push_str("\n## Validation\n\n");
        md.push_str(&format!(
            "- Command: `{}`\n",
            validation.validation_command.as_deref().unwrap_or("(none)")
        ));
        md.push_str(&format!(
            "- Exit code: {}\n",
            validation
                .exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "N/A".to_string())
        ));
        md.push_str(&format!(
            "- Patch applies cleanly: {}\n",
            validation.patch_applies_cleanly
        ));
        md.push_str(&format!(
            "- Validation passed: {}\n",
            validation.validation_passed
        ));
        md.push_str(&format!(
            "- Test discovered: {}\n",
            validation.test_discovered
        ));
        md.push_str(&format!("- Test executed: {}\n", validation.test_executed));
        md.push_str(&format!("- Test count: {}\n", validation.test_count));
        if !validation.test_names_found.is_empty() {
            md.push_str(&format!(
                "- Test names: {}\n",
                validation.test_names_found.join(", ")
            ));
        }
        if !validation.warnings.is_empty() {
            md.push_str(&format!("- Warnings: {}\n", validation.warnings.len()));
        }
        if !validation.failures.is_empty() {
            md.push_str(&format!("- Failures: {}\n", validation.failures.len()));
            for f in &validation.failures {
                md.push_str(&format!("  - `{f}`\n"));
            }
        }
    }

    if let Some(ref integrity) = bundle.integrity {
        md.push_str("\n## Integrity\n\n");
        md.push_str(&format!(
            "- Original commit unchanged: {}\n",
            integrity.original_commit_unchanged
        ));
        md.push_str(&format!(
            "- No tracked modifications: {}\n",
            integrity.no_tracked_modifications
        ));
        md.push_str(&format!(
            "- No staged modifications: {}\n",
            integrity.no_staged_modifications
        ));
        md.push_str(&format!(
            "- Candidate changes confined: {}\n",
            integrity.candidate_changes_confined
        ));
        md.push_str(&format!(
            "- Proposal not applied: {}\n",
            integrity.proposal_not_applied
        ));
    }

    if let Some(ref cleanup) = bundle.cleanup {
        md.push_str("\n## Cleanup\n\n");
        md.push_str(&format!(
            "- Worktree removed: {}\n",
            cleanup.worktree_removed
        ));
        md.push_str(&format!(
            "- Evidence preserved: {}\n",
            cleanup.evidence_preserved
        ));
    }

    md.push_str("\n## Governance\n\n");
    md.push_str(&format!(
        "- Authority: `{}`\n",
        bundle.effective_governance.authority
    ));
    md.push_str(&format!(
        "- Allowed paths: {}\n",
        if bundle.effective_governance.allowed_paths.is_empty() {
            "(any)".to_string()
        } else {
            bundle.effective_governance.allowed_paths.join(", ")
        }
    ));
    md.push_str(&format!(
        "- Forbidden paths: {}\n",
        if bundle.effective_governance.forbidden_paths.is_empty() {
            "(none)".to_string()
        } else {
            bundle.effective_governance.forbidden_paths.join(", ")
        }
    ));
    md.push_str(&format!(
        "- Max files: {}\n",
        bundle
            .effective_governance
            .max_files_changed
            .map(|n| n.to_string())
            .unwrap_or_else(|| "(unlimited)".to_string())
    ));
    md.push_str(&format!(
        "- Max lines: {}\n",
        bundle
            .effective_governance
            .max_lines_changed
            .map(|n| n.to_string())
            .unwrap_or_else(|| "(unlimited)".to_string())
    ));

    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_machine_terminal_states() {
        assert!(EvaluationState::ReviewGate.is_terminal());
        assert!(EvaluationState::PreflightBlocked.is_terminal());
        assert!(EvaluationState::GenerationFailed.is_terminal());
        assert!(EvaluationState::GovernanceRejected.is_terminal());
        assert!(EvaluationState::CandidateCompileFailed.is_terminal());
        assert!(EvaluationState::CandidateTestFailed.is_terminal());
        assert!(EvaluationState::ValidationFailed.is_terminal());
        assert!(EvaluationState::InfraBlocked.is_terminal());
        assert!(EvaluationState::IntegrityFailed.is_terminal());
        assert!(EvaluationState::InternalError.is_terminal());
        assert!(!EvaluationState::Created.is_terminal());
        assert!(!EvaluationState::PreflightPassed.is_terminal());
        assert!(!EvaluationState::Generating.is_terminal());
        assert!(!EvaluationState::ProposalGenerated.is_terminal());
        assert!(!EvaluationState::GovernancePassed.is_terminal());
        assert!(!EvaluationState::Validating.is_terminal());
        assert!(!EvaluationState::ValidationComplete.is_terminal());
        assert!(!EvaluationState::IntegrityVerified.is_terminal());
    }

    #[test]
    fn outcome_labels() {
        assert_eq!(
            EvaluationState::ReviewGate.outcome_label(),
            "REVIEW_REQUIRED"
        );
        assert_eq!(
            EvaluationState::PreflightBlocked.outcome_label(),
            "PREFLIGHT_BLOCKED"
        );
        assert_eq!(
            EvaluationState::GenerationFailed.outcome_label(),
            "GENERATION_FAILED"
        );
    }

    #[test]
    fn classify_generation_error_infra() {
        assert_eq!(classify_generation_error("disk full"), "infra_blocked");
        assert_eq!(
            classify_generation_error("credential not found"),
            "infra_blocked"
        );
        assert_eq!(
            classify_generation_error("network timeout"),
            "infra_blocked"
        );
    }

    #[test]
    fn classify_generation_error_not_infra() {
        assert_eq!(
            classify_generation_error("provider returned no edits"),
            "generation_failed"
        );
    }

    #[test]
    fn classify_dry_run_error_compile() {
        assert_eq!(
            classify_dry_run_error("compiler error"),
            "candidate_compile_failed"
        );
    }

    #[test]
    fn classify_dry_run_error_infra() {
        assert_eq!(
            classify_dry_run_error("disk full during worktree"),
            "infra_blocked"
        );
    }

    #[test]
    fn classify_validation_failure_compile_error() {
        let vr = ValidationRecord {
            validation_command: None,
            exit_code: Some(1),
            stdout_preview: String::new(),
            stderr_preview: "error[E0308]: could not compile".to_string(),
            start_time: String::new(),
            completion_time: String::new(),
            test_discovered: false,
            test_executed: false,
            test_names_found: Vec::new(),
            test_count: 0,
            warnings: Vec::new(),
            failures: Vec::new(),
            patch_applies_cleanly: true,
            validation_passed: false,
        };
        assert_eq!(classify_validation_failure(&vr), "candidate_compile_failed");
    }

    #[test]
    fn classify_validation_failure_not_infra() {
        let vr = ValidationRecord {
            validation_command: None,
            exit_code: Some(1),
            stdout_preview: String::new(),
            stderr_preview: "assertion `left == right` failed".to_string(),
            start_time: String::new(),
            completion_time: String::new(),
            test_discovered: false,
            test_executed: false,
            test_names_found: Vec::new(),
            test_count: 0,
            warnings: Vec::new(),
            failures: vec!["assertion failed".to_string()],
            patch_applies_cleanly: true,
            validation_passed: false,
        };
        assert_ne!(classify_validation_failure(&vr), "infra_blocked");
    }

    #[test]
    fn parse_test_evidence_rust_output() {
        let stdout = "running 1\ntest tests::it_works ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored";
        let (discovered, executed, names, count, _warnings, _failures) =
            parse_test_evidence(stdout, "", Some(0));
        assert!(discovered);
        assert!(executed);
        assert!(names.contains(&"tests::it_works".to_string()));
        assert_eq!(count, 1);
    }

    #[test]
    fn parse_test_evidence_no_tests() {
        let (discovered, _executed, names, count, _, _) =
            parse_test_evidence("no tests here", "", Some(0));
        assert!(!discovered);
        assert!(!names.is_empty() || !discovered);
        assert_eq!(count, 0);
    }

    #[test]
    fn truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long() {
        let s = "a".repeat(100);
        let t = truncate(&s, 10);
        assert!(t.len() < 100);
        assert!(t.contains("truncated"));
    }

    #[test]
    fn test_extract_test_names() {
        let output = "test foo::bar ... ok\ntest baz::qux ... FAILED";
        let names = extract_test_names(output);
        assert!(names.contains(&"foo::bar".to_string()));
        assert!(names.contains(&"baz::qux".to_string()));
    }

    #[test]
    fn test_count_tests_from_output() {
        let output = "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out";
        assert_eq!(count_tests_from_output(output), 5);
    }

    #[test]
    fn failure_to_terminal_state_mapping() {
        assert_eq!(
            failure_to_terminal_state("infra_blocked"),
            EvaluationState::InfraBlocked
        );
        assert_eq!(
            failure_to_terminal_state("candidate_compile_failed"),
            EvaluationState::CandidateCompileFailed
        );
        assert_eq!(
            failure_to_terminal_state("validation_passed_review_required"),
            EvaluationState::ReviewGate
        );
    }
}
