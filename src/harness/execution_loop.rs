use crate::harness::{
    acceptance::{AcceptanceCriterion, compile_acceptance_criteria},
    artifacts::{HarnessArtifact, generate_completion_artifact},
    completion::{CompletionDecision, CompletionEvidence, evaluate_completion},
    confidence::{ConfidenceScore, compute_confidence},
    edit_protocol::EditOperation,
    environment::{EnvironmentProfile, fingerprint_environment},
    failure::{FailureKind, classify_patch_failure, classify_validation_failure},
    file_control::{FilePolicy, FileSet, build_file_set},
    git_checkpoint::{GitCheckpoint, create_pre_task_checkpoint},
    patch_applier::{PatchResult, apply_patch, dry_run_patch},
    repo_intelligence::{RepoContext, build_repo_context},
    review::{ReviewIssue, ReviewSeverity, review_diff},
    risk::{RiskAssessment, RiskLevel, assess_risk},
    sandbox::LocalSandboxRuntime,
    semantic_diff::analyze_semantic_diff,
    trajectory::Trajectory,
    validation::{ValidationPlan, ValidationResult, run_validation},
    verification::{VerificationStrength, assess_verification_strength},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Instant};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HarnessExecutionRequest {
    pub work_context_id: String,
    pub repo_root: PathBuf,
    pub task: String,
    pub requirements: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub mode: HarnessMode,
    pub limits: HarnessLimits,
    #[serde(default)]
    pub mentioned_files: Vec<PathBuf>,
    #[serde(default)]
    pub mentioned_symbols: Vec<String>,
    #[serde(default)]
    pub proposed_edits: Vec<EditOperation>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HarnessMode {
    Review,
    Autonomous,
    Benchmark,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct HarnessLimits {
    pub max_steps: u32,
    pub max_time_ms: u64,
    pub max_cost_usd: f64,
    pub max_patch_attempts: u32,
}
impl Default for HarnessLimits {
    fn default() -> Self {
        Self {
            max_steps: 20,
            max_time_ms: 300000,
            max_cost_usd: 1.0,
            max_patch_attempts: 2,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessExecutionResult {
    pub work_context_id: String,
    pub repo_context: RepoContext,
    pub environment: EnvironmentProfile,
    pub file_set: FileSet,
    pub acceptance: Vec<AcceptanceCriterion>,
    pub patch_result: Option<PatchResult>,
    pub validation_result: Option<ValidationResult>,
    pub review_issues: Vec<ReviewIssue>,
    pub risk_assessment: RiskAssessment,
    pub confidence: ConfidenceScore,
    pub verification_strength: VerificationStrength,
    pub completion_decision: CompletionDecision,
    pub trajectory: Trajectory,
    pub git_checkpoint: Option<GitCheckpoint>,
    pub artifacts: Vec<HarnessArtifact>,
    pub failures: Vec<FailureKind>,
    pub summary: String,
}
pub async fn execute_harness_task(req: HarnessExecutionRequest) -> Result<HarnessExecutionResult> {
    let started = Instant::now();
    let mut traj = Trajectory::new(req.work_context_id.clone());
    let env = fingerprint_environment(&req.repo_root).await?;
    let repo = build_repo_context(
        &req.repo_root,
        &req.task,
        &req.mentioned_files,
        &req.mentioned_symbols,
        8000,
    )
    .await?;
    let policy = FilePolicy::default_for_repo(req.repo_root.canonicalize()?);
    let files = build_file_set(&repo, &req.mentioned_files, &policy)?;
    let acceptance = compile_acceptance_criteria(if req.acceptance_criteria.is_empty() {
        &req.requirements
    } else {
        &req.acceptance_criteria
    });
    if req.proposed_edits.is_empty() {
        traj.record_step(
            "patch.generate",
            0,
            vec!["no structured edits supplied".into()],
        );
        traj.complete();
        return Ok(HarnessExecutionResult {
            work_context_id: req.work_context_id,
            repo_context: repo,
            environment: env,
            file_set: files,
            acceptance,
            patch_result: None,
            validation_result: None,
            review_issues: vec![],
            risk_assessment: RiskAssessment {
                level: RiskLevel::Low,
                reasons: vec![],
                requires_approval: false,
            },
            confidence: ConfidenceScore {
                score: 0.0,
                factors: vec!["no provider edits supplied".into()],
            },
            verification_strength: VerificationStrength::None,
            completion_decision: CompletionDecision::Blocked("no structured edits supplied".into()),
            trajectory: traj,
            git_checkpoint: None,
            artifacts: vec![],
            failures: vec![FailureKind::ModelFailure],
            summary: "Harness blocked before patching.".into(),
        });
    }
    let dry = dry_run_patch(&req.proposed_edits, &files, &policy)
        .await
        .context("patch dry-run failed")?;
    let mut failures = dry
        .failures
        .iter()
        .map(classify_patch_failure)
        .collect::<Vec<_>>();
    let checkpoint = create_pre_task_checkpoint(&req.repo_root).await.ok();
    let patch = if dry.failures.is_empty() {
        Some(apply_patch(&req.proposed_edits, &files, &policy).await?)
    } else {
        Some(dry)
    };
    let plan = ValidationPlan {
        format_commands: env.format_commands.clone(),
        lint_commands: vec![],
        test_commands: env.test_commands.clone(),
        repro_commands: vec![],
    };
    let validation = if patch.as_ref().is_some_and(|p| p.failures.is_empty()) {
        Some(run_validation(&req.repo_root, &plan, &LocalSandboxRuntime::default()).await?)
    } else {
        None
    };
    if let Some(v) = &validation {
        if !v.passed {
            failures.push(classify_validation_failure(v))
        }
    }
    let diff = patch.as_ref().map(|p| p.diff.as_str()).unwrap_or("");
    let review = review_diff(diff);
    let risk = assess_risk(&analyze_semantic_diff(diff), &review);
    let strength = assess_verification_strength(validation.as_ref());
    let confidence = compute_confidence(validation.as_ref(), &review, &risk, strength);
    let evidence = CompletionEvidence {
        patch_exists: patch
            .as_ref()
            .is_some_and(|p| !p.diff.is_empty() && p.failures.is_empty()),
        validation_ran: validation.is_some(),
        validation_passed: validation.as_ref().is_some_and(|v| v.passed),
        review_ran: true,
        critical_issues: review
            .iter()
            .filter(|i| i.severity == ReviewSeverity::Critical)
            .count(),
        confidence: confidence.clone(),
        verification_strength: strength,
        requires_approval: risk.requires_approval,
    };
    let decision = evaluate_completion(&evidence, req.mode)?;
    traj.record_step(
        "completion.evaluate",
        started.elapsed().as_millis() as u64,
        vec![],
    );
    traj.complete();
    let mut result = HarnessExecutionResult {
        work_context_id: req.work_context_id,
        repo_context: repo,
        environment: env,
        file_set: files,
        acceptance,
        patch_result: patch,
        validation_result: validation,
        review_issues: review,
        risk_assessment: risk,
        confidence,
        verification_strength: strength,
        completion_decision: decision,
        trajectory: traj,
        git_checkpoint: checkpoint,
        artifacts: vec![],
        failures,
        summary: "Harness run completed.".into(),
    };
    let content = generate_completion_artifact(&result)?;
    result.artifacts.push(HarnessArtifact {
        kind: "completion".into(),
        path: None,
        content: Some(content),
        metadata: serde_json::json!({}),
    });
    Ok(result)
}
