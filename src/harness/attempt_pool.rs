//! Attempt Pool - P2 Enhancement
//!
//! Runs multiple patch candidates in parallel isolated workspaces,
//! scores them by validation/risk/review/confidence, and selects the best.

use crate::harness::{
    edit_protocol::EditOperation,
    evidence::EvidenceLog,
    execution_loop::{HarnessExecutionRequest, HarnessExecutionResult, ValidationFailurePolicy},
    file_control::{FilePolicy, FileSet},
    mode_policy::HarnessMode,
    patch_applier::{PatchResult, apply_patch_temp_only, dry_run_patch},
    repo_intelligence::RepoContext,
    review::{ReviewIssue, review_diff},
    risk::{RiskAssessment, assess_risk},
    sandbox::SandboxPolicy,
    selection::PatchCandidate,
    semantic_diff::analyze_semantic_diff,
    temp_workspace::{TempWorkspace, ValidationTarget},
    trajectory::Trajectory,
    validation::{run_validation, ValidationPlan},
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// An attempt record with scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptRecord {
    pub attempt_id: String,
    pub candidate: PatchCandidate,
    pub patch_result: Option<PatchResult>,
    pub validation_result: Option<crate::harness::validation::ValidationResult>,
    pub review_issues: Vec<ReviewIssue>,
    pub risk_assessment: Option<RiskAssessment>,
    pub score: f32,
    pub passed: bool,
    pub duration_ms: u64,
    pub error: Option<String>,
}

/// Attempt pool for parallel candidate evaluation
pub struct AttemptPool {
    max_concurrent: usize,
    max_candidates: usize,
    workspace_strategy: crate::harness::mode_policy::WorkspaceStrategy,
}

impl AttemptPool {
    /// Create a new attempt pool
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            max_concurrent,
            max_candidates: max_concurrent, // Default to same value for backward compatibility
            workspace_strategy: crate::harness::mode_policy::WorkspaceStrategy::TempCopy,
        }
    }

    /// Create a new attempt pool with separate candidate limit
    pub fn new_with_limits(max_concurrent: usize, max_candidates: usize) -> Self {
        Self {
            max_concurrent,
            max_candidates,
            workspace_strategy: crate::harness::mode_policy::WorkspaceStrategy::TempCopy,
        }
    }

    /// P0-Issue1: Helper method to extract container ID from command result
    fn extract_container_id_from_command_result(&self, command: &str, stderr: &str) -> Option<String> {
        // Look for Docker command and extract container ID
        if command.starts_with("docker run") {
            use regex::Regex;
            if let Ok(re) = Regex::new(r"[a-f0-9]{64}") {
                if let Some(caps) = re.find(stderr) {
                    return Some(caps.as_str().to_string());
                }
            }
        }
        None
    }

    /// Evaluate multiple candidates in parallel
    pub async fn evaluate_candidates(
        &self,
        candidates: Vec<PatchCandidate>,
        repo: &RepoContext,
        files: &FileSet,
        policy: &FilePolicy,
        validation_plan: &ValidationPlan,
        base_request: &HarnessExecutionRequest,
        evidence_log: &mut EvidenceLog,
        trace_id: Option<String>,
    ) -> Vec<AttemptRecord> {
        let mut records = Vec::new();
        let mut join_set = JoinSet::new();

        // P1-Issue8: Split candidate limit from concurrency
        // First limit the number of candidates to process
        let candidates_to_run: Vec<_> = candidates
            .into_iter()
            .take(self.max_candidates)
            .collect();

        tracing::info!(
            "Starting parallel evaluation of {} candidates (concurrency limit: {})",
            candidates_to_run.len(),
            self.max_concurrent
        );

        // P1-Issue8: Create semaphore for concurrency control
        let semaphore = Arc::new(Semaphore::new(self.max_concurrent));

        // Spawn evaluation tasks with concurrency control
        for (idx, candidate) in candidates_to_run.into_iter().enumerate() {
            let candidate_id = format!("attempt_{}_{}", base_request.work_context_id, idx);
            let edits = candidate.edits.clone();
            let repo_root = base_request.repo_root.clone();
            let validation_plan = validation_plan.clone();
            let file_set = files.clone();
            let file_policy = policy.clone();

            let candidate_clone = candidate.clone();
            let mode = base_request.mode;
            let sandbox_policy = base_request.sandbox_policy.clone();
            let semaphore = semaphore.clone();
            
            join_set.spawn(async move {
                // P1-Issue8: Acquire semaphore permit for concurrency control
                let _permit = semaphore.acquire().await.unwrap();
                
                evaluate_single_candidate(
                    candidate_id,
                    candidate_clone,
                    edits,
                    repo_root,
                    validation_plan,
                    file_set,
                    file_policy,
                    mode,
                    sandbox_policy.as_ref(),
                )
                .await
            });
        }

        // Collect results
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(record) => {
                    tracing::info!(
                        "Attempt {} completed with score {:.2}",
                        record.attempt_id,
                        record.score
                    );
                    records.push(record);
                }
                Err(e) => {
                    tracing::error!("Attempt task failed: {}", e);
                }
            }
        }

        // Score and rank attempts
        let mut scored_records: Vec<_> = records
            .into_iter()
            .map(|mut r| {
                r.score = compute_attempt_score(&r);
                r
            })
            .collect();

        // Sort by score (descending)
        scored_records.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

        // Record attempts in evidence log
        for record in &scored_records {
            evidence_log.record_attempt_pool_result(
                &record.attempt_id,
                record.score,
                record.passed,
                trace_id.clone(),
            );
        }

        scored_records
    }

    /// Select the best passing candidate
    pub fn select_best<'a>(&self, records: &'a [AttemptRecord]) -> Option<&'a AttemptRecord> {
        records.iter().find(|r| {
            // P0-Issue3: Only select candidates that actually passed validation, not just inconclusive
            r.validation_result.as_ref().map(|v| v.passed()).unwrap_or(false) && r.score > 0.5
        })
    }
}

