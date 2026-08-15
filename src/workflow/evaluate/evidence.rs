use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::identity::{ExecutionIdentity, GovernanceScopeSnapshot};
use super::integrity::git_rev_parse_head;

// ---------------------------------------------------------------------------
// Schema version
// ---------------------------------------------------------------------------

/// Current evidence-bundle schema version.
const SCHEMA_VERSION: &str = "1.0.0";
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
/// Create the evidence directory (and any parents) for a run.
pub(super) fn prepare_evidence_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("failed to create evidence dir: {}", path.display()))
}
/// Find an existing evidence bundle for a proposal.
pub(super) fn find_existing_evidence(
    evidence_dir: &Path,
    proposal_id: &str,
) -> Option<EvidenceBundle> {
    // Look for evidence.json in the evidence directory.
    let evidence_path = evidence_dir.join("evidence.json");
    if evidence_path.exists() {
        let text = std::fs::read_to_string(&evidence_path).ok()?;
        let bundle: EvidenceBundle = serde_json::from_str(&text).ok()?;
        if bundle.proposal.as_ref().map(|p| p.id.as_str()) == Some(proposal_id) {
            return Some(bundle);
        }
    }
    None
}
pub(super) fn new_bundle(
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

pub(super) fn new_bundle_from_identity(
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

pub(super) fn write_bundle(evidence_dir: &Path, bundle: &EvidenceBundle) -> Result<()> {
    // Fill repo_pin_after.
    let mut bundle = bundle.clone();
    if let Ok(head) = git_rev_parse_head(Path::new(&bundle.repo)) {
        bundle.repo_pin_after = head;
    }

    let json_path = evidence_dir.join("evidence.json");
    super::durable::atomic_write_json(&json_path, &bundle)
        .context("failed to write evidence.json")?;

    // Write Markdown report.
    let md_path = evidence_dir.join("evidence.md");
    let md = render_markdown_report(&bundle);
    std::fs::write(&md_path, &md).context("failed to write evidence.md")?;

    Ok(())
}

/// Persist the validation record durably, BEFORE the `ValidationComplete`
/// journal event that references it.
pub(super) fn write_validation_artifact(
    evidence_dir: &Path,
    validation: &ValidationRecord,
) -> Result<()> {
    let path = evidence_dir.join("validation.json");
    super::durable::atomic_write_json(&path, validation).context("failed to write validation.json")
}

/// Persist the integrity record durably, BEFORE the `IntegrityVerified`
/// journal event that references it.
pub(super) fn write_integrity_artifact(
    evidence_dir: &Path,
    integrity: &IntegrityRecord,
) -> Result<()> {
    let path = evidence_dir.join("integrity.json");
    super::durable::atomic_write_json(&path, integrity).context("failed to write integrity.json")
}

/// Read a previously persisted validation record (written by
/// [`write_validation_artifact`]). Used when resuming finalization after a
/// late cancellation: validation is durable and must NOT be re-run.
pub(super) fn read_validation_artifact(evidence_dir: &Path) -> Result<ValidationRecord> {
    let path = evidence_dir.join("validation.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read validation artifact {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("corrupt validation artifact {}", path.display()))
}

/// Read a previously persisted integrity record (written by
/// [`write_integrity_artifact`]). Used when resuming finalization after a
/// durable `IntegrityVerified` state: the result is authoritative and must NOT
/// be recomputed merely because another process resumed. A missing or corrupt
/// artifact fails closed rather than healing itself by re-running integrity.
pub(super) fn read_integrity_artifact(evidence_dir: &Path) -> Result<IntegrityRecord> {
    let path = evidence_dir.join("integrity.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read integrity artifact {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("corrupt integrity artifact {}", path.display()))
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
    use crate::workflow::evaluate::identity::EvaluationState;

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
    fn prepare_evidence_dir_creates_path() {
        let dir = tempfile::tempdir().unwrap();
        let evidence_dir = dir.path().join("a").join("b").join("evidence");
        prepare_evidence_dir(&evidence_dir).unwrap();
        assert!(evidence_dir.is_dir());
    }

    #[test]
    fn bundle_json_writes_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let identity = sample_identity();
        let bundle = new_bundle(&identity, "abc123", Path::new("/tmp/repo"), dir.path());
        write_bundle(dir.path(), &bundle).unwrap();

        let reloaded: EvidenceBundle = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("evidence.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(reloaded.run_id, "run-1");
        assert_eq!(reloaded.task_id, "task-1");
        assert_eq!(reloaded.final_state, "in_progress");
        assert_eq!(reloaded.schema_version, "1.0.0");
    }

    #[test]
    fn find_existing_evidence_matches_proposal_id() {
        let dir = tempfile::tempdir().unwrap();
        let identity = sample_identity();
        let mut bundle = new_bundle(&identity, "abc123", Path::new("/tmp/repo"), dir.path());
        bundle.proposal = Some(ProposalRecord {
            id: "proposal-1".to_string(),
            patch_hash: "deadbeef".to_string(),
            changed_files: vec!["src/main.rs".to_string()],
            added_lines: 1,
            removed_lines: 0,
            base_sha: "abc123".to_string(),
        });
        write_bundle(dir.path(), &bundle).unwrap();

        assert!(find_existing_evidence(dir.path(), "proposal-1").is_some());
        assert!(find_existing_evidence(dir.path(), "other").is_none());
    }

    #[test]
    fn markdown_report_contains_stable_sections() {
        let dir = tempfile::tempdir().unwrap();
        let identity = sample_identity();
        let bundle = new_bundle(&identity, "abc123", Path::new("/tmp/repo"), dir.path());
        let md = render_markdown_report(&bundle);
        assert!(md.contains("# Evaluation Evidence"));
        assert!(md.contains("**Schema:** `1.0.0`"));
        assert!(md.contains("## Outcome"));
        assert!(md.contains("## Governance"));
    }
}
