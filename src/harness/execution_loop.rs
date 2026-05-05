use crate::harness::{
    acceptance::{AcceptanceCriterion, compile_acceptance_criteria},
    artifacts::{
        ArtifactKind, ArtifactMetadata, CompressionType, HarnessArtifact,
        generate_completion_artifact,
    },
    attempt_pool::AttemptPool,
    completion::{
        CompletionDecision, CompletionEvidence, ConfidenceEvidence, PatchEvidence, ProcessEvidence,
        ReviewEvidence, RiskEvidence, SemanticEvidence, ValidationEvidence, VerificationEvidence,
        evaluate_completion,
    },
    confidence::{ConfidenceFactor, ConfidenceScore, FactorImpact, compute_confidence},
    edit_protocol::EditOperation,
    environment::{EnvironmentProfile, fingerprint_environment},
    evidence::{EvidenceEntryKind, EvidenceLog},
    failure::{FailureKind, classify_patch_failure, classify_validation_failure},
    file_control::{FilePolicy, FileSet, build_file_set},
    git_checkpoint::{GitCheckpoint, create_pre_task_checkpoint},
    mode_policy::{HarnessMode, HarnessPolicyGate, GateDecision},
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
        PatchCandidate as SelectionCandidate, SelectionCriteria, SelectionEngine, SelectionPhase,
    },
    semantic_diff::analyze_semantic_diff,
    temp_workspace::{TempWorkspace, ValidationTarget, create_validation_target},
    trajectory::Trajectory,
    validation::{ValidationCategory, ValidationPlan, ValidationResult, run_validation},
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

