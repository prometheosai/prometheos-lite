//! WorkOrchestrator - Central service for persistent work execution
//!
//! This module provides the WorkOrchestrator, which owns the high-level
//! execution loop for persistent work contexts with hard stop contracts.

use anyhow::{Context, Result};
use std::sync::Arc;

use super::evolution_engine::EvolutionEngine;
use super::execution_service::WorkExecutionService;
use super::service::WorkContextService;
use super::types::{
    AutonomyLevel, HarnessMetadata, TestExecutionResult, WorkContext, WorkDomain, WorkPhase,
    WorkStatus,
};
use crate::db::repository::WorkContextEventOperations;
use crate::harness::completion::CompletionDecision;
use crate::intent::{Intent, IntentClassifier};
use crate::workflow::evaluate::CancellationToken;

/// EvolutionTrigger - when to trigger playbook evolution
#[derive(Debug, Clone, Copy)]
pub enum EvolutionTrigger {
    /// Context completed successfully
    Completion,
    /// Context partially failed
    PartialFailure,
    /// User provided a correction
    UserCorrection,
    /// Retry was triggered
    Retry,
}

/// ExecutionLimits - hard stop contracts for autonomous execution
#[derive(Debug, Clone)]
pub struct ExecutionLimits {
    pub max_iterations: u32,
    pub max_runtime_ms: u64,
    pub max_tool_calls: u32,
    pub max_cost: f64,
    pub approval_required_for_side_effects: bool,
    pub completion_criteria: Vec<String>,
    pub failure_threshold: f32,
    // V1.4 verification loop settings
    pub verification_max_iterations: u32,
    pub verification_max_failures: u32,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            max_runtime_ms: 300_000, // 5 minutes
            max_tool_calls: 50,
            max_cost: 1.0, // $1.00
            approval_required_for_side_effects: true,
            completion_criteria: Vec::new(),
            failure_threshold: 0.3,
            verification_max_iterations: 5, // V1.4 default
            verification_max_failures: 3,   // V1.4 default
        }
    }
}

impl ExecutionLimits {
    pub fn with_max_iterations(mut self, max: u32) -> Self {
        self.max_iterations = max;
        self
    }

    pub fn with_max_runtime_ms(mut self, max: u64) -> Self {
        self.max_runtime_ms = max;
        self
    }

    pub fn with_max_tool_calls(mut self, max: u32) -> Self {
        self.max_tool_calls = max;
        self
    }

    pub fn with_max_cost(mut self, max: f64) -> Self {
        self.max_cost = max;
        self
    }

    pub fn with_verification_limits(mut self, max_iterations: u32, max_failures: u32) -> Self {
        self.verification_max_iterations = max_iterations;
        self.verification_max_failures = max_failures;
        self
    }
}

/// WorkOrchestrator - Central service owning the high-level execution loop
pub struct WorkOrchestrator {
    work_context_service: Arc<WorkContextService>,
    playbook_resolver: Arc<super::playbook_resolver::PlaybookResolver>,
    work_execution_service: Arc<WorkExecutionService>,
    intent_classifier: Arc<IntentClassifier>,
    evolution_engine: Arc<EvolutionEngine>,
}

impl WorkOrchestrator {
    pub fn new(
        work_context_service: Arc<WorkContextService>,
        playbook_resolver: Arc<super::playbook_resolver::PlaybookResolver>,
        work_execution_service: Arc<WorkExecutionService>,
        intent_classifier: Arc<IntentClassifier>,
        evolution_engine: Arc<EvolutionEngine>,
    ) -> Self {
        Self {
            work_context_service,
            playbook_resolver,
            work_execution_service,
            intent_classifier,
            evolution_engine,
        }
    }

    /// Submit a user intent to create or attach to a WorkContext
    pub async fn submit_user_intent(
        &self,
        user_id: String,
        message: String,
        conversation_id: Option<String>,
    ) -> Result<WorkContext> {
        // 1. Classify intent
        let classification = self
            .intent_classifier
            .classify_with_override(&message, None)
            .await?;

        // 2. Route to context (create new or attach to existing)
        let mut context = match self.work_context_service.route_context(
            &user_id,
            conversation_id.as_deref(),
            None,
        )? {
            Some(ctx) => ctx,
            None => {
                // Create new context
                let domain = self.infer_domain_from_intent(&classification.intent);
                let mut context = self.work_context_service.create_context(
                    user_id.clone(),
                    self.generate_title(&message),
                    domain,
                    message.clone(),
                )?;

                // Set autonomy level based on intent type
                match classification.intent {
                    crate::intent::Intent::CodingTask | crate::intent::Intent::ProjectAction => {
                        context.autonomy_level = AutonomyLevel::Review;
                    }
                    _ => {
                        context.autonomy_level = AutonomyLevel::Chat;
                    }
                }

                context
            }
        };

        // 3. Attach to conversation if provided
        if let Some(ref conv_id) = conversation_id {
            context.conversation_id = Some(conv_id.clone());
            self.work_context_service
                .set_active_context_for_conversation(conv_id, &context.id)?;
        }

        // 4. Select playbook
        if let Some(playbook) = self.playbook_resolver.resolve_playbook(&context)? {
            // Apply playbook settings
            context.domain_profile_id = Some(playbook.domain_profile_id.clone());
            context.approval_policy = playbook.default_approval_policy;
            context.playbook_id = Some(playbook.id.clone());
            self.work_context_service.update_context(&context)?;

            // Update playbook usage
            self.playbook_resolver.update_playbook_usage(&playbook.id)?;
        }

        // Persist context mutations (autonomy level, conversation binding, playbook settings)
        // before any execution path that reloads context from storage.
        self.work_context_service.update_context(&context)?;

        // 5. Execute flow based on autonomy level
        // Chat mode: create + set AwaitingApproval (no execution)
        // Review mode: execute planning → Await approval
        // Autonomous mode: execute immediately
        if context.autonomy_level == AutonomyLevel::Chat {
            self.work_context_service
                .update_status(&mut context, WorkStatus::AwaitingApproval)?;
            self.work_context_service.update_context(&context)?;
        } else if context.autonomy_level == AutonomyLevel::Review {
            // Review mode: execute planning flow
            self.work_execution_service
                .continue_context(&context.id)
                .await?;

            // Reload context to get updated state
            context = self
                .work_context_service
                .get_context(&context.id)?
                .ok_or_else(|| {
                    anyhow::anyhow!("Context not found after execution: {}", context.id)
                })?;
        } else {
            // Autonomous mode: execute immediately
            self.work_execution_service
                .continue_context(&context.id)
                .await?;

            // Reload context to get updated state
            context = self
                .work_context_service
                .get_context(&context.id)?
                .ok_or_else(|| {
                    anyhow::anyhow!("Context not found after execution: {}", context.id)
                })?;
        }

        Ok(context)
    }

