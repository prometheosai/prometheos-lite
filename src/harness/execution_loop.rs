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
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

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
    #[serde(skip)]
    pub progress_callback: Option<Box<dyn Fn(HarnessProgress) + Send + Sync>>,
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
        message: String,
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
            return Err(format!("Time limit reached: {}ms > {}ms", elapsed, self.limits.max_time_ms));
        }
        
        if self.step_count >= self.limits.max_steps {
            return Err(format!("Step limit reached: {} >= {}", self.step_count, self.limits.max_steps));
        }
        
        if self.cost_accrued >= self.limits.max_cost_usd {
            return Err(format!("Cost limit reached: ${:.4} >= ${:.4}", self.cost_accrued, self.limits.max_cost_usd));
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

pub async fn execute_harness_task(req: HarnessExecutionRequest) -> Result<HarnessExecutionResult> {
    let (mut ctx, mut _progress_rx) = ExecutionContext::new(req.limits);
    let started = Instant::now();
    
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
        traj.record_step("patch.generate", ctx.elapsed_ms(), vec!["no structured edits supplied".into()]);
        traj.complete();
        
        ctx.send_progress(HarnessProgress::Error {
            step: "patch.generate".into(),
            message: "No structured edits supplied".into(),
        });
        
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
                reasons: vec!["No edits proposed".into()],
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
    let dry = dry_run_patch(&req.proposed_edits, &files, &policy)
        .await
        .context("patch dry-run failed")?;
    
    let dry_failures: Vec<FailureKind> = dry.failures
        .iter()
        .map(classify_patch_failure)
        .collect();
    
    ctx.send_progress(HarnessProgress::PatchResult {
        success: dry.failures.is_empty(),
        files_changed: dry.changed_files.len(),
        failures: dry.failures.len(),
    });
    
    ctx.increment_step();
    if let Err(reason) = ctx.check_limits() {
        return create_terminated_result(&req, started.elapsed().as_millis() as u64, &reason, &ctx);
    }
    
    let checkpoint = create_pre_task_checkpoint(&req.repo_root).await.ok();
    
    let patch = if dry.failures.is_empty() {
        Some(apply_patch(&req.proposed_edits, &files, &policy).await?)
    } else {
        Some(dry)
    };
    
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
    };
    
    let validation = if patch.as_ref().is_some_and(|p| p.failures.is_empty()) {
        ctx.send_progress(HarnessProgress::Validating {
            commands_to_run: plan.format_commands.len() + plan.lint_commands.len() + plan.test_commands.len(),
        });
        
        let val_start = Instant::now();
        let result = run_validation(&req.repo_root, &plan, &LocalSandboxRuntime::default()).await?;
        metrics.validation_ms = val_start.elapsed().as_millis() as u64;
        
        let tests_run = result.command_results.len();
        let tests_passed = result.command_results.iter().filter(|r| r.exit_code == Some(0)).count();
        
        ctx.send_progress(HarnessProgress::ValidationResult {
            passed: result.passed,
            tests_run,
            tests_passed,
        });
        
        Some(result)
    } else {
        None
    };
    
    let mut failures = dry_failures;
    if let Some(ref v) = validation {
        if !v.passed {
            failures.push(classify_validation_failure(v));
        }
    }
    
    ctx.increment_step();
    if let Err(reason) = ctx.check_limits() {
        return create_terminated_result(&req, started.elapsed().as_millis() as u64, &reason, &ctx);
    }
    
    let diff = patch.as_ref().map(|p| p.diff.as_str()).unwrap_or("");
    
    let review_start = Instant::now();
    let review = review_diff(diff);
    metrics.review_ms = review_start.elapsed().as_millis() as u64;
    
    let critical_count = review.iter().filter(|i| i.severity == ReviewSeverity::Critical).count();
    let max_severity = review.iter().map(|i| &i.severity).max();
    
    ctx.send_progress(HarnessProgress::Reviewing {
        issues_found: review.len(),
        max_severity: max_severity.map(|s| format!("{:?}", s)),
    });
    
    let risk = assess_risk(&analyze_semantic_diff(diff), &review);
    
    ctx.send_progress(HarnessProgress::RiskAssessment {
        level: format!("{:?}", risk.level),
        requires_approval: risk.requires_approval,
    });
    
    let strength = assess_verification_strength(validation.as_ref());
    let confidence = compute_confidence(validation.as_ref(), &review, &risk, strength);
    
    let evidence = CompletionEvidence {
        patch_exists: patch.as_ref().is_some_and(|p| !p.diff.is_empty() && p.failures.is_empty()),
        validation_ran: validation.is_some(),
        validation_passed: validation.as_ref().is_some_and(|v| v.passed),
        review_ran: true,
        critical_issues: critical_count,
        confidence: confidence.clone(),
        verification_strength: strength,
        requires_approval: risk.requires_approval,
    };
    
    let decision = evaluate_completion(&evidence, req.mode)?;
    
    ctx.send_progress(HarnessProgress::Completing {
        decision: format!("{:?}", decision),
        confidence: confidence.score,
    });
    
    traj.record_step("completion.evaluate", ctx.elapsed_ms(), vec![]);
    traj.complete();
    
    metrics.total_duration_ms = started.elapsed().as_millis() as u64;
    
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
        execution_metrics: metrics,
        step_count: ctx.step_count,
        terminated_early: false,
        termination_reason: None,
    };
    
    let content = generate_completion_artifact(&result)?;
    result.artifacts.push(HarnessArtifact {
        kind: "completion".into(),
        path: None,
        content: Some(content),
        metadata: serde_json::json!({}),
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