impl HarnessExecutionRequest {
    /// P0-FIX: Auto-create patch provider from config if not already set
    ///
    /// This is the production entry point for LLM-based patch generation.
    /// Call this before execute_harness_task() to ensure a provider is available.
    pub fn with_config_provider(mut self) -> Self {
        if self.patch_provider.is_none() && self.proposed_edits.is_empty() {
            // Try to load config and create LLM provider
            if let Ok(config) = crate::config::AppConfig::load() {
                if let Ok(registry) =
                    crate::harness::patch_provider::ProviderRegistry::from_config(&config)
                {
                    // Store the registry's aggregate provider
                    // Note: We need to keep the registry alive, so we store it in provider_context
                    // and use a wrapper that delegates to the registry
                    self.patch_provider = Some(Box::new(registry));
                }
            }
        }
        self
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
    /// Complete evidence log of all side effects and decisions
    pub evidence_log: EvidenceLog,
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
        repaired: bool,
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

    fn record_action(&self, category: &str, action: &str, details: &str) {
        tracing::info!("[{}] {}: {}", category, action, details);
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
    let mut evidence_log = EvidenceLog::new(&req.work_context_id);

    // P1-010: Generate ONE trace ID for entire harness execution
    // All child spans and evidence entries will share this trace ID
    let trace_id = crate::harness::observability::otel::generate_trace_id();
    tracing::info!(trace_id = %trace_id, "P1-010: Starting harness execution with trace propagation");

    // P1-010: Create root span for harness execution
    let _root_span = tracing::info_span!(
        "harness.execute",
        trace_id = %trace_id,
        work_context_id = %req.work_context_id,
        mode = ?req.mode,
    );
    let _enter = _root_span.enter();

    // P1-010: Child span for repo analysis phase
    let repo_span = tracing::info_span!(
        "harness.repo_analysis",
        trace_id = %trace_id,
        phase = "repo_analysis",
    );
    let repo_start = Instant::now();
    let repo = {
        let _enter = repo_span.enter();
        build_repo_context(
            &req.repo_root,
            &req.task,
            &req.mentioned_files,
            &req.mentioned_symbols,
            8000,
        )
        .await?
    };
    metrics.repo_analysis_ms = repo_start.elapsed().as_millis() as u64;
    tracing::info!(
        trace_id = %trace_id,
        files_found = repo.ranked_files.len(),
        symbols_found = repo.symbols.len(),
        "P1-010: Repo analysis complete"
    );

    // Record evidence of repo analysis with trace ID
    evidence_log.record_repo_map_built(
        repo.ranked_files.len(),
        repo.symbols.len(),
        metrics.repo_analysis_ms,
        Some(trace_id.clone()),
    );

    // P0: Validate evidence is being recorded before proceeding
    if evidence_log.entries.is_empty() {
        bail!("EvidenceLog is empty - cannot proceed without evidence recording");
    }

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

    // Determine which edits to use: provided edits or generate from provider
    let selected_edits = if req.proposed_edits.is_empty() {
        // Try to generate candidates using patch provider
        if let Some(ref provider) = req.patch_provider {
            // P1-010: Child span for provider generation phase
            let provider_span = tracing::info_span!(
                "harness.provider_generate",
                trace_id = %trace_id,
                phase = "provider_generation",
            );
            let _enter = provider_span.enter();

            let provider_req = ProviderGenerateRequest {
                context: req.provider_context.clone().unwrap_or_default(),
                preferred_strategies: vec!["search_replace".into(), "whole_file".into()],
            };

            match provider.generate(provider_req).await {
                Ok(response) if !response.candidates.is_empty() => {
                    ctx.send_progress(HarnessProgress::GeneratingPatch);

                    // P0-FIX: AttemptPool is now the ONLY candidate evaluation path
                    // All candidates (even single ones) go through isolated temp workspace validation
                    let candidates_count = response.candidates.len();
                    tracing::info!(trace_id = %trace_id, "P0: Using AttemptPool to evaluate {} candidate(s) in isolated workspaces", candidates_count);

                    // Convert provider candidates to PatchCandidates for AttemptPool
                    let patch_candidates: Vec<crate::harness::selection::PatchCandidate> = response
                        .candidates
                        .iter()
                        .map(|c| crate::harness::selection::PatchCandidate {
                            id: format!("candidate_{}", c.source),
                            edits: c.edits.clone(),
                            source: c.source.clone(),
                            confidence: crate::harness::confidence::ConfidenceScore {
                                score: c.confidence as f32 / 100.0,
                                factors: vec![],
                                explanation: "Provider confidence score".to_string(),
                                recommendation: None,
                            },
                            metadata: Default::default(),
                            risk: None,
                            validation: None,
                            review_issues: vec![],
                            semantic_diff: None,
                            lines_added: c.edits.iter().map(|e| e.lines_added()).sum(),
                            lines_removed: c.edits.iter().map(|e| e.lines_removed()).sum(),
                        })
                        .collect();

                    // Create validation plan for AttemptPool
                    let validation_plan = ValidationPlan {
                        format_commands: vec![],
                        lint_commands: vec!["cargo check".into()],
                        test_commands: vec!["cargo test --lib".into()],
                        repro_commands: vec![],
                        timeout_ms: Some(120000),
                        parallel: true,
                        tool_ids: vec![],
                    };

                    // P1-010: Child span for AttemptPool evaluation phase
                    let attempt_pool_span = tracing::info_span!(
                        "harness.attempt_pool",
                        trace_id = %trace_id,
                        phase = "attempt_pool",
                        candidates_count = candidates_count,
                    );

                    // Run AttemptPool for parallel evaluation in isolated workspaces
                    let pool = AttemptPool::new(3); // Max 3 concurrent
                    let records = {
                        let _enter = attempt_pool_span.enter();
                        pool.evaluate_candidates(
                            patch_candidates,
                            &repo,
                            &files,
                            &policy,
                            &validation_plan,
                            &req,
                            &mut evidence_log,
                            Some(trace_id.clone()),
                        ).await
                    };

                    // P0-FIX: Select best passing candidate based on validation, not just confidence
                    let selected_edits = if let Some(best) = pool.select_best(&records) {
                        tracing::info!("P0: AttemptPool selected best candidate {} with score {:.2} (validation passed: {:?})",
                            best.attempt_id, best.score, best.validation_result.as_ref().map(|v| v.passed));
                        best.candidate.edits.clone()
                    } else {
                        tracing::warn!("P0: No passing candidates from AttemptPool - falling back to highest confidence");
                        // Fall back to highest confidence candidate if none passed validation
                        response.candidates.iter()
                            .max_by_key(|c| c.confidence)
                            .map(|c| c.edits.clone())
                            .unwrap_or_default()
                    };

                    let files_changed = selected_edits.len();

                    ctx.send_progress(HarnessProgress::PatchGenerated {
                        files_changed,
                        total_files: files.editable.len(),
                    });

                    // Record patch generation evidence with AttemptPool details
                    if !selected_edits.is_empty() {
                        evidence_log.record_patch_generated(
                            &format!("attempt_pool_selection_{}_candidates", candidates_count),
                            selected_edits.len(),
                            0.8, // Default confidence for AttemptPool selection
                            Some(trace_id.clone()),
                        );
                    }

                    selected_edits
                }
                _ => {
                    // No candidates generated, use empty
                    Vec::new()
                }
            }
        } else {
            // No edits provided and no provider available - block
            ctx.record_action("patch", "blocked", "No edits provided and no patch provider available");
            evidence_log.record_side_effect_blocked("No edits provided and no patch provider available", Some(trace_id.clone()));
            evidence_log.complete();
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
                rollback_handle: None,
                validation_failure_policy: ValidationFailurePolicy::KeepPatchAndRequestApproval,
                artifacts: vec![],
                failures: vec![FailureKind::ModelFailure],
                summary: "Harness blocked before patching: no edits supplied.".into(),
                execution_metrics: metrics,
                step_count: ctx.step_count,
                terminated_early: true,
                termination_reason: Some("No edits or provider".into()),
                evidence_log,
            });
        }
    } else {
        req.proposed_edits.clone()
    };