    /// Complete a context and trigger evolution if applicable
    /// Triggers: completion, partial failure, user correction, retry
    pub async fn complete_context(
        &self,
        context_id: String,
        trigger: EvolutionTrigger,
    ) -> Result<WorkContext> {
        let context = self
            .work_context_service
            .get_context(&context_id)?
            .ok_or_else(|| anyhow::anyhow!("Context not found: {}", context_id))?;

        // Evaluate context and store result
        let evaluation_result = EvolutionEngine::evaluate_context(&context);
        let evaluation_json = serde_json::to_value(&evaluation_result)
            .context("Failed to serialize evaluation result")?;

        // Only trigger evolution if playbook is associated
        if let Some(ref playbook_id) = context.playbook_id {
            // Extract patterns from completed context
            let (success_patterns, failure_patterns) = EvolutionEngine::extract_patterns(&context);

            // Evolve playbook based on patterns
            self.evolution_engine.evolve_playbook(
                playbook_id,
                success_patterns,
                failure_patterns,
            )?;
        }

        // Update context status based on trigger
        let mut context = context;
        context.set_evaluation_result(evaluation_json);

        match trigger {
            EvolutionTrigger::Completion => {
                if context.domain == WorkDomain::Software {
                    context.set_harness_metadata(HarnessMetadata {
                        completion_decision: Some(CompletionDecision::Complete),
                        ..Default::default()
                    });
                }
                self.work_context_service
                    .update_status(&mut context, WorkStatus::Completed)?;
            }
            EvolutionTrigger::PartialFailure => {
                self.work_context_service
                    .update_status(&mut context, WorkStatus::Blocked)?;
            }
            EvolutionTrigger::UserCorrection => {
                // User corrected the context, continue execution
                self.work_context_service
                    .clear_blocked_reason(&mut context)?;
            }
            EvolutionTrigger::Retry => {
                // Retry triggered, reset to InProgress
                self.work_context_service
                    .update_status(&mut context, WorkStatus::InProgress)?;
            }
        }

        self.work_context_service.update_context(&context)?;
        Ok(context)
    }

    /// Continue a blocked context
    pub async fn continue_context(&self, context_id: String) -> Result<WorkContext> {
        let mut context = self
            .work_context_service
            .get_context(&context_id)?
            .ok_or_else(|| anyhow::anyhow!("Context not found: {}", context_id))?;

        if context.is_cancelled() {
            anyhow::bail!("cancelled WorkContext is terminal: {context_id}");
        }

        // Clear blocked reason if set, then execute
        if context.is_blocked() {
            self.work_context_service
                .clear_blocked_reason(&mut context)?;
        }

        // Explicit user continuation acts as approval for chat-mode contexts.
        // Promote to Review so execution can proceed through guarded flows.
        if context.autonomy_level == AutonomyLevel::Chat {
            context.autonomy_level = AutonomyLevel::Review;
            self.work_context_service.update_context(&context)?;
        }

        let context = self
            .work_execution_service
            .continue_context(&context_id)
            .await?;

        Ok(context)
    }

    /// Run context until blocked or complete, respecting limits. No external
    /// cancellation signal: the loop still stops gracefully on a durable
    /// `Cancelled` status (observed at the iteration entry guard), which is
    /// how cross-process cancels reach a run.
    pub async fn run_until_blocked_or_complete(
        &self,
        context_id: String,
        limits: ExecutionLimits,
    ) -> Result<WorkContext> {
        self.run_until_blocked_or_complete_with_token(context_id, limits, CancellationToken::new())
            .await
    }