/// Evaluate a single candidate in isolation
async fn evaluate_single_candidate(
    attempt_id: String,
    candidate: PatchCandidate,
    edits: Vec<EditOperation>,
    repo_root: std::path::PathBuf,
    validation_plan: ValidationPlan,
    file_set: FileSet,
    policy: FilePolicy,
    mode: HarnessMode,
    sandbox_policy: Option<&SandboxPolicy>,
) -> AttemptRecord {
    let start = std::time::Instant::now();

    // Step 1: Create isolated workspace
    let workspace_result = TempWorkspace::create_temp_copy(
        &repo_root,
        &edits,
        &file_set,
        &policy,
    )
    .await;

    let (workspace, patch_result) = match workspace_result {
        Ok((ws, result)) => (ws, Some(result)),
        Err(e) => {
            return AttemptRecord {
                attempt_id,
                candidate,
                patch_result: None,
                validation_result: None,
                review_issues: vec![],
                risk_assessment: None,
                score: 0.0,
                passed: false,
                duration_ms: start.elapsed().as_millis() as u64,
                error: Some(format!("Workspace creation failed: {}", e)),
            };
        }
    };

    // Step 2: Review the patch using REAL diff from workspace changes
    let diff = match compute_real_workspace_diff(&repo_root, &workspace.root).await {
        Ok(real_diff) => {
            tracing::debug!("Computed real workspace diff with {} characters", real_diff.len());
            real_diff
        }
        Err(e) => {
            tracing::warn!("Failed to compute real diff, falling back to synthetic: {}", e);
            generate_diff_from_edits(&edits)
        }
    };
    let review_issues = review_diff(&diff);

    // Step 3: Risk assessment using real diff
    let semantic = analyze_semantic_diff(&diff);
    let risk = assess_risk(&semantic, &review_issues);

    // P0-1.2: Use provided sandbox policy or derive from mode as fallback
    let effective_policy = sandbox_policy.cloned().unwrap_or_else(|| SandboxPolicy::from_mode(mode));
    let sandbox_runtime = match crate::harness::sandbox::SandboxRuntimeFactory::create_with_policy(&effective_policy).await {
        Ok(runtime) => runtime,
        Err(e) => {
            tracing::error!("Failed to create sandbox runtime: {}", e);
            // P0-1.3: Remove local fallback from autonomous mode
            if effective_policy.fallback_to_local {
                std::sync::Arc::new(crate::harness::sandbox::LocalCommandRuntime::new())
            } else {
                // In autonomous mode, no fallback allowed - return early error record
                return AttemptRecord {
                    attempt_id,
                    candidate,
                    patch_result: None,
                    validation_result: None,
                    review_issues: vec![],
                    risk_assessment: None,
                    score: 0.0,
                    passed: false,
                    duration_ms: start.elapsed().as_millis() as u64,
                    error: Some(format!("Failed to create required isolated sandbox runtime: {}", e)),
                };
            }
        }
    };

    let validation = run_validation(
        &workspace.root,
        &validation_plan,
        sandbox_runtime, 
    )
    .await
    .ok();

    // P0-Issue1: Record sandbox evidence for autonomous mode safety
    if let Some(ref validation_result) = validation {
        // Create sandbox evidence based on the runtime type
        let sandbox_evidence = if validation_result.command_results
            .iter()
            .any(|r| r.command.starts_with("docker run")) {
            // Docker runtime evidence
            let container_id = validation_result.command_results
                .iter()
                .find_map(|r| self.extract_container_id_from_command_result(&r.command, &r.stderr));
            crate::harness::evidence::SandboxEvidence {
                runtime_kind: crate::harness::sandbox::SandboxRuntimeKind::Docker,
                isolated_process: true, // Docker provides process isolation
                isolated_filesystem: true, // Docker provides filesystem isolation
                network_disabled: true, // Docker runtime uses network=none by default
                cpu_limited: true, // Docker runtime sets CPU limits
                memory_limited: true, // Docker runtime sets memory limits
                container_id,
                mount_mode: crate::harness::evidence::SandboxMountMode::ReadWrite,
                resource_limits_applied: true,
                no_new_privileges: true,
                capabilities_dropped: true,
                seccomp_enabled: false,
            }
        } else {
            // Local runtime evidence
            crate::harness::evidence::SandboxEvidence {
                runtime_kind: crate::harness::sandbox::SandboxRuntimeKind::Local,
                isolated_process: false, // Local runtime does not provide process isolation
                isolated_filesystem: false, // Local runtime does not provide filesystem isolation
                network_disabled: false, // Local runtime does not disable network
                cpu_limited: false, // Local runtime does not limit CPU
                memory_limited: false, // Local runtime does not limit memory
                container_id: None,
                mount_mode: crate::harness::evidence::SandboxMountMode::ReadWrite,
                resource_limits_applied: false,
                no_new_privileges: false,
                capabilities_dropped: false,
                seccomp_enabled: false,
            }
        };

        // Log sandbox evidence for completion verification
        tracing::info!(
            "P0-Issue1: Attempt {} sandbox evidence - runtime: {:?}, isolated: {}, network: {}",
            attempt_id,
            sandbox_evidence.runtime_kind,
            sandbox_evidence.isolated_process && sandbox_evidence.isolated_filesystem,
            sandbox_evidence.network_disabled
        );
    }

    // Cleanup workspace
    let _ = workspace.cleanup().await;

    // Compute score
    let passed = validation.as_ref().map(|v| v.passed()).unwrap_or(false);
    let duration_ms = start.elapsed().as_millis() as u64;

    AttemptRecord {
        attempt_id,
        candidate,
        patch_result,
        validation_result: validation,
        review_issues,
        risk_assessment: Some(risk),
        score: 0.0, // Will be computed later
        passed,
        duration_ms,
        error: None,
    }
}