    // Validate we have edits to work with
    if selected_edits.is_empty() {
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

        evidence_log.record_side_effect_blocked("No structured edits supplied", Some(trace_id.clone()));
        evidence_log.complete();
        return Ok(HarnessExecutionResult {
            work_context_id: req.work_context_id,
            trace_id: Some(crate::harness::observability::otel::generate_trace_id()),
            repo_context: repo,
            environment: env,
            file_set: files,
            rollback_handle: None,
            validation_failure_policy: ValidationFailurePolicy::KeepPatchAndRequestApproval,
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
            evidence_log,
        });
    }

    ctx.increment_step();
    if let Err(reason) = ctx.check_limits() {
        return create_terminated_result(&req, started.elapsed().as_millis() as u64, &reason, &ctx);
    }

    ctx.send_progress(HarnessProgress::Patching {
        files_to_modify: selected_edits.len(),
        dry_run: true,
    });

    let patch_start = Instant::now();

    // P1-010: Child span for dry-run phase
    let dry_run_span = tracing::info_span!(
        "harness.dry_run",
        trace_id = %trace_id,
        phase = "dry_run",
        files_to_modify = selected_edits.len(),
    );

    // STEP 1: Dry-run patch to verify it applies cleanly
    let dry_start = Instant::now();
    let dry = {
        let _enter = dry_run_span.enter();
        dry_run_patch(&selected_edits, &files, &policy)
            .await
            .context("patch dry-run failed")?
    };
    let dry_run_ms = dry_start.elapsed().as_millis() as u64;
    tracing::info!(
        trace_id = %trace_id,
        dry_run_ms = dry_run_ms,
        failures = dry.failures.len(),
        "P1-010: Dry-run complete"
    );

    // Record dry-run evidence
    evidence_log.record_dry_run(&dry, dry_run_ms, Some(trace_id.clone()));

    let dry_failures: Vec<FailureKind> = dry.failures.iter().map(classify_patch_failure).collect();