    /// Run context until blocked or complete, observing `token` at every
    /// cancellation checkpoint (#222).
    ///
    /// Cancellation semantics — deliberately cooperative, never an abrupt
    /// kill:
    /// - The in-flight iteration is **never select-dropped**: a tool node
    ///   owns its child process (`tokio::process::Command` without
    ///   `kill_on_drop`), so severing the future at an arbitrary suspension
    ///   point would orphan a live, workspace-mutating process. The
    ///   iteration completes, but `WorkExecutionService` observes
    ///   cancellation at its post-flow pre-persist checkpoint and skips
    ///   every durable write, so a cancelled context gains no post-cancel
    ///   progress and no orphan rows.
    /// - Same-process cancels (`/cancel` fires the registered token) are
    ///   observed at the next checkpoint: the loop top or the iteration's
    ///   post-flow checkpoint.
    /// - Cross-process cancels (another process or server instance flips
    ///   the durable status) are observed by the same checkpoints through
    ///   the status re-reads — graceful polling after control returns to
    ///   the loop, never an immediate mid-step wake. Honest limitation: the
    ///   in-flight step finishes its work (provider calls, tools) before
    ///   the checkpoint discards its results.
    /// - Every graceful stop persists a mandatory `execution_interrupted`
    ///   event (with `checkpoint_ref: null` — the legacy run path has no
    ///   graph checkpoints; the graph-run execution slice owns real
    ///   checkpoint refs). If that evidence insert fails, the run returns
    ///   `Err` — the durable `Cancelled` status and its `context_cancelled`
    ///   event are unaffected, but the missing interruption evidence is
    ///   surfaced, never swallowed.
    /// - Graceful conversion is strictly typed: ONLY the iteration entry
    ///   refusal (`CancelledRefusal`, raised before any write) converts
    ///   into a graceful stop. A persistence error that races a
    ///   concurrent cancel mid-write-sequence — with partial rows already
    ///   written — propagates as a genuine error, never a masked
    ///   cancellation. Likewise, a durable flip that wins the race
    ///   against the loop's own entry converts to a graceful evidenced
    ///   stop ONLY when this run's token fired (the cancel targeted this
    ///   registered run); otherwise the terminal refusal stands.
    pub async fn run_until_blocked_or_complete_with_token(
        &self,
        context_id: String,
        limits: ExecutionLimits,
        token: CancellationToken,
    ) -> Result<WorkContext> {
        let mut context = self
            .work_context_service
            .get_context(&context_id)?
            .ok_or_else(|| anyhow::anyhow!("Context not found: {}", context_id))?;

        if context.is_cancelled() {
            // #222 race repair: if THIS run's token fired, the durable
            // flip targeted this registered run and simply won the race
            // against the loop's entry — the run never started an
            // iteration. That is a graceful stop with mandatory evidence,
            // never a 500 with missing evidence. If the token never fired
            // (plain CLI path, or a cancel from another process before
            // this run registered), the terminal refusal stands.
            if token.is_cancelled() {
                let phase = context.current_phase;
                self.record_execution_interrupted(&context_id, 0, &phase)?;
                return Ok(context);
            }
            anyhow::bail!("cancelled WorkContext cannot be run: {context_id}");
        }

        // Explicit run request is also a human approval signal.
        if context.autonomy_level == AutonomyLevel::Chat {
            context.autonomy_level = AutonomyLevel::Review;
            self.work_context_service.update_context(&context)?;
        }

        let mut iterations = 0;
        let start = std::time::Instant::now();

        loop {
            // Test-deterministic safe point (no-op in production): parks
            // the loop between iterations so tests can flip the durable
            // status and fire the token before the next iteration starts.
            token.park_at_safe_point().await;

            // Check limits
            if iterations >= limits.max_iterations {
                self.work_context_service
                    .set_blocked_reason(&mut context, "Max iterations reached".to_string())?;
                break;
            }

            if start.elapsed().as_millis() as u64 >= limits.max_runtime_ms {
                self.work_context_service
                    .set_blocked_reason(&mut context, "Max runtime exceeded".to_string())?;
                break;
            }

            // Check completion - empty criteria should NOT mean complete
            if context.is_complete()
                || (!context.completion_criteria.is_empty() && context.is_completion_satisfied())
            {
                // Use complete_context() to trigger evaluation and evolution
                context = self
                    .complete_context(context.id.clone(), EvolutionTrigger::Completion)
                    .await?;
                break;
            }

            // Check blocked
            if context.is_blocked() {
                break;
            }

            // Execute next step using WorkExecutionService, observing the
            // token at the iteration's post-flow cancellation checkpoint.
            match self
                .work_execution_service
                .continue_context_with_token(&context.id, &token)
                .await
            {
                Ok(next) => {
                    context = next;
                    iterations += 1;
                    // Cancellation observed by this iteration (post-flow
                    // checkpoint or a completed write sequence racing the
                    // durable flip): graceful stop with mandatory evidence.
                    if context.is_cancelled() {
                        let phase = context.current_phase;
                        self.record_execution_interrupted(&context.id, iterations, &phase)?;
                        break;
                    }
                }
                Err(e) => {
                    // #222 race repair: ONLY the typed pre-write entry
                    // refusal converts to a graceful cancellation - by
                    // construction it is raised before any write of the
                    // iteration, so nothing partial can exist. Any other
                    // error - including a persistence failure racing a
                    // concurrent cancel after partial rows were written -
                    // propagates as a genuine error; the durable Cancelled
                    // status and the last-guard make the state consistent,
                    // and the error is surfaced, never masked.
                    if e.downcast_ref::<super::CancelledRefusal>().is_some() {
                        let fresh = self
                            .work_context_service
                            .get_context(&context_id)?
                            .ok_or_else(|| anyhow::anyhow!("Context not found: {}", context_id))?;
                        let phase = fresh.current_phase;
                        self.record_execution_interrupted(&context_id, iterations, &phase)?;
                        context = fresh;
                        break;
                    }
                    return Err(e);
                }
            }
        }

        Ok(context)
    }

    /// Persist the mandatory `execution_interrupted` durable event for a
    /// run that stopped because its context was cancelled (#222). This is
    /// evidence, not telemetry: an insertion failure propagates as `Err` so
    /// missing interruption evidence is always surfaced, never swallowed.
    ///
    /// `checkpoint_ref` is `null` by contract: the legacy WorkContext run
    /// path produces no graph checkpoints. The graph-run execution slice
    /// (#132 remaining work) owns real checkpoint refs; this slice must
    /// not fabricate one.
    fn record_execution_interrupted(
        &self,
        context_id: &str,
        iterations: u32,
        phase: &WorkPhase,
    ) -> Result<()> {
        let event = super::WorkContextEvent::new(
            uuid::Uuid::new_v4().to_string(),
            context_id.to_string(),
            "execution_interrupted".to_string(),
            serde_json::json!({
                "reason": "cancelled",
                "iterations": iterations,
                "phase": format!("{:?}", phase),
                "checkpoint_ref": Option::<String>::None,
            }),
        );
        let db = self.work_context_service.get_db();
        WorkContextEventOperations::create_event(&**db, &event)
            .map(|_| ())
            .with_context(|| {
                format!(
                    "failed to persist execution_interrupted evidence for work context {context_id}"
                )
            })
    }

