use crate::harness::{
    edit_protocol::{EditOperation, SearchReplaceEdit, WholeFileEdit},
    failure::{
        FailureContext, FailureDetails, FailureKind, classify_patch_failure,
        classify_validation_failure,
    },
    file_control::{FilePolicy, FileSet},
    patch_applier::{PatchFailure, PatchResult, RollbackHandle, apply_patch_with_rollback, dry_run_patch},
    patch_provider::{
        AggregatePatchProvider, AttemptOutcome, AttemptRecord, GenerateRequest, GenerateResponse,
        HeuristicPatchProvider, LlmPatchProvider, PatchProvider, PatchProviderContext,
        ProviderCandidate, ProviderCapabilities, RepairRequest as ProviderRepairRequest,
        RepairResponse, RepairStrategy as ProviderRepairStrategy, RiskEstimate,
    },
    repo_intelligence::RepoMap,
    sandbox::{LocalSandboxRuntime, SandboxRuntime},
    validation::{ValidationPlan, ValidationResult},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepairRequest {
    pub failure: FailureDetails,
    pub original_edits: Vec<EditOperation>,
    pub patch_result: Option<PatchResult>,
    pub validation_result: Option<ValidationResult>,
    pub attempt_count: u32,
    pub max_attempts: u32,
    #[serde(skip)]
    pub provider_context: Option<PatchProviderContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepairResult {
    pub success: bool,
    pub attempts: Vec<RepairAttempt>,
    pub final_edits: Option<Vec<EditOperation>>,
    pub final_failure: Option<FailureDetails>,
    pub total_duration_ms: u64,
    pub repair_strategy: RepairStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepairAttempt {
    pub attempt_number: u32,
    pub strategy: RepairStrategy,
    pub prompt: String,
    pub edits: Vec<EditOperation>,
    pub result: AttemptResult,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AttemptResult {
    Success,
    PartialSuccess {
        remaining_failures: Vec<FailureDetails>,
    },
    Failure {
        reason: String,
        failure: FailureDetails,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum RepairStrategy {
    FixSearchReplace,
    FixSyntaxError,
    FixCompileError,
    FixTestFailure,
    AddMissingImport,
    RemoveConflictingChange,
    ExpandContextWindow,
    NarrowSearchPattern,
    UseWholeFileEdit,
    RetryWithMoreContext,
    RetryWithNarrowerSearch,
    RequestClarification,
    Abort,
    NarrowSearchContext,
    ExpandSearchWildcard,
    RelaxLineAnchors,
    SwitchToWholeFile,
    AddContextLines,
    FixUnclosedDelimiters,
    RetryWithLLM,
}

pub struct RepairLoop {
    max_attempts: u32,
    strategies_tried: HashMap<RepairStrategy, u32>,
    attempt_history: VecDeque<RepairAttempt>,
}

impl RepairLoop {
    pub fn new(max_attempts: u32) -> Self {
        Self {
            max_attempts,
            strategies_tried: HashMap::new(),
            attempt_history: VecDeque::with_capacity(max_attempts as usize),
        }
    }

    /// Generate repair edits based on failure details and strategy
    fn generate_repair_edits(
        &self,
        failure: &FailureDetails,
        current_edits: &[EditOperation],
        strategy: RepairStrategy,
    ) -> anyhow::Result<Vec<EditOperation>> {
        use crate::harness::patch_provider::narrow_search_repair;

        match strategy {
            RepairStrategy::NarrowSearchContext => {
                tracing::info!("Applying narrow search context repair");
                narrow_search_repair(current_edits)
            }
            RepairStrategy::ExpandSearchWildcard => {
                tracing::info!("Applying expand search wildcard repair");
                narrow_search_repair(current_edits)
            }
            RepairStrategy::RelaxLineAnchors => {
                tracing::info!("Applying relax line anchors repair");
                narrow_search_repair(current_edits)
            }
            RepairStrategy::SwitchToWholeFile => {
                tracing::info!("P2-011: Applying whole-file replacement strategy (RISK INCREASE)");
                // P2-011: Whole-file fallback increases risk because it replaces entire file content
                // This should only be used when search/replace fails and requires explicit approval in strict modes
                let mut whole_file_edits = Vec::new();
                for edit in current_edits {
                    if let EditOperation::SearchReplace(sr) = edit {
                        if let Ok(content) = std::fs::read_to_string(&sr.file) {
                            let new_content = content.replace(&sr.search, &sr.replace);
                            whole_file_edits.push(EditOperation::WholeFile(
                                crate::harness::edit_protocol::WholeFileEdit {
                                    file: sr.file.clone(),
                                    content: new_content,
                                }
                            ));
                        } else {
                            whole_file_edits.push(edit.clone());
                        }
                    } else {
                        whole_file_edits.push(edit.clone());
                    }
                }
                Ok(whole_file_edits)
            }
            RepairStrategy::AddContextLines => {
                tracing::info!("Applying add context lines repair");
                narrow_search_repair(current_edits)
            }
            RepairStrategy::FixUnclosedDelimiters => {
                tracing::info!("Applying fix unclosed delimiters repair");
                crate::harness::patch_provider::fix_unclosed_delimiters(current_edits)
            }
            RepairStrategy::RetryWithLLM => {
                tracing::info!("Applying retry with LLM strategy");
                Ok(current_edits.to_vec())
            }
            // Other strategies - not yet implemented
            _ => {
                tracing::info!("Strategy {:?} not yet implemented, returning original edits", strategy);
                Ok(current_edits.to_vec())
            }
        }
    }
    /// P2-011: Execute repair with evidence-driven validation
    ///
    /// Each repair attempt creates a new candidate that must pass validation.
    /// All attempts are recorded in the EvidenceLog for auditability.
    pub async fn execute_repair(
        &mut self,
        request: RepairRequest,
        file_set: &FileSet,
        policy: &FilePolicy,
        sandbox: &dyn SandboxRuntime,
        provider: Option<&dyn PatchProvider>,
        evidence_log: &mut crate::harness::evidence::EvidenceLog,
        trace_id: Option<String>,
    ) -> Result<RepairResult> {
        let start = Instant::now();
        let mut current_edits = request.original_edits.clone();

        // P2-011: Record repair loop start in evidence log
        evidence_log.record_repair_action(
            "repair_loop",
            "started",
            &format!("Starting repair loop for failure: {:?}", request.failure.kind),
            trace_id.clone(),
        );

        for attempt in 1..=self.max_attempts {
            let strategy = self.select_strategy(&request, attempt);
            let prompt = self.generate_failure_prompt(&request, strategy);

            let attempt_start = Instant::now();

            // P2-011: Record strategy selection
            evidence_log.record_repair_action(
                "repair_loop",
                "strategy_selected",
                &format!("Attempt {}: Selected strategy {:?}", attempt, strategy),
                trace_id.clone(),
            );

            let repair_edits = self.generate_repair_edits(
                &request.failure,
                &current_edits,
                strategy,
            )?;

            // P2-011: Record repair edits generation
            evidence_log.record_repair_action(
                "repair_loop",
                "edits_generated",
                &format!("Attempt {}: Generated {} repair edits using {:?}",
                    attempt, repair_edits.len(), strategy),
                trace_id.clone(),
            );

            let dry_result = dry_run_patch(&repair_edits, file_set, policy).await?;

            // P2-011: Record dry-run result
            if dry_result.failures.is_empty() {
                evidence_log.record_dry_run(&dry_result, attempt_start.elapsed().as_millis() as u64, trace_id.clone());
            } else {
                evidence_log.record_repair_action(
                    "repair_loop",
                    "dry_run_failed",
                    &format!("Attempt {}: Dry-run failed with {} failures", attempt, dry_result.failures.len()),
                    trace_id.clone(),
                );
            }

            let (patch_result, attempt_result) = if dry_result.failures.is_empty() {
                // P0 SAFETY: Use apply_patch_with_rollback so failed repairs can be undone
                let (patch, rollback) = apply_patch_with_rollback(&repair_edits, file_set, policy).await?;

                let validation_plan = ValidationPlan {
                    format_commands: vec![],
                    lint_commands: vec![],
                    test_commands: vec!["cargo test".into()],
                    repro_commands: vec![],
                    timeout_ms: Some(60000),
                    parallel: true,
                    tool_ids: vec![],
                    disable_cache: false,
                };

                // Clone the sandbox Arc for validation
                let sandbox_arc: std::sync::Arc<dyn crate::harness::sandbox::SandboxRuntime + Send + Sync> = std::sync::Arc::new(
                    LocalSandboxRuntime::default()
                );
                let validation_result = crate::harness::validation::run_validation(
                    &policy.repo_root,
                    &validation_plan,
                    sandbox_arc,
                )
                .await;

                let result = match &validation_result {
                    Ok(validation) => {
                        if validation.passed() {
                            // P2-011: Record successful validation
                            evidence_log.record_validation_completed(validation, trace_id.clone());
                            AttemptResult::Success
                        } else {
                            // P2-011: Record failed validation
                            evidence_log.record_repair_action(
                                "repair_loop",
                                "validation_failed",
                                &format!("Attempt {}: Validation failed with {} errors", attempt, validation.errors.len()),
                                trace_id.clone(),
                            );
                            let failure = classify_validation_failure(&validation);
                            AttemptResult::PartialSuccess {
                                remaining_failures: vec![create_failure_from_kind(
                                    failure,
                                    "Validation failed",
                                )],
                            }
                        }
                    }
                    Err(e) => {
                        // P2-011: Record validation error
                        evidence_log.record_repair_action(
                            "repair_loop",
                            "validation_error",
                            &format!("Attempt {}: Validation error: {}", attempt, e),
                            trace_id.clone(),
                        );
                        AttemptResult::Failure {
                            reason: e.to_string(),
                            failure: create_failure_from_kind(FailureKind::ToolFailure, &e.to_string()),
                        }
                    },
                };

                // P0 SAFETY: Rollback failed repair attempts to avoid partial/corrupt state
                let should_rollback = match &validation_result {
                    Ok(v) => !v.passed(),
                    Err(_) => true,
                };

                if should_rollback {
                    tracing::info!("Repair validation failed, rolling back patch");
                    evidence_log.record_rollback(&format!("Attempt {}: Repair validation failed", attempt), trace_id.clone());
                    if let Err(e) = rollback.rollback().await {
                        tracing::error!("Failed to rollback repair patch: {}", e);
                    }
                }

                (Some(patch), result)
            } else {
                let failure = classify_patch_failure(&dry_result.failures[0]);
                let result = AttemptResult::Failure {
                    reason: format!("Patch dry-run failed: {:?}", dry_result.failures[0]),
                    failure: create_failure_from_kind(failure, &dry_result.failures[0].reason),
                };
                (None, result)
            };

            let attempt_record = RepairAttempt {
                attempt_number: attempt,
                strategy,
                prompt,
                edits: repair_edits.clone(),
                result: attempt_result.clone(),
                duration_ms: attempt_start.elapsed().as_millis() as u64,
            };

            *self.strategies_tried.entry(strategy).or_insert(0) += 1;
            self.attempt_history.push_back(attempt_record);

            if self.attempt_history.len() > self.max_attempts as usize {
                self.attempt_history.pop_front();
            }

            match &attempt_result {
                AttemptResult::Success => {
                    // P2-011: Record successful repair completion
                    evidence_log.record_repair_action(
                        "repair_loop",
                        "completed",
                        &format!("Repair succeeded after {} attempts using {:?}", attempt, strategy),
                        trace_id.clone(),
                    );
                    return Ok(RepairResult {
                        success: true,
                        attempts: self.attempt_history.iter().cloned().collect(),
                        final_edits: Some(repair_edits),
                        final_failure: None,
                        total_duration_ms: start.elapsed().as_millis() as u64,
                        repair_strategy: strategy,
                    });
                }
                AttemptResult::PartialSuccess { .. } => {
                    current_edits = repair_edits;
                }
                AttemptResult::Failure { failure, .. } => {
                    if attempt == self.max_attempts {
                        // P2-011: Record failed repair after max attempts
                        evidence_log.record_repair_action(
                            "repair_loop",
                            "failed",
                            &format!("Repair failed after {} max attempts", self.max_attempts),
                            trace_id.clone(),
                        );
                        return Ok(RepairResult {
                            success: false,
                            attempts: self.attempt_history.iter().cloned().collect(),
                            final_edits: None,
                            final_failure: Some(failure.clone()),
                            total_duration_ms: start.elapsed().as_millis() as u64,
                            repair_strategy: strategy,
                        });
                    }
                }
            }
        }

        // P2-011: Record exhausted repair loop
        evidence_log.record_repair_action(
            "repair_loop",
            "exhausted",
            &format!("Repair loop exhausted after {} attempts", self.max_attempts),
            trace_id.clone(),
        );

        Ok(RepairResult {
            success: false,
            attempts: self.attempt_history.iter().cloned().collect(),
            final_edits: None,
            final_failure: request.failure.into(),
            total_duration_ms: start.elapsed().as_millis() as u64,
            repair_strategy: RepairStrategy::Abort,
        })
    }

    fn select_strategy(&self, request: &RepairRequest, attempt: u32) -> RepairStrategy {
        let failure_kind = request.failure.kind;

        let strategy_scores: Vec<(RepairStrategy, i32)> = vec![
            (
                RepairStrategy::FixSearchReplace,
                self.score_strategy(RepairStrategy::FixSearchReplace, &failure_kind),
            ),
            (
                RepairStrategy::FixSyntaxError,
                self.score_strategy(RepairStrategy::FixSyntaxError, &failure_kind),
            ),
            (
                RepairStrategy::FixCompileError,
                self.score_strategy(RepairStrategy::FixCompileError, &failure_kind),
            ),
            (
                RepairStrategy::FixTestFailure,
                self.score_strategy(RepairStrategy::FixTestFailure, &failure_kind),
            ),
            (
                RepairStrategy::ExpandContextWindow,
                self.score_strategy(RepairStrategy::ExpandContextWindow, &failure_kind),
            ),
            (
                RepairStrategy::NarrowSearchPattern,
                self.score_strategy(RepairStrategy::NarrowSearchPattern, &failure_kind),
            ),
            (
                RepairStrategy::UseWholeFileEdit,
                self.score_strategy(RepairStrategy::UseWholeFileEdit, &failure_kind),
            ),
        ];

        let mut sorted = strategy_scores;
        sorted.sort_by(|a, b| b.1.cmp(&a.1));

        for (strategy, _) in sorted {
            let times_tried = self.strategies_tried.get(&strategy).copied().unwrap_or(0);
            if times_tried < 2 {
                return strategy;
            }
        }

        if attempt >= self.max_attempts - 1 {
            RepairStrategy::Abort
        } else {
            RepairStrategy::RequestClarification
        }
    }

    fn score_strategy(&self, strategy: RepairStrategy, failure_kind: &FailureKind) -> i32 {
        let base_score = match (strategy, failure_kind) {
            (RepairStrategy::FixSearchReplace, FailureKind::PatchApplyFailure) => 100,
            (RepairStrategy::FixSyntaxError, FailureKind::PatchParseFailure) => 90,
            (RepairStrategy::FixSyntaxError, FailureKind::CompileFailure) => 85,
            (RepairStrategy::FixCompileError, FailureKind::CompileFailure) => 100,
            (RepairStrategy::FixTestFailure, FailureKind::TestFailure) => 100,
            (RepairStrategy::FixTestFailure, FailureKind::RegressionFailure) => 95,
            (RepairStrategy::AddMissingImport, FailureKind::CompileFailure) => 80,
            (RepairStrategy::RemoveConflictingChange, FailureKind::PatchApplyFailure) => 75,
            (RepairStrategy::ExpandContextWindow, FailureKind::PatchApplyFailure) => 70,
            (RepairStrategy::NarrowSearchPattern, FailureKind::PatchApplyFailure) => 65,
            (RepairStrategy::UseWholeFileEdit, _) => 50,
            (RepairStrategy::RequestClarification, FailureKind::ModelFailure) => 60,
            _ => 10,
        };

        let times_tried = self.strategies_tried.get(&strategy).copied().unwrap_or(0);
        base_score - (times_tried as i32 * 20)
    }

    fn generate_failure_prompt(&self, request: &RepairRequest, strategy: RepairStrategy) -> String {
        let failure = &request.failure;
        let history = self.format_attempt_history();

        let strategy_guidance = match strategy {
            RepairStrategy::FixSearchReplace => {
                "The search pattern likely doesn't match exactly. Consider:\n\
                - Checking for exact whitespace, including trailing spaces\n\
                - Expanding context lines to make the search unique\n\
                - Using a different portion of the code\n\
                - Checking if the file was already modified"
            }
            RepairStrategy::FixSyntaxError => {
                "There's a syntax error in the generated code. Please:\n\
                - Ensure all brackets, braces, and parentheses are balanced\n\
                - Check for proper indentation\n\
                - Verify string literals are properly closed\n\
                - Ensure all statements are properly terminated"
            }
            RepairStrategy::FixCompileError => {
                "The code fails to compile. Please:\n\
                - Check for missing imports or use statements\n\
                - Verify type compatibility\n\
                - Ensure all referenced variables and functions exist\n\
                - Check for proper visibility (public/private) modifiers"
            }
            RepairStrategy::FixTestFailure => {
                "Tests are failing. Please:\n\
                - Review the test output to understand the expected vs actual values\n\
                - Ensure the fix addresses the root cause, not just symptoms\n\
                - Check for edge cases that might not be handled\n\
                - Verify no regressions were introduced"
            }
            RepairStrategy::ExpandContextWindow => {
                "The search pattern needs more context. Please:\n\
                - Include more lines before and after the target code\n\
                - Ensure the context uniquely identifies the target location\n\
                - Consider including function signatures or class definitions"
            }
            RepairStrategy::NarrowSearchPattern => {
                "The search pattern matches multiple locations. Please:\n\
                - Add more specific context to make the search unique\n\
                - Include variable names or specific literals\n\
                - Consider using more lines of context"
            }
            RepairStrategy::UseWholeFileEdit => {
                "The incremental edit is failing. Consider:\n\
                - Using a whole-file edit instead\n\
                - This is appropriate when making widespread changes\n\
                - Be careful to preserve all other parts of the file"
            }
            _ => "Please analyze the failure and provide a corrected fix.",
        };

        format!(
            "## Repair Attempt {}\n\n\
            ### Previous Attempts\n{}\n\n\
            ### Current Failure\n\
            - Type: {:?}\n\
            - Category: {:?}\n\
            - Severity: {:?}\n\
            - Message: {}\n\n\
            ### Selected Strategy: {:?}\n\n{}",
            self.attempt_history.len() + 1,
            if history.is_empty() {
                "None\n".into()
            } else {
                history
            },
            failure.kind,
            failure.category,
            failure.severity,
            failure.message,
            strategy,
            strategy_guidance
        )
    }

    fn format_attempt_history(&self) -> String {
        if self.attempt_history.is_empty() {
            return String::new();
        }

        let mut history = String::new();
        for attempt in &self.attempt_history {
            let status = match attempt.result {
                AttemptResult::Success => "✓ Success",
                AttemptResult::PartialSuccess { .. } => "◐ Partial",
                AttemptResult::Failure { .. } => "✗ Failed",
            };
            history.push_str(&format!(
                "- Attempt {}: {:?} - {} ({}ms)\n",
                attempt.attempt_number, attempt.strategy, status, attempt.duration_ms
            ));
        }
        history
    }
}

fn create_failure_from_kind(kind: FailureKind, message: impl Into<String>) -> FailureDetails {
    FailureDetails {
        kind,
        category: kind.category(),
        severity: kind.default_severity(),
        message: message.into(),
        context: FailureContext::default(),
        suggestion: None,
        recovery_action: kind.default_recovery(),
    }
}

fn get_edit_file(edit: &EditOperation) -> Option<PathBuf> {
    match edit {
        EditOperation::SearchReplace(e) => Some(e.file.clone()),
        EditOperation::WholeFile(e) => Some(e.file.clone()),
        EditOperation::CreateFile(e) => Some(e.file.clone()),
        EditOperation::DeleteFile(e) => Some(e.file.clone()),
        EditOperation::RenameFile(e) => Some(e.from.clone()),
        _ => None,
    }
}

pub async fn run_repair_loop(
    failure: FailureDetails,
    original_edits: Vec<EditOperation>,
    patch_result: Option<PatchResult>,
    validation_result: Option<ValidationResult>,
    file_set: &FileSet,
    policy: &FilePolicy,
    sandbox: &dyn SandboxRuntime,
    max_attempts: u32,
    provider: Option<&dyn PatchProvider>,
    provider_context: Option<PatchProviderContext>,
    evidence_log: &mut crate::harness::evidence::EvidenceLog,
    trace_id: Option<String>,
) -> Result<RepairResult> {
    let request = RepairRequest {
        failure,
        original_edits,
        patch_result,
        validation_result,
        attempt_count: 0,
        max_attempts,
        provider_context,
    };

    let mut loop_state = RepairLoop::new(max_attempts);
    loop_state
        .execute_repair(request, file_set, policy, sandbox, provider, evidence_log, trace_id)
        .await
}

pub fn format_repair_report(result: &RepairResult) -> String {
    let mut report = String::new();

    report.push_str("## Repair Loop Report\n\n");
    report.push_str(&format!(
        "**Status:** {}\n\n",
        if result.success {
            "✓ SUCCESS"
        } else {
            "✗ FAILED"
        }
    ));
    report.push_str(&format!("**Strategy:** {:?}\n", result.repair_strategy));
    report.push_str(&format!(
        "**Total Duration:** {}ms\n\n",
        result.total_duration_ms
    ));

    report.push_str("### Attempts\n");
    for attempt in &result.attempts {
        let result_icon = match attempt.result {
            AttemptResult::Success => "✓",
            AttemptResult::PartialSuccess { .. } => "◐",
            AttemptResult::Failure { .. } => "✗",
        };
        report.push_str(&format!(
            "\n**Attempt {}** {} ({:?}) - {}ms\n",
            attempt.attempt_number, result_icon, attempt.strategy, attempt.duration_ms
        ));
    }

    if let Some(ref failure) = result.final_failure {
        report.push_str(&format!(
            "\n### Final Failure\n{:?}: {}\n",
            failure.kind, failure.message
        ));
    }

    if result.success {
        report.push_str("\n### ✓ Repair Successful\n");
    } else {
        report.push_str("\n### ✗ Repair Failed After Maximum Attempts\n");
    }

    report
}