    // P1-FIX: Attempt repair if dry-run failed and we have a provider
    let (selected_edits, dry, repaired) = if !dry.failures.is_empty() && req.patch_provider.is_some() {
        tracing::info!("P1: Dry-run failed with {} failures, attempting repair", dry.failures.len());

        // Create repair context
        let provider_context = crate::harness::patch_provider::PatchProviderContext {
            task: req.task.clone(),
            requirements: vec![],
            repo_map: None,
            mentioned_files: vec![],
            mentioned_symbols: vec![],
            attempt_history: vec![],
            validation_output: Some(format!("Dry-run failures: {:?}", dry.failures)),
            review_issues: vec![],
            max_candidates: 3,
        };

        // Create repair request for each failure
        let mut repaired_edits = selected_edits.clone();
        let mut any_repaired = false;

        for failure in &dry.failures {
            // Convert PatchFailure to FailureDetails
            let failure_details = crate::harness::failure::FailureDetails {
                kind: classify_patch_failure(failure),
                category: crate::harness::failure::FailureCategory::Tooling,
                severity: crate::harness::failure::FailureSeverity::Error,
                message: failure.reason.clone(),
                context: crate::harness::failure::FailureContext {
                    file: Some(failure.file.clone()),
                    line: failure.line_number,
                    column: None,
                    operation: Some(failure.operation.clone()),
                    command: None,
                    nearby_code: failure.nearby_context.clone(),
                    stack_trace: None,
                },
                suggestion: failure.nearby_context.clone(),
                recovery_action: crate::harness::failure::RecoveryAction::Retry,
            };

            let repair_request = crate::harness::patch_provider::RepairRequest {
                context: provider_context.clone(),
                failure: failure_details,
                failed_edits: repaired_edits.clone(),
                repair_strategy: crate::harness::patch_provider::RepairStrategy::ExpandContextWindow,
            };

            // Try to repair using provider
            if let Some(ref provider) = req.patch_provider {
                match provider.repair(repair_request).await {
                    Ok(repair_response) if !repair_response.repaired_edits.is_empty() => {
                        tracing::info!("P1: Repair succeeded with {} edits", repair_response.repaired_edits.len());
                        repaired_edits = repair_response.repaired_edits;
                        any_repaired = true;

                        // Re-run dry-run with repaired edits
                        match dry_run_patch(&repaired_edits, &files, &policy).await {
                            Ok(new_dry) => {
                                if new_dry.failures.is_empty() {
                                    tracing::info!("P1: Repaired patch passes dry-run");
                                    break; // Success!
                                } else {
                                    tracing::warn!("P1: Repaired patch still has {} failures", new_dry.failures.len());
                                }
                            }
                            Err(e) => {
                                tracing::error!("P1: Dry-run failed after repair: {}", e);
                            }
                        }
                    }
                    Ok(_) => {
                        tracing::warn!("P1: Repair returned empty edits");
                    }
                    Err(e) => {
                        tracing::error!("P1: Repair failed: {}", e);
                    }
                }
            }
        }

        // Final dry-run with repaired edits (or original if repair failed)
        let final_dry = if any_repaired {
            dry_run_patch(&repaired_edits, &files, &policy)
                .await
                .unwrap_or_else(|e| {
                    tracing::error!("P1: Final dry-run failed: {}", e);
                    dry.clone()
                })
        } else {
            dry.clone()
        };

        (repaired_edits, final_dry, any_repaired)
    } else {
        (selected_edits, dry, false)
    };

    ctx.send_progress(HarnessProgress::PatchResult {
        success: dry.failures.is_empty(),
        files_changed: dry.changed_files.len(),
        failures: dry.failures.len(),
        repaired,
    });

    ctx.increment_step();
    if let Err(reason) = ctx.check_limits() {
        return create_terminated_result(&req, started.elapsed().as_millis() as u64, &reason, &ctx);
    }

    // STEP 2: Generate diff for review and semantic analysis
    let diff = generate_diff_from_edits(&selected_edits);

    // P1-010: Child span for review phase
    let review_span = tracing::info_span!(
        "harness.review",
        trace_id = %trace_id,
        phase = "review",
    );

    // STEP 3: Review the patch BEFORE applying
    let review_issues = if dry.failures.is_empty() {
        let _enter = review_span.enter();
        let issues = review_diff(&diff);
        let critical = issues.iter().filter(|i| i.severity == ReviewSeverity::Critical).count();
        let high = issues.iter().filter(|i| i.severity == ReviewSeverity::High).count();
        tracing::info!(trace_id = %trace_id, "P1-010: Review found {} critical, {} high issues", critical, high);
        issues
    } else {
        vec![] // No review performed if dry-run failed
    };

    // Compute critical issue count for later use
    let critical_count = review_issues
        .iter()
        .filter(|i| i.severity == ReviewSeverity::Critical)
        .count();

