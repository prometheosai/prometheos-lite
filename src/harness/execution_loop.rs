use crate::harness::{
    acceptance::{AcceptanceCriterion, compile_acceptance_criteria},
    artifacts::{
        ArtifactKind, ArtifactMetadata, CompressionType, HarnessArtifact,
        generate_completion_artifact,
    },
    completion::{
        CompletionDecision, CompletionEvidence, ConfidenceEvidence, PatchEvidence, ProcessEvidence,
        ReviewEvidence, RiskEvidence, SemanticEvidence, ValidationEvidence, VerificationEvidence,
        evaluate_completion,
    },
    confidence::{ConfidenceFactor, ConfidenceScore, FactorImpact, compute_confidence},
    edit_protocol::EditOperation,
    environment::{EnvironmentProfile, fingerprint_environment},
    failure::{FailureKind, classify_patch_failure, classify_validation_failure},
    file_control::{FilePolicy, FileSet, build_file_set},
    git_checkpoint::{GitCheckpoint, create_pre_task_checkpoint},
    patch_applier::{
        PatchResult, RollbackHandle, apply_patch, apply_patch_with_rollback, dry_run_patch,
    },
    patch_provider::{
        GenerateRequest as ProviderGenerateRequest, PatchCandidate as ProviderCandidate,
        PatchProvider, PatchProviderContext, RiskEstimate,
    },
    repo_intelligence::{RepoContext, build_repo_context},
    review::{ReviewIssue, ReviewIssueType, ReviewSeverity, review_diff},
    risk::{RiskAssessment, RiskCategory, RiskLevel, RiskReason, RiskSeverity, assess_risk},
    sandbox::LocalSandboxRuntime,
    selection::{
        CandidateOutcome, PatchCandidate as SelectionCandidate, SelectionCriteria, SelectionEngine,
    },
    semantic_diff::analyze_semantic_diff,
    trajectory::Trajectory,
    validation::{ValidationPlan, ValidationResult, run_validation},
    verification::{VerificationStrength, assess_verification_strength},
};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

#[derive(Serialize, Deserialize)]
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
    /// Optional patch provider for generating/repairing edits
    #[serde(skip)]
    pub patch_provider: Option<Box<dyn crate::harness::patch_provider::PatchProvider>>,
    /// Context for patch provider (task description, repo map, etc.)
    pub provider_context: Option<crate::harness::patch_provider::PatchProviderContext>,
    /// Optional progress callback
    #[serde(skip)]
    pub progress_callback: Option<Box<dyn Fn(HarnessProgress) + Send + Sync>>,
    #[serde(default = "default_validation_failure_policy")]
    pub validation_failure_policy: ValidationFailurePolicy,
}

fn default_validation_failure_policy() -> ValidationFailurePolicy {
    ValidationFailurePolicy::RollbackAutomatically
}

impl std::fmt::Debug for HarnessExecutionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HarnessExecutionRequest")
            .field("work_context_id", &self.work_context_id)
            .field("repo_root", &self.repo_root)
            .field("task", &self.task)
            .field("requirements", &self.requirements)
            .field("acceptance_criteria", &self.acceptance_criteria)
            .field("mode", &self.mode)
            .field("limits", &self.limits)
            .field("mentioned_files", &self.mentioned_files)
            .field("mentioned_symbols", &self.mentioned_symbols)
            .field("proposed_edits", &self.proposed_edits)
            .field("progress_callback", &"<callback>")
            .finish()
    }
}

impl Clone for HarnessExecutionRequest {
    fn clone(&self) -> Self {
        Self {
            work_context_id: self.work_context_id.clone(),
            repo_root: self.repo_root.clone(),
            task: self.task.clone(),
            requirements: self.requirements.clone(),
            acceptance_criteria: self.acceptance_criteria.clone(),
            mode: self.mode,
            limits: self.limits,
            mentioned_files: self.mentioned_files.clone(),
            mentioned_symbols: self.mentioned_symbols.clone(),
            proposed_edits: self.proposed_edits.clone(),
            patch_provider: None, // Cannot clone trait object
            provider_context: self.provider_context.clone(),
            validation_failure_policy: self.validation_failure_policy,
            progress_callback: None, // Cannot clone trait object
        }
    }
}