    /// V1.4 Verification Loop - run plan → patch → test → failure → re-plan loop
    /// This method implements the verification loop for software development flows
    /// with bounded retries: max_iterations=5, max_failures=3
    pub async fn run_verification_loop(
        &self,
        context_id: String,
        limits: ExecutionLimits,
    ) -> Result<WorkContext> {
        let mut context = self
            .work_context_service
            .get_context(&context_id)?
            .ok_or_else(|| anyhow::anyhow!("Context not found: {}", context_id))?;

        let mut verification_iterations = 0;
        let mut verification_failures = 0;

        loop {
            // Check verification iteration limit
            if verification_iterations >= limits.verification_max_iterations {
                self.work_context_service.set_blocked_reason(
                    &mut context,
                    format!(
                        "Verification max iterations reached: {}",
                        limits.verification_max_iterations
                    ),
                )?;
                break;
            }

            // Check verification failure limit
            if verification_failures >= limits.verification_max_failures {
                self.work_context_service.set_blocked_reason(
                    &mut context,
                    format!(
                        "Verification max failures reached: {}",
                        limits.verification_max_failures
                    ),
                )?;
                break;
            }

            // Execute one iteration of the verification loop
            context = self
                .work_execution_service
                .continue_context(&context.id)
                .await?;

            // Check if tests passed (look for test results in artifacts or evaluation)
            let tests_passed = self.check_tests_passed(&context).await?;

            if tests_passed {
                // Tests passed - complete the context
                context = self
                    .complete_context(context.id.clone(), EvolutionTrigger::Completion)
                    .await?;
                break;
            } else {
                // Tests failed - increment failure count and continue loop
                verification_failures += 1;

                // If we haven't exceeded failure limit, the loop will continue
                // and the flow should re-plan (this is handled by the flow itself)
                if verification_failures >= limits.verification_max_failures {
                    self.work_context_service.set_blocked_reason(
                        &mut context,
                        format!(
                            "Tests failed {} times, exceeding max failures",
                            verification_failures
                        ),
                    )?;
                    break;
                }
            }

            verification_iterations += 1;
        }

        Ok(context)
    }

    /// Check if tests passed by executing real tests
    ///
    /// This method detects project type, runs appropriate tests,
    /// parses results, and stores test artifacts.
    async fn check_tests_passed(&self, context: &WorkContext) -> Result<bool> {
        // Check evaluation result if available (from flow execution)
        if let Some(evaluation) = &context.evaluation_result
            && let Some(success) = evaluation.get("test_success").and_then(|v| v.as_bool())
        {
            return Ok(success);
        }

        // Get project path from context artifacts
        let project_path = self.detect_project_path(context).await?;

        if project_path.is_none() {
            tracing::warn!(
                "No project path found for context {}, skipping real test execution",
                context.id
            );
            return Ok(!context.is_blocked());
        }

        let project_path = project_path.unwrap();

        // Detect project type and run appropriate tests
        let test_result = self.execute_project_tests(&project_path).await?;

        // Store test results in context evaluation
        let mut context = context.clone();
        let evaluation = serde_json::json!({
            "test_success": test_result.success,
            "test_command": test_result.command,
            "test_output": test_result.output,
            "test_errors": test_result.errors,
            "tests_run": test_result.tests_run,
            "tests_passed": test_result.tests_passed,
            "tests_failed": test_result.tests_failed,
            "project_type": test_result.project_type,
            "executed_at": chrono::Utc::now().to_rfc3339(),
        });

        context.evaluation_result = Some(evaluation);
        self.work_context_service.update_context(&context)?;

        // Persist test artifacts
        self.persist_test_artifacts(&context.id, &test_result)
            .await?;

        Ok(test_result.success)
    }

    /// Detect project path from context artifacts
    async fn detect_project_path(
        &self,
        context: &WorkContext,
    ) -> Result<Option<std::path::PathBuf>> {
        // Check context metadata for project path
        if let Some(project_path) = context
            .metadata
            .get("project_path")
            .and_then(|v| v.as_str())
        {
            return Ok(Some(std::path::PathBuf::from(project_path)));
        }
        if let Some(repo_path) = context.metadata.get("repo_path").and_then(|v| v.as_str()) {
            return Ok(Some(std::path::PathBuf::from(repo_path)));
        }

        // Default to current directory if this is a local context
        Ok(Some(std::path::PathBuf::from(".")))
    }