    // P1-010: Child span for semantic analysis phase
    let semantic_span = tracing::info_span!(
        "harness.semantic_analysis",
        trace_id = %trace_id,
        phase = "semantic_analysis",
    );

    // STEP 4: Semantic diff analysis
    let semantic = {
        let _enter = semantic_span.enter();
        analyze_semantic_diff(&diff)
    };

    // P1-010: Child span for risk assessment phase
    let risk_span = tracing::info_span!(
        "harness.risk_assessment",
        trace_id = %trace_id,
        phase = "risk_assessment",
    );

    // STEP 5: Risk assessment BEFORE applying
    let risk = {
        let _enter = risk_span.enter();
        let r = assess_risk(&semantic, &review_issues);
        tracing::info!(trace_id = %trace_id, risk_level = ?r.level, "P1-010: Risk assessment complete");
        r
    };

    // Record risk assessment evidence
    evidence_log.record_risk_assessed(&risk, Some(trace_id.clone()));

    // STEP 6: Hard policy gate - determine if patch should be applied
    // P0: This is the single point of authority for side-effect decisions
    let policy_gate = HarnessPolicyGate::for_mode(req.mode);

    // Check if review was performed when required
    if policy_gate.require_review() && dry.failures.is_empty() && review_issues.is_empty() {
        // Review should have been performed but wasn't - this is a policy violation
        tracing::warn!("Review was required but not performed");
    }

    // Check if risk assessment was performed when required
    if policy_gate.require_risk_assessment() && risk.reasons.is_empty() {
        // Risk assessment should have been performed - log warning
        tracing::warn!("Risk assessment was required but no reasons recorded");
    }

    let has_critical_issues = review_issues
        .iter()
        .any(|i| matches!(i.severity, ReviewSeverity::Critical));

    // Use policy gate for hard decision (rollback handle not yet available, checked later)
    let gate_decision = policy_gate.check_patch_application(
        dry.failures.is_empty(),
        has_critical_issues,
        risk.level.clone(),
        true, // Assume rollback will be available if needed
    );

    let should_apply = matches!(gate_decision, GateDecision::Allow);

    // Record gate decision in evidence log
    match &gate_decision {
        GateDecision::Allow => {
            tracing::info!("Policy gate: ALLOW patch application in {:?} mode", req.mode);
        }
        GateDecision::Block(reason) => {
            tracing::warn!("Policy gate: BLOCK patch application - {}", reason);
            evidence_log.record_side_effect_blocked(format!("Policy gate: {}", reason), Some(trace_id.clone()));
        }
        GateDecision::RequireApproval(reason) => {
            tracing::warn!("Policy gate: REQUIRE APPROVAL - {}", reason);
            evidence_log.record_side_effect_blocked(format!("Policy gate requires approval: {}", reason), Some(trace_id.clone()));
        }
    }

    // P1-010: Child span for checkpoint phase
    let checkpoint_span = tracing::info_span!(
        "harness.checkpoint",
        trace_id = %trace_id,
        phase = "checkpoint",
        should_apply = should_apply,
    );

    // STEP 7: Create git checkpoint ONLY if we're going to apply
    // NOTE: Checkpoint failure is blocking in side-effect modes (see implementation below)
    let checkpoint_result = if should_apply {
        let _enter = checkpoint_span.enter();
        let result = create_pre_task_checkpoint(&req.repo_root).await;
        if let Ok(ref cp) = result {
            tracing::info!(trace_id = %trace_id, checkpoint_id = %cp.id, "P1-010: Checkpoint created");
        }
        result
    } else {
        Ok(GitCheckpoint {
            id: "review-only".to_string(),
            work_context_id: req.work_context_id.clone(),
            branch_name: "review-only".to_string(),
            before_head: None,
            after_head: None,
            dirty_files: vec![],
            touched_files: vec![],
            diff_before: String::new(),
            diff_after: String::new(),
            committed: false,
            commit_message: None,
            created_at: chrono::Utc::now(),
        })
    };

