use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
/// Update the persisted identity's state, fail-closed.
///
/// Every failure (read, parse, serialize, atomic publish) is propagated to the
/// caller. The identity is never left half-written: publication goes through
/// [`super::durable::atomic_write_json`].
pub(super) fn update_identity_state(path: &Path, state: EvaluationState) -> Result<()> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read identity state file {}", path.display()))?;
    let mut identity: ExecutionIdentity = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse identity state file {}", path.display()))?;
    identity.state = state;
    super::durable::versioned_write_json(path, &identity)
        .with_context(|| format!("failed to persist identity state {}", path.display()))?;
    Ok(())
}

/// Read the current state from a persisted identity document.
pub(super) fn read_identity_state(path: &Path) -> Option<EvaluationState> {
    let text = std::fs::read_to_string(path).ok()?;
    let identity: ExecutionIdentity = serde_json::from_str(&text).ok()?;
    Some(identity.state)
}
/// Persist the execution identity to `execution_identity.json` in the
/// evidence directory, before any model call (the exactly-once gate).
///
/// Returns the identity path used by later state updates.
pub(super) fn persist_execution_identity(
    evidence_dir: &Path,
    identity: &ExecutionIdentity,
) -> Result<PathBuf> {
    let identity_path = evidence_dir.join("execution_identity.json");
    super::durable::versioned_write_json(&identity_path, identity)
        .context("failed to persist execution identity")?;
    Ok(identity_path)
}
pub(super) fn evidence_dir_for(repo: &Path, run_id: &str) -> PathBuf {
    repo.join(".prometheos").join("evidence").join(run_id)
}
pub fn now_iso() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    chrono::DateTime::from_timestamp(dur.as_secs() as i64, dur.subsec_nanos())
        .map(|d| d.to_rfc3339())
        .unwrap_or_else(|| dur.as_secs().to_string())
}
pub(super) fn hash_str(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_identity() -> ExecutionIdentity {
        ExecutionIdentity {
            run_id: "run-1".to_string(),
            task_id: "task-1".to_string(),
            repo: "/tmp/repo".to_string(),
            repo_pin: "abc123".to_string(),
            model: "mock".to_string(),
            provider: "mock".to_string(),
            governance_scope: GovernanceScopeSnapshot {
                allowed_paths: vec!["src/**".to_string()],
                forbidden_paths: vec![],
                allow_dependency_changes: false,
                max_files_changed: Some(5),
                max_lines_changed: None,
                authority: "propose".to_string(),
                validation_command: Some("cargo test".to_string()),
            },
            created_at: "2026-01-01T00:00:00Z".to_string(),
            state: EvaluationState::Created,
        }
    }

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
    fn persist_execution_identity_writes_round_trippable_file() {
        let dir = tempfile::tempdir().unwrap();
        let identity = sample_identity();
        let path = persist_execution_identity(dir.path(), &identity).unwrap();

        assert_eq!(
            path.file_name().unwrap(),
            std::ffi::OsStr::new("execution_identity.json")
        );
        assert!(path.exists());

        let stored: ExecutionIdentity =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(stored.run_id, "run-1");
        assert_eq!(stored.task_id, "task-1");
        assert_eq!(stored.repo, "/tmp/repo");
        assert_eq!(stored.state, EvaluationState::Created);
        assert_eq!(stored.governance_scope.authority, "propose");
    }
    #[test]
    fn update_identity_state_changes_only_state() {
        let dir = tempfile::tempdir().unwrap();
        let identity = sample_identity();
        let path = persist_execution_identity(dir.path(), &identity).unwrap();

        update_identity_state(&path, EvaluationState::PreflightPassed).unwrap();

        let stored: ExecutionIdentity =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(stored.state, EvaluationState::PreflightPassed);
        // All other fields remain unchanged.
        assert_eq!(stored.run_id, "run-1");
        assert_eq!(stored.task_id, "task-1");
        assert_eq!(stored.governance_scope.authority, "propose");
    }
}