impl PartialEq for HarnessExecutionRequest {
    fn eq(&self, other: &Self) -> bool {
        self.work_context_id == other.work_context_id
            && self.repo_root == other.repo_root
            && self.task == other.task
            && self.requirements == other.requirements
            && self.acceptance_criteria == other.acceptance_criteria
            && self.mode == other.mode
            && self.limits == other.limits
            && self.mentioned_files == other.mentioned_files
            && self.mentioned_symbols == other.mentioned_symbols
            && self.proposed_edits == other.proposed_edits
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HarnessMode {
    Review,
    ReviewOnly,
    Assisted,
    Autonomous,
    Benchmark,
}

/// Policy for handling validation failures after patch application
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ValidationFailurePolicy {
    /// Keep the patch and request manual approval
    KeepPatchAndRequestApproval,
    /// Automatically rollback the patch
    #[default]
    RollbackAutomatically,
    /// Rollback only on critical failures
    RollbackOnCriticalFailure,
    /// Never rollback (manual intervention required)
    NeverRollback,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct HarnessLimits {
    pub max_steps: u32,
    pub max_time_ms: u64,
    pub max_cost_usd: f64,
    pub max_patch_attempts: u32,
    pub max_tokens: Option<u64>,
    pub max_file_size_bytes: Option<u64>,
}

impl Default for HarnessLimits {
    fn default() -> Self {
        Self {
            max_steps: 20,
            max_time_ms: 300000,
            max_cost_usd: 1.0,
            max_patch_attempts: 2,
            max_tokens: None,
            max_file_size_bytes: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessExecutionResult {
    pub work_context_id: String,
    /// OpenTelemetry trace ID for distributed tracing
    pub trace_id: Option<String>,
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
    /// Rollback handle for undoing the patch if needed
    #[serde(skip)]
    pub rollback_handle: Option<RollbackHandle>,
    /// Policy for handling validation failures
    pub validation_failure_policy: ValidationFailurePolicy,
    pub artifacts: Vec<HarnessArtifact>,
    pub failures: Vec<FailureKind>,
    pub summary: String,
    pub execution_metrics: ExecutionMetrics,
    pub step_count: u32,
    pub terminated_early: bool,
    pub termination_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecutionMetrics {
    pub total_duration_ms: u64,
    pub repo_analysis_ms: u64,
    pub patch_generation_ms: u64,
    pub validation_ms: u64,
    pub review_ms: u64,
    pub cost_estimate_usd: f64,
    pub tokens_used: u64,
    pub files_modified: usize,
    pub lines_changed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HarnessProgress {
    Started {
        work_context_id: String,
        step: u32,
        total_steps: u32,
    },
    RepoAnalysis {
        files_found: usize,
        symbols_found: usize,
    },
    EnvironmentDetected {
        languages: Vec<String>,
        package_manager: Option<String>,
    },
    Patching {
        files_to_modify: usize,
        dry_run: bool,
    },
    PatchResult {
        success: bool,
        files_changed: usize,
        failures: usize,
    },
    GeneratingPatch,
    PatchGenerated {
        files_changed: usize,
        total_files: usize,
    },
    Validating {
        commands_to_run: usize,
    },
    ValidationResult {
        passed: bool,
        tests_run: usize,
        tests_passed: usize,
    },
    Reviewing {
        issues_found: usize,
        max_severity: Option<String>,
    },
    RiskAssessment {
        level: String,
        requires_approval: bool,
    },
    Completing {
        decision: String,
        confidence: f32,
    },
    Finished {
        success: bool,
        duration_ms: u64,
    },
    StepLimitReached {
        step: u32,
        max_steps: u32,
    },
    TimeLimitReached {
        elapsed_ms: u64,
        max_ms: u64,
    },
    Error {
        step: String,
        error: String,
    },
    RollingBack {
        reason: String,
    },
    RolledBack {
        restored_files: usize,
        deleted_files: usize,
        recreated_files: usize,
    },
    RollbackFailed {
        error: String,
    },
}

#[derive(Debug)]
struct ExecutionContext {
    limits: HarnessLimits,
    start_time: Instant,
    step_count: u32,
    cost_accrued: f64,
    tokens_used: u64,
    progress_sender: Option<mpsc::UnboundedSender<HarnessProgress>>,
}

impl ExecutionContext {
    fn new(limits: HarnessLimits) -> (Self, mpsc::UnboundedReceiver<HarnessProgress>) {
        let (tx, rx) = mpsc::unbounded_channel();

        let ctx = Self {
            limits,
            start_time: Instant::now(),
            step_count: 0,
            cost_accrued: 0.0,
            tokens_used: 0,
            progress_sender: Some(tx),
        };

        (ctx, rx)
    }

    fn check_limits(&self) -> Result<(), String> {
        let elapsed = self.start_time.elapsed().as_millis() as u64;
        if elapsed > self.limits.max_time_ms {
            return Err(format!(
                "Time limit reached: {}ms > {}ms",
                elapsed, self.limits.max_time_ms
            ));
        }

        if self.step_count >= self.limits.max_steps {
            return Err(format!(
                "Step limit reached: {} >= {}",
                self.step_count, self.limits.max_steps
            ));
        }

        if self.cost_accrued >= self.limits.max_cost_usd {
            return Err(format!(
                "Cost limit reached: ${:.4} >= ${:.4}",
                self.cost_accrued, self.limits.max_cost_usd
            ));
        }

        Ok(())
    }

    fn increment_step(&mut self) {
        self.step_count += 1;
    }

    fn add_cost(&mut self, cost: f64) {
        self.cost_accrued += cost;
    }

    fn add_tokens(&mut self, tokens: u64) {
        self.tokens_used += tokens;
    }

    fn send_progress(&self, progress: HarnessProgress) {
        if let Some(ref sender) = self.progress_sender {
            let _ = sender.send(progress);
        }
    }

    fn elapsed_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }
}

pub async fn execute_harness_task(
    mut req: HarnessExecutionRequest,
) -> Result<HarnessExecutionResult> {
    // Extract callback first using take() to avoid partial move
    let progress_callback = req.progress_callback.take();
    let (mut ctx, mut progress_rx) = ExecutionContext::new(req.limits);
    let started = Instant::now();

    // Spawn progress forwarding task if callback is provided
    let _progress_handle = if let Some(callback) = progress_callback {
        Some(tokio::spawn(async move {
            while let Some(progress) = progress_rx.recv().await {
                callback(progress);
            }
        }))
    } else {
        // Drain the channel to prevent blocking
        tokio::spawn(async move {
            while let Some(_progress) = progress_rx.recv().await {
                // Drop progress if no callback is registered
            }
        });
        None
    };

    ctx.send_progress(HarnessProgress::Started {
        work_context_id: req.work_context_id.clone(),
        step: 0,
        total_steps: req.limits.max_steps,
    });

    ctx.increment_step();
    if let Err(reason) = ctx.check_limits() {
        return create_terminated_result(&req, started.elapsed().as_millis() as u64, &reason, &ctx);
    }

    let mut traj = Trajectory::new(req.work_context_id.clone());
    let mut metrics = ExecutionMetrics::default();

    let repo_start = Instant::now();
    let repo = build_repo_context(
        &req.repo_root,
        &req.task,
        &req.mentioned_files,
        &req.mentioned_symbols,
        8000,
    )
    .await?;
    metrics.repo_analysis_ms = repo_start.elapsed().as_millis() as u64;

    ctx.send_progress(HarnessProgress::RepoAnalysis {
        files_found: repo.ranked_files.len(),
        symbols_found: repo.symbols.len(),
    });

    ctx.increment_step();
    if let Err(reason) = ctx.check_limits() {
        return create_terminated_result(&req, started.elapsed().as_millis() as u64, &reason, &ctx);
    }

    let env_start = Instant::now();
    let env = fingerprint_environment(&req.repo_root).await?;
    metrics.repo_analysis_ms += env_start.elapsed().as_millis() as u64;

    ctx.send_progress(HarnessProgress::EnvironmentDetected {
        languages: env.languages.clone(),
        package_manager: env.package_manager.clone(),
    });

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
            ctx.elapsed_ms(),
            vec!["no structured edits supplied".into()],
        );
        traj.complete();

        ctx.send_progress(HarnessProgress::Error {
            step: "patch.generate".into(),
            error: "No structured edits supplied".into(),
        });

        return Ok(HarnessExecutionResult {
            work_context_id: req.work_context_id,
            trace_id: Some(crate::harness::observability::otel::generate_trace_id()),
            repo_context: repo,
            environment: env,
            file_set: files,
            acceptance,
            patch_result: None,
            validation_result: None,
            review_issues: vec![],
            risk_assessment: RiskAssessment {
                level: RiskLevel::Low,
                reasons: vec![RiskReason {
                    category: RiskCategory::Logic,
                    description: "No edits proposed".into(),
                    severity: RiskSeverity::Info,
                    mitigation: None,
                }],
                requires_approval: false,
                can_override: true,
                override_conditions: vec!["manual review".into()],
            },
            confidence: ConfidenceScore {
                score: 0.0,
                factors: vec![ConfidenceFactor {
                    name: "edits_supplied".into(),
                    weight: 1.0,
                    score: 0.0,
                    description: "no provider edits supplied".into(),
                    impact: FactorImpact::Negative,
                }],
                explanation: "No edits were supplied for evaluation".into(),
                recommendation: Some("Provide structured edits for processing".into()),
            },
            verification_strength: VerificationStrength::None,
            completion_decision: CompletionDecision::Blocked("no structured edits supplied".into()),
            trajectory: traj,
            git_checkpoint: None,
            artifacts: vec![],
            failures: vec![FailureKind::ModelFailure],
            summary: "Harness blocked before patching: no edits supplied.".into(),
            execution_metrics: metrics,
            step_count: ctx.step_count,
            terminated_early: false,
            termination_reason: None,
        });
    }

    ctx.increment_step();
    if let Err(reason) = ctx.check_limits() {
        return create_terminated_result(&req, started.elapsed().as_millis() as u64, &reason, &ctx);
    }

    ctx.send_progress(HarnessProgress::Patching {
        files_to_modify: req.proposed_edits.len(),
        dry_run: true,
    });

    let patch_start = Instant::now();

    // STEP 1: Dry-run patch to verify it applies cleanly
    let dry = dry_run_patch(&req.proposed_edits, &files, &policy)
        .await
        .context("patch dry-run failed")?;

    let dry_failures: Vec<FailureKind> = dry.failures.iter().map(classify_patch_failure).collect();

    ctx.send_progress(HarnessProgress::PatchResult {
        success: dry.failures.is_empty(),
        files_changed: dry.changed_files.len(),
        failures: dry.failures.len(),
    });

    ctx.increment_step();
    if let Err(reason) = ctx.check_limits() {
        return create_terminated_result(&req, started.elapsed().as_millis() as u64, &reason, &ctx);
    }

    // STEP 2: Generate diff for review and semantic analysis
    let diff = generate_diff_from_edits(&req.proposed_edits);

    // STEP 3: Review the patch BEFORE applying
    let review_issues = if dry.failures.is_empty() {
        review_diff(&diff)
    } else {
        vec![] // No review performed if dry-run failed
    };

    // Compute critical issue count for later use
    let critical_count = review_issues
        .iter()
        .filter(|i| i.severity == ReviewSeverity::Critical)
        .count();

    // STEP 4: Semantic diff analysis
    let semantic = analyze_semantic_diff(&diff);

    // STEP 5: Risk assessment BEFORE applying
    let risk = assess_risk(&semantic, &review_issues);

    // STEP 6: Approval gate - determine if patch should be applied
    let should_apply = match req.mode {
        HarnessMode::ReviewOnly => {
            // ReviewOnly mode: NEVER apply patches
            false
        }
        HarnessMode::Assisted => {
            // Assisted mode: apply only if dry-run passed and no critical issues
            let has_critical_issues = review_issues
                .iter()
                .any(|i| matches!(i.severity, ReviewSeverity::Critical));
            let high_risk = matches!(risk.level, RiskLevel::High | RiskLevel::Critical);

            dry.failures.is_empty() && !has_critical_issues && !high_risk
        }
        HarnessMode::Autonomous => {
            // Autonomous mode: apply if dry-run passed and risk is acceptable
            let unacceptable_risk = matches!(risk.level, RiskLevel::Critical);
            dry.failures.is_empty() && !unacceptable_risk
        }
        HarnessMode::Benchmark => {
            // Benchmark mode: apply if dry-run passed
            dry.failures.is_empty()
        }
        _ => {
            // Default: require approval for any patch
            dry.failures.is_empty()
        }
    };

    // STEP 7: Create git checkpoint ONLY if we're going to apply
    let checkpoint = if should_apply {
        create_pre_task_checkpoint(&req.repo_root).await.ok()
    } else {
        None
    };

    // STEP 7.5: Generate and select best patch if provider available
    let selected_edits = if req.proposed_edits.is_empty() {
        // Try to generate candidates using patch provider
        if let Some(ref provider) = req.patch_provider {
            let provider_req = ProviderGenerateRequest {
                context: req.provider_context.clone().unwrap_or_default(),
                preferred_strategies: vec!["search_replace".into(), "whole_file".into()],
            };

            match provider.generate(provider_req).await {
                Ok(response) if !response.candidates.is_empty() => {
                    ctx.send_progress(HarnessProgress::GeneratingPatch);

                    // Convert provider candidates to selection candidates
                    let selection_candidates: Vec<SelectionCandidate> = response
                        .candidates
                        .iter()
                        .map(|c| SelectionCandidate {
                            id: format!("candidate_{}", c.source),
                            edits: c.edits.clone(),
                            source: c.source.clone(),
                            confidence: c.confidence as f64 / 100.0,
                            metadata: Default::default(),
                        })
                        .collect();

                    // Use SelectionEngine to pick the best
                    let selection_engine = SelectionEngine::new(SelectionCriteria::default());
                    let outcome = selection_engine.select_best_patch(&selection_candidates);

                    ctx.send_progress(HarnessProgress::PatchGenerated {
                        files_changed: outcome
                            .selected
                            .as_ref()
                            .map(|s| s.edits.len())
                            .unwrap_or(0),
                        total_files: files.files.len(),
                    });

                    outcome.selected.map(|s| s.edits).unwrap_or_default()
                }
                _ => {
                    // No candidates generated, use empty
                    Vec::new()
                }
            }
        } else {
            req.proposed_edits.clone()
        }
    } else {
        req.proposed_edits.clone()
    };

    // STEP 8: Apply patch only if approved - with rollback support
    let (patch, rollback_handle) =
        if should_apply && dry.failures.is_empty() && !selected_edits.is_empty() {
            let (result, handle) =
                apply_patch_with_rollback(&selected_edits, &files, &policy).await?;
            (Some(result), Some(handle))
        } else {
            // Return dry-run result (patch not actually applied)
            (Some(dry.clone()), None)
        };
    let dry_failures = dry.failures.clone();

    metrics.patch_generation_ms = patch_start.elapsed().as_millis() as u64;
    metrics.files_modified = patch.as_ref().map(|p| p.changed_files.len()).unwrap_or(0);

    if let Some(ref p) = patch {
        metrics.lines_changed = p.diff.lines().count();
    }

    ctx.increment_step();
    if let Err(reason) = ctx.check_limits() {
        return create_terminated_result(&req, started.elapsed().as_millis() as u64, &reason, &ctx);
    }

    let plan = ValidationPlan {
        format_commands: env.format_commands.clone(),
        lint_commands: env.lint_commands.clone(),
        test_commands: env.test_commands.clone(),
        repro_commands: vec![],
        timeout_ms: Some(120000),
        parallel: true,
    };

    let validation = if patch.as_ref().is_some_and(|p| p.failures.is_empty()) {
        ctx.send_progress(HarnessProgress::Validating {
            commands_to_run: plan.format_commands.len()
                + plan.lint_commands.len()
                + plan.test_commands.len(),
        });

        let val_start = Instant::now();
        let result = run_validation(&req.repo_root, &plan, &LocalSandboxRuntime::default()).await?;
        metrics.validation_ms = val_start.elapsed().as_millis() as u64;

        let tests_run = result.command_results.len();
        let tests_passed = result
            .command_results
            .iter()
            .filter(|r| r.exit_code == Some(0))
            .count();

        ctx.send_progress(HarnessProgress::ValidationResult {
            passed: result.passed,
            tests_run,
            tests_passed,
        });

        Some(result)
    } else {
        None
    };

    let mut failures: Vec<FailureKind> = dry_failures
        .iter()
        .map(|_f| FailureKind::PatchApplyFailure)
        .collect();
    if let Some(ref v) = validation {
        if !v.passed {
            failures.push(classify_validation_failure(v));
        }
    }

    // STEP 9: Handle validation failure rollback policy
    if let Some(ref v) = validation {
        if !v.passed && rollback_handle.is_some() {
            let should_rollback = match req.validation_failure_policy {
                ValidationFailurePolicy::RollbackAutomatically => {
                    ctx.send_progress(HarnessProgress::RollingBack {
                        reason: "validation failed - automatic rollback".into(),
                    });
                    true
                }
                ValidationFailurePolicy::RollbackOnCriticalFailure => {
                    let has_critical = v
                        .errors
                        .iter()
                        .any(|e| e.contains("critical") || e.contains("fatal"));
                    if has_critical {
                        ctx.send_progress(HarnessProgress::RollingBack {
                            reason: "critical validation failure - automatic rollback".into(),
                        });
                    }
                    has_critical
                }
                _ => false,
            };

            if should_rollback {
                if let Some(handle) = rollback_handle {
                    match handle.rollback().await {
                        Ok(result) => {
                            failures.push(FailureKind::ValidationFailed);
                            failures.push(FailureKind::PatchRolledBack);
                            ctx.send_progress(HarnessProgress::RolledBack {
                                restored_files: result.restored.len(),
                                deleted_files: result.deleted.len(),
                                recreated_files: result.recreated.len(),
                            });
                        }
                        Err(e) => {
                            failures.push(FailureKind::ValidationFailed);
                            failures.push(FailureKind::RollbackFailed);
                            ctx.send_progress(HarnessProgress::RollbackFailed {
                                error: e.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    ctx.increment_step();
    if let Err(reason) = ctx.check_limits() {
        return create_terminated_result(&req, started.elapsed().as_millis() as u64, &reason, &ctx);
    }

    ctx.increment_step();
    if let Err(reason) = ctx.check_limits() {
        return create_terminated_result(&req, started.elapsed().as_millis() as u64, &reason, &ctx);
    }

    // Build completion evidence using the review_issues from pre-apply phase
    let strength = assess_verification_strength(validation.as_ref());
    let confidence = compute_confidence(validation.as_ref(), &review_issues, &risk, strength);

    // Count breaking changes - API changes have `breaking` field, dependency changes use risk_level
    let breaking_api_changes = semantic.api_changes.iter().filter(|c| c.breaking).count();
    let breaking_dep_changes = semantic
        .dependency_changes
        .iter()
        .filter(|c| {
            matches!(
                c.risk_level,
                crate::harness::semantic_diff::RiskLevel::High
                    | crate::harness::semantic_diff::RiskLevel::Critical
            )
        })
        .count();

    let evidence = CompletionEvidence {
        // 8 Evidence Dimensions
        patch_evidence: PatchEvidence {
            patch_created: patch.as_ref().is_some_and(|p| !p.diff.is_empty()),
            files_modified: patch.as_ref().map(|p| p.changed_files.len()).unwrap_or(0),
            lines_changed: patch.as_ref().map(|p| p.diff.lines().count()).unwrap_or(0),
            patch_applied_cleanly: patch.as_ref().is_some_and(|p| p.failures.is_empty()),
            patch_hash: patch
                .as_ref()
                .map(|p| format!("{:x}", md5::compute(&p.diff))),
            dry_run_passed: dry.failures.is_empty(),
        },
        validation_evidence: ValidationEvidence {
            validation_performed: validation.is_some(),
            all_validations_passed: validation.as_ref().is_some_and(|v| v.passed),
            format_check_passed: validation.as_ref().map(|v| v.passed).unwrap_or(false),
            static_check_passed: validation.as_ref().map(|v| v.passed).unwrap_or(false),
            lint_check_passed: validation.as_ref().map(|v| v.passed).unwrap_or(false),
            test_passed: validation.as_ref().map(|v| v.passed).unwrap_or(false),
            validation_summary: validation
                .as_ref()
                .map(|v| format!("{} commands run", v.command_results.len()))
                .unwrap_or_default(),
        },
        review_evidence: ReviewEvidence {
            review_performed: !review_issues.is_empty() || dry.failures.is_empty(),
            total_issues: review_issues.len(),
            critical_issues: review_issues
                .iter()
                .filter(|i| i.severity == ReviewSeverity::Critical)
                .count(),
            high_issues: review_issues
                .iter()
                .filter(|i| i.severity == ReviewSeverity::High)
                .count(),
            medium_issues: review_issues
                .iter()
                .filter(|i| i.severity == ReviewSeverity::Medium)
                .count(),
            low_issues: review_issues
                .iter()
                .filter(|i| i.severity == ReviewSeverity::Low)
                .count(),
            security_issues: review_issues
                .iter()
                .filter(|i| i.issue_type == ReviewIssueType::Security)
                .count(),
            breaking_change_issues: review_issues
                .iter()
                .filter(|i| i.issue_type == ReviewIssueType::ApiChange)
                .count(),
            review_passed: !review_issues
                .iter()
                .any(|i| i.severity == ReviewSeverity::Critical),
        },
        risk_evidence: RiskEvidence {
            risk_assessed: true,
            overall_risk_level: format!("{:?}", risk.level),
            security_risk: format!("{:?}", risk.level),
            api_risk: format!("{:?}", risk.level),
            database_risk: format!("{:?}", risk.level),
            dependency_risk: format!("{:?}", risk.level),
            requires_approval: risk.requires_approval,
            risk_reasons: risk.reasons.iter().map(|r| r.description.clone()).collect(),
        },
        verification_evidence: VerificationEvidence {
            verification_level: strength,
            test_count: validation
                .as_ref()
                .map(|v| v.command_results.len())
                .unwrap_or(0),
            coverage_percent: None,
            reproduction_test_passed: false,
            integration_tests_passed: false,
            verification_summary: format!("Verification strength: {:?}", strength),
        },
        semantic_evidence: SemanticEvidence {
            api_changes_detected: !semantic.api_changes.is_empty(),
            auth_changes_detected: !semantic.auth_changes.is_empty(),
            database_changes_detected: !semantic.database_changes.is_empty(),
            dependency_changes_detected: !semantic.dependency_changes.is_empty(),
            config_changes_detected: !semantic.config_changes.is_empty(),
            breaking_changes_count: breaking_api_changes + breaking_dep_changes,
            security_relevant_changes: !semantic.auth_changes.is_empty()
                || semantic.api_changes.iter().any(|c| {
                    c.change_type == crate::harness::semantic_diff::ApiChangeType::VisibilityChanged
                }),
        },
        confidence_evidence: ConfidenceEvidence {
            confidence_score: confidence.score,
            confidence_classification: format!("score-{:.2}", confidence.score),
            validation_contribution: 0.4,
            risk_contribution: 0.3,
            review_contribution: 0.3,
            confidence_factors: confidence.factors.iter().map(|f| f.name.clone()).collect(),
        },
        process_evidence: ProcessEvidence {
            git_checkpoint_created: checkpoint.is_some(),
            rollback_available: checkpoint.is_some(),
            all_phases_completed: true,
            no_critical_errors: failures.is_empty(),
            time_limit_respected: true,
            step_limit_respected: true,
        },

        // Legacy fields
        patch_exists: patch
            .as_ref()
            .is_some_and(|p| !p.diff.is_empty() && p.failures.is_empty()),
        validation_ran: validation.is_some(),
        validation_passed: validation.as_ref().is_some_and(|v| v.passed),
        review_ran: true,
        critical_issues: critical_count,
        confidence: confidence.clone(),
        verification_strength: strength,
        requires_approval: risk.requires_approval,

        // Decision metadata
        decision_factors: vec!["harness execution completed".into()],
        evidence_completeness: 1.0,
    };

    let decision = evaluate_completion(&evidence, req.mode)?;

    ctx.send_progress(HarnessProgress::Completing {
        decision: format!("{:?}", decision),
        confidence: confidence.score,
    });

    traj.record_step("completion.evaluate", ctx.elapsed_ms(), vec![]);
    traj.complete();

    metrics.total_duration_ms = started.elapsed().as_millis() as u64;

    let trace_id = crate::harness::observability::otel::generate_trace_id();

    let mut result = HarnessExecutionResult {
        work_context_id: req.work_context_id,
        trace_id: Some(trace_id.clone()),
        repo_context: repo,
        environment: env,
        file_set: files,
        acceptance,
        patch_result: patch,
        validation_result: validation,
        review_issues,
        risk_assessment: risk,
        confidence,
        verification_strength: strength,
        completion_decision: decision,
        trajectory: traj,
        git_checkpoint: checkpoint,
        rollback_handle,
        validation_failure_policy: req.validation_failure_policy,
        artifacts: vec![],
        failures,
        summary: "Harness run completed.".into(),
        execution_metrics: metrics,
        step_count: ctx.step_count,
        terminated_early: false,
        termination_reason: None,
    };

    let content = generate_completion_artifact(&result)?;
    result.artifacts.push(HarnessArtifact {
        id: format!("artifact-{}", result.work_context_id),
        kind: ArtifactKind::Report,
        path: None,
        content: Some(content.clone()),
        compressed_content: None,
        compression: CompressionType::None,
        metadata: ArtifactMetadata {
            work_context_id: result.work_context_id.clone(),
            harness_run_id: result.work_context_id.clone(),
            tags: vec!["completion".into()],
            custom_fields: HashMap::new(),
        },
        created_at: Utc::now(),
        size_bytes: content.len(),
        compressed_size_bytes: None,
    });

    ctx.send_progress(HarnessProgress::Finished {
        success: true,
        duration_ms: ctx.elapsed_ms(),
    });

    Ok(result)
}

fn create_terminated_result(
    req: &HarnessExecutionRequest,
    elapsed_ms: u64,
    reason: &str,
    ctx: &ExecutionContext,
) -> Result<HarnessExecutionResult> {
    let progress = if reason.contains("Time") {
        HarnessProgress::TimeLimitReached {
            elapsed_ms,
            max_ms: req.limits.max_time_ms,
        }
    } else {
        HarnessProgress::StepLimitReached {
            step: ctx.step_count,
            max_steps: req.limits.max_steps,
        }
    };

    ctx.send_progress(progress);

    bail!("Harness execution terminated: {}", reason)
}

pub async fn execute_harness_task_with_progress<F>(
    req: HarnessExecutionRequest,
    mut progress_callback: F,
) -> Result<HarnessExecutionResult>
where
    F: FnMut(HarnessProgress) + Send + 'static,
{
    let (ctx, mut rx) = ExecutionContext::new(req.limits);

    let handle = tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            progress_callback(progress);
        }
    });

    let result = execute_harness_task(req).await;

    drop(ctx);
    handle.abort();

    result
}

pub fn estimate_execution_cost(limits: &HarnessLimits, estimated_files: usize) -> f64 {
    let base_cost = 0.01;
    let per_file_cost = 0.005;
    let time_cost = limits.max_time_ms as f64 / 1000.0 / 60.0 * 0.05;

    base_cost + (per_file_cost * estimated_files as f64) + time_cost
}

pub fn check_resource_limits(
    limits: &HarnessLimits,
    files_to_process: &[PathBuf],
) -> Result<(), String> {
    if let Some(max_size) = limits.max_file_size_bytes {
        for file in files_to_process {
            if let Ok(metadata) = std::fs::metadata(file) {
                if metadata.len() > max_size {
                    return Err(format!(
                        "File {} exceeds size limit: {} bytes > {} bytes",
                        file.display(),
                        metadata.len(),
                        max_size
                    ));
                }
            }
        }
    }

    if limits.max_steps < 5 {
        return Err("Minimum 5 steps required for safe execution".into());
    }

    if limits.max_time_ms < 10000 {
        return Err("Minimum 10 seconds required for safe execution".into());
    }

    Ok(())
}

/// Generate a unified diff representation from edit operations for review purposes
fn generate_diff_from_edits(edits: &[EditOperation]) -> String {
    use std::fmt::Write;

    let mut diff_output = String::new();

    for edit in edits {
        match edit {
            EditOperation::SearchReplace(sr) => {
                let _ = writeln!(diff_output, "--- a/{}", sr.file.display());
                let _ = writeln!(diff_output, "+++ b/{}", sr.file.display());
                let search_lines: Vec<_> = sr.search.lines().collect();
                let replace_lines: Vec<_> = sr.replace.lines().collect();
                let _ = writeln!(
                    diff_output,
                    "@@ -1,{} +1,{} @@",
                    search_lines.len(),
                    replace_lines.len()
                );
                for line in &search_lines {
                    let _ = writeln!(diff_output, "-{}", line);
                }
                for line in &replace_lines {
                    let _ = writeln!(diff_output, "+{}", line);
                }
            }
            EditOperation::UnifiedDiff(ud) => {
                if let Some(ref file) = ud.target_file {
                    let _ = writeln!(diff_output, "--- a/{}", file.display());
                    let _ = writeln!(diff_output, "+++ b/{}", file.display());
                }
                let _ = writeln!(diff_output, "{}", ud.diff);
            }
            EditOperation::WholeFile(wf) => {
                let _ = writeln!(diff_output, "--- a/{}", wf.file.display());
                let _ = writeln!(diff_output, "+++ b/{}", wf.file.display());
                let _ = writeln!(diff_output, "@@ -1,1 +1,{} @@", wf.content.lines().count());
                for line in wf.content.lines() {
                    let _ = writeln!(diff_output, "+{}", line);
                }
            }
            EditOperation::CreateFile(cf) => {
                let _ = writeln!(diff_output, "--- /dev/null");
                let _ = writeln!(diff_output, "+++ b/{}", cf.file.display());
                let _ = writeln!(diff_output, "@@ -0,0 +1,{} @@", cf.content.lines().count());
                for line in cf.content.lines() {
                    let _ = writeln!(diff_output, "+{}", line);
                }
            }
            EditOperation::DeleteFile(df) => {
                let _ = writeln!(diff_output, "--- a/{}", df.file.display());
                let _ = writeln!(diff_output, "+++ /dev/null");
                let _ = writeln!(diff_output, "@@ File deleted @@");
            }
            EditOperation::RenameFile(rf) => {
                let _ = writeln!(diff_output, "--- a/{}", rf.from.display());
                let _ = writeln!(diff_output, "+++ b/{}", rf.to.display());
                let _ = writeln!(diff_output, "@@ File renamed @@");
            }
        }
        let _ = writeln!(diff_output);
    }

    diff_output
}