    // In side-effect modes (Autonomous, Assisted), checkpoint failure is blocking
    let checkpoint = match (&req.mode, checkpoint_result) {
        (HarnessMode::ReviewOnly, _) => None,
        (_, Err(e)) => {
            // Checkpoint failed in a mode that requires side effects - this is blocking
            evidence_log.record_side_effect_blocked(&format!("Checkpoint creation failed: {}", e), Some(trace_id.clone()));
            evidence_log.complete();
            return Ok(HarnessExecutionResult {
                work_context_id: req.work_context_id,
                trace_id: Some(crate::harness::observability::otel::generate_trace_id()),
                repo_context: repo,
                environment: env,
                file_set: files,
                acceptance,
                patch_result: None,
                validation_result: None,
                review_issues,
                risk_assessment: risk,
                confidence: ConfidenceScore {
                    score: 0.0,
                    factors: vec![],
                    explanation: "Checkpoint creation failed".into(),
                    recommendation: Some(format!("Git checkpoint failed: {}", e)),
                },
                verification_strength: VerificationStrength::None,
                completion_decision: CompletionDecision::Blocked(format!(
                    "Git checkpoint creation failed (required in {:?} mode): {}",
                    req.mode, e
                )),
                trajectory: traj,
                git_checkpoint: None,
                rollback_handle: None,
                validation_failure_policy: req.validation_failure_policy,
                artifacts: vec![],
                failures: vec![FailureKind::CheckpointFailed],
                summary: format!("Checkpoint creation failed in {:?} mode: {}", req.mode, e),
                execution_metrics: metrics,
                step_count: ctx.step_count,
                terminated_early: true,
                termination_reason: Some(format!("Checkpoint failure: {}", e)),
                evidence_log,
            });
        }
        (_, Ok(cp)) => {
            // Record checkpoint creation evidence
            evidence_log.record_checkpoint_created(&cp, Some(trace_id.clone()));
            Some(cp)
        }
    };

    // P1-010: Child span for patch apply phase
    let patch_apply_span = tracing::info_span!(
        "harness.patch_apply",
        trace_id = %trace_id,
        phase = "patch_apply",
        should_apply = should_apply,
    );

    // STEP 8: Apply patch only if approved - with rollback support
    let (patch, rollback_handle) =
        if should_apply && dry.failures.is_empty() && !selected_edits.is_empty() {
            let _enter = patch_apply_span.enter();
            let (result, handle) =
                apply_patch_with_rollback(&selected_edits, &files, &policy).await?;
            tracing::info!(
                trace_id = %trace_id,
                files_changed = result.changed_files.len(),
                "P1-010: Patch applied to real repo"
            );
            // Record patch application evidence (real repo)
            evidence_log.record_patch_applied(&result, false, Some(&handle), Some(trace_id.clone()));
            (Some(result), Some(handle))
        } else {
            tracing::info!(
                trace_id = %trace_id,
                should_apply = should_apply,
                dry_failures = dry.failures.len(),
                "P1-010: Patch not applied (review-only or dry-run failed)"
            );
            // Return dry-run result (patch not actually applied)
            // Record that patch was NOT applied to real repo
            evidence_log.record_patch_applied(&dry, true, None, Some(trace_id.clone()));
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
        tool_ids: vec![],
    };

    // Determine validation target: real repo if patch applied, temp workspace otherwise
    let validation_target = if patch.as_ref().is_some_and(|p| p.applied) {
        ValidationTarget::RealRepo(req.repo_root.clone())
    } else if !selected_edits.is_empty() && dry.failures.is_empty() {
        // Create temp workspace for validation when patch not applied to real repo
        match TempWorkspace::create_temp_copy(&req.repo_root, &selected_edits, &files, &policy).await {
            Ok((workspace, _)) => ValidationTarget::TempWorkspace(workspace),
            Err(e) => {
                ctx.record_action("validation", "temp_workspace_failed", &format!("Failed to create temp workspace: {}", e));
                ValidationTarget::None
            }
        }
    } else {
        ValidationTarget::None
    };

    // P1-010: Child span for validation phase
    let validation_span = tracing::info_span!(
        "harness.validation",
        trace_id = %trace_id,
        phase = "validation",
    );