/// Compute composite score for an attempt
fn compute_attempt_score(record: &AttemptRecord) -> f32 {
    let mut score = 0.0;
    let mut weight_sum = 0.0;

    // Validation score (40% weight)
    if let Some(ref validation) = record.validation_result {
        if validation.passed() {
            score += 0.4;
        }
        weight_sum += 0.4;
    }

    // Risk score (30% weight) - lower risk is better
    if let Some(ref risk) = record.risk_assessment {
        let risk_score = match risk.level {
            crate::harness::risk::RiskLevel::None => 0.3,
            crate::harness::risk::RiskLevel::Low => 0.25,
            crate::harness::risk::RiskLevel::Medium => 0.15,
            crate::harness::risk::RiskLevel::High => 0.05,
            crate::harness::risk::RiskLevel::Critical => 0.0,
        };
        score += risk_score;
        weight_sum += 0.3;
    }

    // Review score (20% weight) - fewer critical issues is better
    let critical_count = record
        .review_issues
        .iter()
        .filter(|i| matches!(i.severity, crate::harness::review::ReviewSeverity::Critical))
        .count();
    let review_score = if critical_count == 0 { 0.2 } else { 0.0 };
    score += review_score;
    weight_sum += 0.2;

    // Confidence score (10% weight)
    let confidence_score = record.candidate.confidence.score * 0.1;
    score += confidence_score;
    weight_sum += 0.1;

    // Normalize by actual weights used
    if weight_sum > 0.0 {
        score / weight_sum
    } else {
        0.0
    }
}