    /// Execute tests based on project type
    async fn execute_project_tests(
        &self,
        project_path: &std::path::Path,
    ) -> Result<TestExecutionResult> {
        use tokio::process::Command;

        // Detect project type
        let project_type = self.detect_project_type(project_path).await?;

        let (test_command, args) = match project_type.as_str() {
            "rust" => ("cargo", vec!["test", "--all-features", "--", "--nocapture"]),
            "node" => ("npm", vec!["test"]),
            "python" => {
                // Try pytest first, fall back to unittest
                if project_path.join("pytest.ini").exists()
                    || project_path.join("pyproject.toml").exists()
                {
                    ("pytest", vec!["-v"])
                } else {
                    ("python", vec!["-m", "unittest", "discover", "-v"])
                }
            }
            "go" => ("go", vec!["test", "-v", "./..."]),
            _ => {
                return Ok(TestExecutionResult {
                    success: false,
                    command: "unknown".to_string(),
                    output: "Unsupported project type".to_string(),
                    errors: vec![format!("Unknown project type: {}", project_type)],
                    tests_run: 0,
                    tests_passed: 0,
                    tests_failed: 0,
                    project_type,
                });
            }
        };

        tracing::info!(
            "Executing tests for {} project at {:?}",
            project_type,
            project_path
        );

        // Execute test command
        let output = Command::new(test_command)
            .args(&args)
            .current_dir(project_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to execute test command: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let full_output = stdout.to_string();

        // Parse test results based on project type
        let (tests_run, tests_passed, tests_failed, errors) =
            self.parse_test_results(&project_type, &stdout, &stderr, output.status.success());

        let success = output.status.success() && tests_failed == 0;

        Ok(TestExecutionResult {
            success,
            command: format!("{} {}", test_command, args.join(" ")),
            output: full_output,
            errors,
            tests_run,
            tests_passed,
            tests_failed,
            project_type,
        })
    }

    /// Detect project type from files in project path
    async fn detect_project_type(&self, project_path: &std::path::Path) -> Result<String> {
        if project_path.join("Cargo.toml").exists() {
            return Ok("rust".to_string());
        }
        if project_path.join("package.json").exists() {
            return Ok("node".to_string());
        }
        if project_path.join("requirements.txt").exists()
            || project_path.join("pyproject.toml").exists()
            || project_path.join("setup.py").exists()
        {
            return Ok("python".to_string());
        }
        if project_path.join("go.mod").exists() {
            return Ok("go".to_string());
        }

        anyhow::bail!("Cannot detect project type at {:?}", project_path)
    }

    /// Parse test output to extract results
    fn parse_test_results(
        &self,
        project_type: &str,
        stdout: &str,
        stderr: &str,
        command_success: bool,
    ) -> (usize, usize, usize, Vec<String>) {
        let mut tests_run = 0usize;
        let mut tests_passed = 0usize;
        let mut tests_failed = 0usize;
        let mut errors = Vec::new();

        match project_type {
            "rust" => {
                // Parse cargo test output
                for line in stdout.lines() {
                    if line.contains("test result:") {
                        // Parse: "test result: ok. 42 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"
                        if let Some(passed_str) = line.split("passed").next()
                            && let Some(num) = passed_str.split_whitespace().last()
                            && let Ok(n) = num.parse::<usize>()
                        {
                            tests_passed += n;
                        }
                        if let Some(failed_str) = line.split("failed").next()
                            && let Some(num) = failed_str.split_whitespace().last()
                            && let Ok(n) = num.parse::<usize>()
                        {
                            tests_failed += n;
                        }
                    }
                    if line.contains("FAILED") || line.contains("error[") {
                        errors.push(line.to_string());
                    }
                }
                tests_run = tests_passed + tests_failed;
            }
            "node" => {
                // Parse npm test output (Jest/Mocha style)
                for line in stdout.lines() {
                    if line.contains("passing") || line.contains("failing") {
                        // Parse Jest/Mocha summary
                        if let Some(passing) = line.split("passing").next()
                            && let Some(n) = passing.split_whitespace().last()
                            && let Ok(num) = n.parse::<usize>()
                        {
                            tests_passed = num;
                        }
                        if line.contains("failing")
                            && let Some(failing) = line.split("failing").next()
                            && let Some(n) = failing.split_whitespace().last()
                            && let Ok(num) = n.parse::<usize>()
                        {
                            tests_failed = num;
                        }
                    }
                    if line.contains("FAIL") || line.contains("Error:") {
                        errors.push(line.to_string());
                    }
                }
                tests_run = tests_passed + tests_failed;
            }
            "python" => {
                // Parse pytest/unittest output
                for line in stdout.lines() {
                    if (line.contains("passed")
                        || line.contains("failed")
                        || line.contains("error"))
                        && let Some(n) = line.split_whitespace().next()
                        && let Ok(num) = n.parse::<usize>()
                    {
                        if line.contains("passed") {
                            tests_passed += num;
                        } else if line.contains("failed") {
                            tests_failed += num;
                            errors.push(line.to_string());
                        }
                    }
                    if line.contains("ERROR") || line.contains("FAILED") {
                        errors.push(line.to_string());
                    }
                }
                tests_run = tests_passed + tests_failed;
            }
            "go" => {
                // Parse go test output
                for line in stdout.lines() {
                    if line.contains("PASS") {
                        tests_passed += 1;
                    } else if line.contains("FAIL") {
                        tests_failed += 1;
                        errors.push(line.to_string());
                    }
                }
                tests_run = tests_passed + tests_failed;
            }
            _ => {}
        }

        if !command_success && tests_failed == 0 {
            // Command failed but no tests detected - likely compilation/setup error
            errors.push(stderr.lines().take(5).collect::<Vec<_>>().join("\n"));
            tests_failed = 1; // Mark as failed
        }

        (tests_run, tests_passed, tests_failed, errors)
    }

    /// Persist test artifacts to database/storage
    async fn persist_test_artifacts(
        &self,
        context_id: &str,
        test_result: &TestExecutionResult,
    ) -> Result<()> {
        // Store test results in a structured format
        let _artifact_data = serde_json::json!({
            "context_id": context_id,
            "test_result": {
                "success": test_result.success,
                "command": test_result.command,
                "output": &test_result.output.chars().take(10000).collect::<String>(), // Limit size
                "errors": test_result.errors,
                "tests_run": test_result.tests_run,
                "tests_passed": test_result.tests_passed,
                "tests_failed": test_result.tests_failed,
                "project_type": test_result.project_type,
            },
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });

        tracing::info!(
            "Test artifacts persisted for context {}: {} tests, {} passed, {} failed",
            context_id,
            test_result.tests_run,
            test_result.tests_passed,
            test_result.tests_failed
        );

        // In production, this would save to artifact repository
        // For now, we log and store in evaluation_result
        Ok(())
    }

    /// Route to the appropriate context based on priority
    pub fn route_to_context(
        &self,
        user_id: &str,
        conversation_id: Option<&str>,
        explicit_context_id: Option<&str>,
    ) -> Result<Option<WorkContext>> {
        self.work_context_service
            .route_context(user_id, conversation_id, explicit_context_id)
    }

    fn infer_domain_from_intent(&self, intent: &Intent) -> super::types::WorkDomain {
        match intent {
            Intent::CodingTask => super::types::WorkDomain::Software,
            Intent::FileEdit => super::types::WorkDomain::Software,
            Intent::ProjectAction => super::types::WorkDomain::Software,
            _ => super::types::WorkDomain::General,
        }
    }

    fn generate_title(&self, message: &str) -> String {
        // Simple title generation - take first 50 chars
        let title = message.chars().take(50).collect::<String>();
        if message.len() > 50 {
            format!("{}...", title)
        } else {
            title
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::db::repository::WorkContextEventOperations;
    use crate::flow::RuntimeContext;
    use crate::flow::execution_service::FlowExecutionService;
    use crate::work::execution_service::WorkExecutionService;
    use crate::work::playbook_resolver::PlaybookResolver;
    use crate::work::service::WorkContextService;
    use crate::work::types::WorkDomain;

    #[test]
    fn test_execution_limits_default() {
        let limits = ExecutionLimits::default();
        assert_eq!(limits.max_iterations, 10);
        assert_eq!(limits.max_runtime_ms, 300_000);
        assert_eq!(limits.max_tool_calls, 50);
        assert_eq!(limits.max_cost, 1.0);
        assert!(limits.approval_required_for_side_effects);
    }

    #[test]
    fn test_execution_limits_builder() {
        let limits = ExecutionLimits::default()
            .with_max_iterations(20)
            .with_max_runtime_ms(600_000);

        assert_eq!(limits.max_iterations, 20);
        assert_eq!(limits.max_runtime_ms, 600_000);
    }

    /// Gated test provider: the flow blocks inside `generate()` until the
    /// test fires `release`, and cancels `arrived` once it is inside
    /// `generate()`. This gives cancellation tests a deterministic
    /// mid-iteration state: the iteration is provably past both entry
    /// checks and in flight, and the test controls exactly when the flow
    /// is allowed to finish. When `fail` is set, the released provider
    /// returns a genuine error instead of a plan — for proving that
    /// genuine errors are never converted into graceful cancellations.
    #[derive(Clone)]
    struct Gate {
        arrived: CancellationToken,
        release: CancellationToken,
        fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    struct GatedProvider {
        gate: Gate,
    }

    #[async_trait::async_trait]
    impl crate::flow::intelligence::LlmProvider for GatedProvider {
        async fn generate(&self, _prompt: &str) -> anyhow::Result<String> {
            self.gate.arrived.cancel();
            self.gate.release.cancelled().await;
            if self.gate.fail.load(std::sync::atomic::Ordering::SeqCst) {
                anyhow::bail!("gated provider failure (injected for race testing)");
            }
            Ok("1. Analyze requirements\n2. Implement changes\n3. Validate with tests".to_string())
        }

        async fn generate_stream(
            &self,
            prompt: &str,
            callback: crate::flow::intelligence::StreamCallback,
        ) -> anyhow::Result<String> {
            let output = self.generate(prompt).await?;
            callback(&output);
            Ok(output)
        }

        fn name(&self) -> &str {
            "gated-test-provider"
        }

        fn model(&self) -> &str {
            "gated-v1"
        }
    }

    impl Gate {
        fn new() -> Self {
            Self {
                arrived: CancellationToken::new(),
                release: CancellationToken::new(),
                fail: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            }
        }
    }

    fn orchestrator_over(
        db: Arc<Db>,
        work_context_service: Arc<WorkContextService>,
        gate: Gate,
    ) -> Arc<WorkOrchestrator> {
        let model_router = Arc::new(crate::flow::intelligence::ModelRouter::new(vec![Box::new(
            GatedProvider { gate },
        )]));
        let runtime = Arc::new(RuntimeContext::default().with_model_router(model_router));
        let flow_execution_service = Arc::new(FlowExecutionService::new(runtime).unwrap());
        let work_execution_service = Arc::new(WorkExecutionService::new(
            work_context_service.clone(),
            flow_execution_service,
        ));
        let playbook_resolver = Arc::new(PlaybookResolver::new(db.clone()));
        let intent_classifier = Arc::new(crate::intent::IntentClassifier::new().unwrap());
        let evolution_engine = Arc::new(crate::work::evolution_engine::EvolutionEngine::new(
            db.clone(),
        ));
        Arc::new(WorkOrchestrator::new(
            work_context_service,
            playbook_resolver,
            work_execution_service,
            intent_classifier,
            evolution_engine,
        ))
    }

    fn setup_orchestrator_gated(
        gate: Gate,
    ) -> (Arc<WorkContextService>, Arc<WorkOrchestrator>, Arc<Db>) {
        let db = Arc::new(Db::in_memory().unwrap());
        let work_context_service = Arc::new(WorkContextService::new(db.clone()));
        let orchestrator = orchestrator_over(db.clone(), work_context_service.clone(), gate);
        (work_context_service, orchestrator, db)
    }

    fn create_context(wcs: &WorkContextService) -> WorkContext {
        let mut context = wcs
            .create_context(
                "user-1".to_string(),
                "Cancellation test".to_string(),
                WorkDomain::General,
                "Goal that will be cancelled".to_string(),
            )
            .unwrap();
        // Review autonomy lets the flow execute (Chat mode would refuse).
        context.autonomy_level = AutonomyLevel::Review;
        wcs.update_context(&context).unwrap();
        context
    }

    fn interrupted_events(db: &Db, context_id: &str) -> Vec<serde_json::Value> {
        WorkContextEventOperations::get_events_for_context(db, context_id)
            .unwrap()
            .into_iter()
            .filter(|e| e.event_type == "execution_interrupted")
            .map(|e| e.data)
            .collect()
    }

    fn count_rows(db: &Db, table: &str) -> i64 {
        db.conn()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// #222 deterministic mid-iteration proof: the cancel lands while the
    /// iteration is provably in flight (the flow is blocked inside the
    /// gated provider, past both entry checks). The run must exit
    /// gracefully with the cancelled context, skip every durable write of
    /// the interrupted iteration (no artifact rows, no flow performance
    /// record, no context rewrite after the flip), and persist the
    /// mandatory execution_interrupted evidence with `checkpoint_ref: null`.
    #[tokio::test]
    async fn run_stops_gracefully_when_cancelled_mid_iteration() {
        let gate = Gate::new();
        let (wcs, orchestrator, db) = setup_orchestrator_gated(gate.clone());
        let context = create_context(&wcs);

        let token = CancellationToken::new();
        let limits = ExecutionLimits::default().with_max_iterations(5);

        let artifacts_before = count_rows(&db, "work_artifacts");
        let performance_before = count_rows(&db, "flow_performance_records");

        let orch = orchestrator.clone();
        let ctx_id = context.id.clone();
        let task_token = token.clone();
        let handle = tokio::spawn(async move {
            orch.run_until_blocked_or_complete_with_token(ctx_id, limits, task_token)
                .await
        });

        // Wait until the iteration is provably in flight (blocked inside
        // the provider) — past the orchestrator and service entry checks.
        gate.arrived.cancelled().await;

        // Production ordering: durable flip first, then the token fire —
        // while the iteration is mid-flight.
        let mut snapshot = wcs.get_context(&context.id).unwrap().unwrap();
        wcs.cancel_context(&mut snapshot, "test cancellation")
            .unwrap();
        let updated_at_after_flip: String = db
            .conn()
            .query_row(
                "SELECT updated_at FROM work_contexts WHERE id = ?1",
                rusqlite::params![context.id],
                |r| r.get(0),
            )
            .unwrap();
        token.cancel();
        // Let the flow finish: the post-flow checkpoint observes the
        // cancellation and skips every write for this iteration.
        gate.release.cancel();

        let result = handle.await.unwrap().unwrap();
        assert!(result.is_cancelled(), "run must exit gracefully, not error");

        // Mandatory durable evidence: exactly one event for this run.
        let events = interrupted_events(&db, &context.id);
        assert_eq!(events.len(), 1, "exactly one execution_interrupted event");
        assert_eq!(events[0]["reason"], "cancelled");
        assert_eq!(
            events[0]["iterations"], 1,
            "the interrupted iteration is counted"
        );
        assert!(
            events[0]["checkpoint_ref"].is_null(),
            "checkpoint_ref must be null — the legacy run path has no graph checkpoints"
        );

        // No orphan rows and no post-cancel progress.
        assert_eq!(count_rows(&db, "work_artifacts"), artifacts_before);
        assert_eq!(
            count_rows(&db, "flow_performance_records"),
            performance_before
        );
        let (status, updated_at): (String, String) = db
            .conn()
            .query_row(
                "SELECT status, updated_at FROM work_contexts WHERE id = ?1",
                rusqlite::params![context.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(status.contains("Cancelled"));
        assert_eq!(
            updated_at, updated_at_after_flip,
            "no context rewrite after the durable flip"
        );

        // No iteration events leaked: only the flip's audit event exists
        // besides the interruption evidence.
        let all = WorkContextEventOperations::get_events_for_context(&*db, &context.id).unwrap();
        let types: Vec<&str> = all.iter().map(|e| e.event_type.as_str()).collect();
        assert!(types.contains(&"context_cancelled"));
        assert!(!types.contains(&"artifact_added"));
        assert!(!types.contains(&"status_changed"));
    }

    /// #222 fail-closed evidence: if the execution_interrupted insert
    /// fails, the graceful exit is refused — the run surfaces Err. The
    /// durable Cancelled state (status + context_cancelled event) is
    /// unaffected by the missing evidence.
    #[tokio::test]
    async fn interrupted_evidence_failure_fails_the_run_loudly() {
        let gate = Gate::new();
        let (wcs, orchestrator, db) = setup_orchestrator_gated(gate.clone());
        let context = create_context(&wcs);

        let token = CancellationToken::new();
        let limits = ExecutionLimits::default().with_max_iterations(5);

        let orch = orchestrator.clone();
        let ctx_id = context.id.clone();
        let task_token = token.clone();
        let handle = tokio::spawn(async move {
            orch.run_until_blocked_or_complete_with_token(ctx_id, limits, task_token)
                .await
        });

        gate.arrived.cancelled().await;

        // Durable flip first (needs the events table intact for its own
        // context_cancelled event), THEN destroy the evidence sink, then
        // wake the run: the interruption evidence cannot be persisted.
        let mut snapshot = wcs.get_context(&context.id).unwrap().unwrap();
        wcs.cancel_context(&mut snapshot, "test cancellation")
            .unwrap();
        db.conn()
            .execute("DROP TABLE work_context_events", [])
            .unwrap();
        token.cancel();
        gate.release.cancel();

        let err = handle.await.unwrap().unwrap_err();
        assert!(
            err.to_string().contains("execution_interrupted"),
            "missing interruption evidence must surface in the error, got: {err}"
        );

        // The durable cancel itself is intact.
        let stored = wcs.get_context(&context.id).unwrap().unwrap();
        assert!(stored.is_cancelled());
    }

    /// #222 retry: after a graceful cancellation exit the context is
    /// terminal — a new run attempt is refused, adds no evidence, and
    /// never resurrects the context.
    #[tokio::test]
    async fn cancelled_context_refuses_subsequent_runs_after_graceful_exit() {
        let gate = Gate::new();
        let (wcs, orchestrator, db) = setup_orchestrator_gated(gate.clone());
        let context = create_context(&wcs);

        let token = CancellationToken::new();
        let limits = ExecutionLimits::default().with_max_iterations(5);

        let orch = orchestrator.clone();
        let ctx_id = context.id.clone();
        let task_token = token.clone();
        let task_limits = limits.clone();
        let handle = tokio::spawn(async move {
            orch.run_until_blocked_or_complete_with_token(ctx_id, task_limits, task_token)
                .await
        });

        gate.arrived.cancelled().await;
        let mut snapshot = wcs.get_context(&context.id).unwrap().unwrap();
        wcs.cancel_context(&mut snapshot, "test cancellation")
            .unwrap();
        token.cancel();
        gate.release.cancel();
        handle.await.unwrap().unwrap();

        // Retry: terminal refusal with the cancelled-context error.
        let retry = orchestrator
            .run_until_blocked_or_complete(context.id.clone(), limits)
            .await;
        assert!(retry.is_err());
        assert!(retry.unwrap_err().to_string().contains("cannot be run"));

        // No new evidence was written for the refused retry.
        assert_eq!(interrupted_events(&db, &context.id).len(), 1);
    }

    /// #222 concurrent runs: two loops on the same context each hold their
    /// own token; one durable flip + both fires stop BOTH runs gracefully,
    /// each persisting its own execution_interrupted evidence, with no
    /// orphan rows from either.
    #[tokio::test]
    async fn concurrent_runs_each_stop_gracefully_with_their_own_evidence() {
        let gate1 = Gate::new();
        let gate2 = Gate::new();
        let (wcs, orchestrator, db) = setup_orchestrator_gated(gate1.clone());
        let context = create_context(&wcs);
        let orchestrator2 = orchestrator_over(db.clone(), wcs.clone(), gate2.clone());

        let token1 = CancellationToken::new();
        let token2 = CancellationToken::new();
        let limits = ExecutionLimits::default().with_max_iterations(5);

        let orch1 = orchestrator.clone();
        let ctx1 = context.id.clone();
        let task_token1 = token1.clone();
        let limits1 = limits.clone();
        let handle1 = tokio::spawn(async move {
            orch1
                .run_until_blocked_or_complete_with_token(ctx1, limits1, task_token1)
                .await
        });
        let orch2 = orchestrator2.clone();
        let ctx2 = context.id.clone();
        let task_token2 = token2.clone();
        let handle2 = tokio::spawn(async move {
            orch2
                .run_until_blocked_or_complete_with_token(ctx2, limits, task_token2)
                .await
        });

        // Both iterations provably in flight (each past its entry checks).
        gate1.arrived.cancelled().await;
        gate2.arrived.cancelled().await;

        // One durable flip; both tokens fired (what registry.fire does).
        let mut snapshot = wcs.get_context(&context.id).unwrap().unwrap();
        wcs.cancel_context(&mut snapshot, "test cancellation")
            .unwrap();
        token1.cancel();
        token2.cancel();
        gate1.release.cancel();
        gate2.release.cancel();

        let result1 = handle1.await.unwrap().unwrap();
        let result2 = handle2.await.unwrap().unwrap();
        assert!(result1.is_cancelled());
        assert!(result2.is_cancelled());

        // Each run persisted exactly its own evidence.
        let events = interrupted_events(&db, &context.id);
        assert_eq!(events.len(), 2, "one execution_interrupted event per run");

        // No orphan rows: neither iteration wrote anything.
        assert_eq!(count_rows(&db, "work_artifacts"), 0);
        assert_eq!(count_rows(&db, "flow_performance_records"), 0);
    }

    /// #222 race repair (binding review): a GENUINE error racing a cancel
    /// must NOT be converted into a graceful cancellation. The iteration
    /// is provably in flight; the durable flip lands mid-iteration; then
    /// the flow itself fails (injected provider failure). The run must
    /// surface the genuine error — not Ok(cancelled) — and must persist NO
    /// execution_interrupted evidence, because nothing was gracefully
    /// interrupted: an error occurred. This is the discrimination that
    /// prevents persistence failures racing a cancel (with partial rows
    /// already written) from being masked as graceful stops.
    #[tokio::test]
    async fn genuine_error_racing_cancel_is_not_converted_to_graceful() {
        let gate = Gate::new();
        let (wcs, orchestrator, db) = setup_orchestrator_gated(gate.clone());
        let context = create_context(&wcs);

        let token = CancellationToken::new();
        let limits = ExecutionLimits::default().with_max_iterations(5);

        let orch = orchestrator.clone();
        let ctx_id = context.id.clone();
        let task_token = token.clone();
        let handle = tokio::spawn(async move {
            orch.run_until_blocked_or_complete_with_token(ctx_id, limits, task_token)
                .await
        });

        // Iteration provably in flight; the durable cancel flips
        // mid-iteration (production ordering: flip, then fire).
        gate.arrived.cancelled().await;
        let mut snapshot = wcs.get_context(&context.id).unwrap().unwrap();
        wcs.cancel_context(&mut snapshot, "test cancellation")
            .unwrap();
        token.cancel();

        // The flow then fails with a genuine error before any write.
        gate.fail.store(true, std::sync::atomic::Ordering::SeqCst);
        gate.release.cancel();

        let err = handle.await.unwrap().unwrap_err();
        assert!(
            !err.to_string().contains("cannot continue"),
            "genuine errors must not surface as the cancellation refusal, got: {err}"
        );

        // No graceful-conversion evidence: the error was real.
        assert_eq!(
            interrupted_events(&db, &context.id).len(),
            0,
            "a genuine error must not be recorded as a graceful cancellation"
        );

        // The durable cancel is intact and nothing was written.
        let stored = wcs.get_context(&context.id).unwrap().unwrap();
        assert!(stored.is_cancelled());
        assert_eq!(count_rows(&db, "work_artifacts"), 0);
        assert_eq!(count_rows(&db, "flow_performance_records"), 0);
    }

    /// #222 race repair (binding review): a cancel that lands AFTER the
    /// run endpoint registered its token but BEFORE the orchestrator's
    /// entry check must exit gracefully WITH mandatory evidence — never a
    /// 500 with missing evidence. Deterministic construction: the flip
    /// and the token fire both happen before the run call enters, which
    /// is exactly the state the race window produces.
    #[tokio::test]
    async fn cancel_between_registration_and_entry_exits_gracefully_with_evidence() {
        let gate = Gate::new();
        let (wcs, orchestrator, db) = setup_orchestrator_gated(gate.clone());
        let context = create_context(&wcs);

        // The API run endpoint registers the token BEFORE calling the
        // orchestrator; the cancel then flips + fires before the loop's
        // entry read. Reproduce that exact state:
        let token = CancellationToken::new();
        let mut snapshot = wcs.get_context(&context.id).unwrap().unwrap();
        wcs.cancel_context(&mut snapshot, "test cancellation")
            .unwrap();
        token.cancel();

        let limits = ExecutionLimits::default().with_max_iterations(5);
        let result = orchestrator
            .run_until_blocked_or_complete_with_token(context.id.clone(), limits, token)
            .await
            .unwrap();

        assert!(result.is_cancelled(), "must be a graceful evidenced stop");

        // Mandatory evidence for the registered run that never started.
        let events = interrupted_events(&db, &context.id);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["reason"], "cancelled");
        assert_eq!(events[0]["iterations"], 0, "no iteration was started");
        assert!(events[0]["checkpoint_ref"].is_null());

        // No work was performed.
        assert_eq!(count_rows(&db, "work_artifacts"), 0);
        assert_eq!(count_rows(&db, "flow_performance_records"), 0);

        // The gate was never reached: the loop refused before the flow.
        assert!(!gate.arrived.is_cancelled());
    }
}