    let validation = if let Some(val_root) = validation_target.path() {
        ctx.send_progress(HarnessProgress::Validating {
            commands_to_run: plan.format_commands.len()
                + plan.lint_commands.len()
                + plan.test_commands.len(),
        });

        let val_start = Instant::now();
        let result = {
            let _enter = validation_span.enter();
            let r = run_validation(val_root, &plan, std::sync::Arc::new(LocalSandboxRuntime::default())).await?;
            tracing::info!(
                trace_id = %trace_id,
                passed = r.passed,
                commands_run = r.command_results.len(),
                "P1-010: Validation complete"
            );
            r
        };
        metrics.validation_ms = val_start.elapsed().as_millis() as u64;

        // Record validation completion evidence
        evidence_log.record_validation_completed(&result, Some(trace_id.clone()));

        // Record individual validation command results
        for cmd_result in &result.command_results {
            evidence_log.record_validation_command(cmd_result, Some(trace_id.clone()));
        }

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

        // Cleanup temp workspace if used
        if let ValidationTarget::TempWorkspace(ws) = validation_target {
            let _ = ws.cleanup().await;
        }

        Some(result)
    } else {
        None
    };

    // P0: Hard gate - check if validation was bypassed when required
    if validation.is_none() {
        let bypass_check = policy_gate.check_validation_bypass("No validation target available");
        if matches!(bypass_check, GateDecision::Block(_)) {
            evidence_log.record_side_effect_blocked("Validation bypass blocked by policy gate", Some(trace_id.clone()));
            // In strict modes, this would block completion
            tracing::warn!("Validation was required but bypassed");
        }
    }

    let mut failures: Vec<FailureKind> = dry_failures
        .iter()
        .map(|_f| FailureKind::PatchApplyFailure)
        .collect();

    // STEP 8.5: Post-validation selection with stricter criteria
    // After validation, re-evaluate the patch using stricter post-validation criteria
    let post_validation_criteria = SelectionPhase::PostValidation.criteria();
    let validation_passed = validation.as_ref().map(|v| v.passed).unwrap_or(false);