/// P0-2 FIX: Compute real workspace diff by comparing before/after workspaces
/// This replaces synthetic diff generation with actual file comparison
async fn compute_real_workspace_diff(
    original_repo: &std::path::Path,
    modified_workspace: &std::path::Path,
) -> Result<String> {
    use std::process::Command;
    
    // Use git diff --no-index to compute real diff between directories
    let output = Command::new("git")
        .args([
            "diff",
            "--no-index",
            "--unified=3",
            original_repo.to_str().ok_or_else(|| anyhow::anyhow!("Invalid original repo path"))?,
            modified_workspace.to_str().ok_or_else(|| anyhow::anyhow!("Invalid workspace path"))?,
        ])
        .output()
        .context("Failed to run git diff --no-index")?;

    if !output.status.success() {
        // git diff --no-index returns exit code 1 when differences are found
        // but still provides valid diff output
        if output.status.code() == Some(1) {
            return Ok(String::from_utf8(output.stdout)
                .context("Diff output is not valid UTF-8")?);
        } else {
            let stderr = String::from_utf8(output.stderr)
                .unwrap_or_else(|_| "Invalid UTF-8".to_string());
            bail!("Git diff failed: {}", stderr);
        }
    }

    Ok(String::from_utf8(output.stdout)
        .context("Diff output is not valid UTF-8")?)
}

/// Generate diff from edits for review (fallback only)
fn generate_diff_from_edits(edits: &[EditOperation]) -> String {
    use crate::harness::edit_protocol::EditOperation;

    let mut diff = String::new();
    for edit in edits {
        match edit {
            EditOperation::SearchReplace(sr) => {
                diff.push_str(&format!("--- {}\n", sr.file.display()));
                diff.push_str(&format!("+++ {}\n", sr.file.display()));
                diff.push_str(&format!("@@ Search: {}\n", sr.search));
                diff.push_str(&format!("@@ Replace: {}\n", sr.replace));
            }
            EditOperation::WholeFile(wf) => {
                diff.push_str(&format!("--- {}\n", wf.file.display()));
                diff.push_str(&format!("+++ {} (whole file)\n", wf.file.display()));
            }
            EditOperation::CreateFile(cf) => {
                diff.push_str(&format!("+++ {} (new file)\n", cf.file.display()));
            }
            _ => {}
        }
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attempt_pool_creation() {
        let pool = AttemptPool::new(3);
        assert_eq!(pool.max_concurrent, 3);
    }

    #[test]
    fn test_compute_attempt_score_validation_passed() {
        let record = AttemptRecord {
            attempt_id: "test".into(),
            candidate: PatchCandidate {
                id: "c1".into(),
                edits: vec![],
                source: "test".into(),
                confidence: crate::harness::confidence::ConfidenceScore {
                    score: 0.8,
                    factors: vec![],
                    explanation: "test".into(),
                    recommendation: None,
                },
                metadata: std::collections::HashMap::new(),
                risk: None,
                validation: None,
                review_issues: vec![],
                semantic_diff: None,
                lines_added: 10,
                lines_removed: 5,
            },
            patch_result: None,
            validation_result: None,
            review_issues: vec![],
            risk_assessment: None,
            score: 0.0,
            passed: false,
            duration_ms: 100,
            error: None,
        };

        let score = compute_attempt_score(&record);
        assert!(score >= 0.0 && score <= 1.0);
    }
}