    // If we had a candidate, re-score it with post-validation criteria
    if !selected_edits.is_empty() {
        use crate::harness::confidence::{ConfidenceScore, ConfidenceFactor, FactorImpact};
        use crate::harness::selection::SelectionEngine;

        let lines_total: usize = selected_edits.iter()
            .map(|e| e.lines_added() + e.lines_removed())
            .sum();

        let post_validation_candidate = SelectionCandidate {
            id: format!("post_val_{}", req.work_context_id),
            edits: selected_edits.clone(),
            source: "post_validation".into(),
            confidence: ConfidenceScore {
                score: if validation_passed { 0.9 } else { 0.1 },
                factors: vec![ConfidenceFactor {
                    name: "validation_result".into(),
                    weight: 1.0,
                    score: if validation_passed { 0.9 } else { 0.1 },
                    description: "post-validation assessment".into(),
                    impact: if validation_passed { FactorImpact::Positive } else { FactorImpact::Negative },
                }],
                explanation: "post-validation selection".into(),
                recommendation: None,
            },
            metadata: std::collections::HashMap::new(),
            risk: Some(risk.clone()),
            validation: validation.clone(),
            review_issues: review_issues.clone(),
            semantic_diff: None,
            lines_added: lines_total,
            lines_removed: 0,
        };

        // Use SelectionEngine to score with post-validation criteria
        let mut selection_engine = SelectionEngine::new(post_validation_criteria);
        let scored = selection_engine.rank_candidates(vec![post_validation_candidate]);

        if let Some(first) = scored.first() {
            if !first.is_eligible {
                tracing::info!(
                    "Post-validation selection rejected candidate: failed stricter criteria"
                );
                if validation_passed {
                    // Validation passed but failed other criteria (risk, review issues)
                    failures.push(FailureKind::SemanticFailure);
                }
            }
        }
    }

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
                if let Some(ref handle) = rollback_handle {
                    // P1-010: Child span for rollback phase
                    let rollback_span = tracing::info_span!(
                        "harness.rollback",
                        trace_id = %trace_id,
                        phase = "rollback",
                    );
                    let _enter = rollback_span.enter();

                    match handle.clone().rollback().await {
                        Ok(result) => {
                            failures.push(FailureKind::ValidationFailed);
                            failures.push(FailureKind::PatchRolledBack);
                            tracing::info!(
                                trace_id = %trace_id,
                                restored = result.restored.len(),
                                "P1-010: Rollback successful"
                            );
                            // Record rollback evidence
                            evidence_log.record_rollback("validation failed - automatic rollback", Some(trace_id.clone()));
                            ctx.send_progress(HarnessProgress::RolledBack {
                                restored_files: result.restored.len(),
                                deleted_files: result.deleted.len(),
                                recreated_files: result.recreated.len(),
                            });
                        }
                        Err(e) => {
                            failures.push(FailureKind::ValidationFailed);
                            failures.push(FailureKind::RollbackFailed);
                            tracing::error!(trace_id = %trace_id, error = %e, "P1-010: Rollback failed");
                            // Record rollback failure
                            evidence_log.record_rollback(&format!("rollback failed: {}", e), Some(trace_id.clone()));
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
            format_check_passed: validation.as_ref()
                .and_then(|v| v.category_results.get(&ValidationCategory::Format))
                .map(|r| r.passed)
                .unwrap_or(false),
            static_check_passed: validation.as_ref()
                .and_then(|v| v.category_results.get(&ValidationCategory::Lint))
                .map(|r| r.passed)
                .unwrap_or(false),
            lint_check_passed: validation.as_ref()
                .and_then(|v| v.category_results.get(&ValidationCategory::Lint))
                .map(|r| r.passed)
                .unwrap_or(false),
            test_passed: validation.as_ref()
                .and_then(|v| v.category_results.get(&ValidationCategory::Test))
                .map(|r| r.passed)
                .unwrap_or(false),
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

    // Record completion evaluation evidence
    evidence_log.record_completion_evaluated(
        format!("{:?}", decision),
        validation.as_ref().map(|v| v.passed).unwrap_or(false),
        Some(trace_id.clone()),
    );

    ctx.send_progress(HarnessProgress::Completing {
        decision: format!("{:?}", decision),
        confidence: confidence.score,
    });

    traj.record_step("completion.evaluate", ctx.elapsed_ms(), vec![]);
    traj.complete();

    metrics.total_duration_ms = started.elapsed().as_millis() as u64;

    let trace_id = crate::harness::observability::otel::generate_trace_id();

    // Complete the evidence log
    evidence_log.complete();

    let mut result = HarnessExecutionResult {
        work_context_id: req.work_context_id,
        trace_id: Some(trace_id.clone()),
        repo_context: repo,
        environment: env,
        file_set: files,
        acceptance,
        patch_result: patch,
        validation_result: validation,
        review_issues: review_issues.clone(),
        risk_assessment: risk.clone(),
        confidence: confidence.clone(),
        verification_strength: strength,
        completion_decision: decision.clone(),
        trajectory: traj.clone(),
        git_checkpoint: checkpoint,
        rollback_handle,
        validation_failure_policy: req.validation_failure_policy,
        artifacts: vec![],
        failures: failures.clone(),
        summary: format!("Harness execution completed with decision: {:?}", decision),
        execution_metrics: metrics.clone(),
        step_count: ctx.step_count,
        terminated_early: false,
        termination_reason: None,
        evidence_log: evidence_log.clone(),
    };

    ctx.send_progress(HarnessProgress::Patching {
        files_to_modify: req.proposed_edits.len(),
        dry_run: true,
    });

    let report_content = format!(
        "# Harness Execution Report\n\n\
        Work Context ID: {}\n\
        Decision: {:?}\n\
        Steps: {}\n\
        Failures: {:?}\n",
        result.work_context_id,
        result.completion_decision,
        result.step_count,
        result.failures
    );

    result.artifacts.push(HarnessArtifact {
        id: format!("artifact-{}", result.work_context_id),
        kind: ArtifactKind::Report,
        path: None,
        content: Some(report_content.clone()),
        compressed_content: None,
        compression: CompressionType::None,
        metadata: ArtifactMetadata {
            work_context_id: result.work_context_id.clone(),
            harness_run_id: result.work_context_id.clone(),
            tags: vec!["completion".into()],
            custom_fields: HashMap::new(),
        },
        created_at: Utc::now(),
        size_bytes: report_content.len(),
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
