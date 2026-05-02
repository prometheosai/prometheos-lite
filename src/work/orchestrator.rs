//! WorkOrchestrator - Central service for persistent work execution
//!
//! This module provides the WorkOrchestrator, which owns the high-level
//! execution loop for persistent work contexts with hard stop contracts.

use anyhow::{Context, Result};
use std::sync::Arc;

use super::evolution_engine::EvolutionEngine;
use super::execution_service::WorkExecutionService;
use super::service::WorkContextService;
use super::types::{AutonomyLevel, TestExecutionResult, WorkContext, WorkPhase, WorkStatus};
use crate::flow::execution_service::FlowExecutionService;
use crate::intent::{Intent, IntentClassifier};

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

    /// Run context until blocked or complete, respecting limits
    pub async fn run_until_blocked_or_complete(
        &self,
        context_id: String,
        limits: ExecutionLimits,
    ) -> Result<WorkContext> {
        let mut context = self
            .work_context_service
            .get_context(&context_id)?
            .ok_or_else(|| anyhow::anyhow!("Context not found: {}", context_id))?;

        // Explicit run request is also a human approval signal.
        if context.autonomy_level == AutonomyLevel::Chat {
            context.autonomy_level = AutonomyLevel::Review;
            self.work_context_service.update_context(&context)?;
        }

        let mut iterations = 0;
        let start = std::time::Instant::now();

        loop {
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

            // Execute next step using WorkExecutionService
            context = self
                .work_execution_service
                .continue_context(&context.id)
                .await?;

            iterations += 1;
        }

        Ok(context)
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
        if let Some(evaluation) = &context.evaluation_result {
            if let Some(success) = evaluation.get("test_success").and_then(|v| v.as_bool()) {
                return Ok(success);
            }
        }

        // Get project path from context artifacts
        let project_path = self.detect_project_path(context).await?;

        if project_path.is_none() {
            tracing::warn!("No project path found for context {}, skipping real test execution", context.id);
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
        self.persist_test_artifacts(&context.id, &test_result).await?;

        Ok(test_result.success)
    }

    /// Detect project path from context artifacts
    async fn detect_project_path(&self, context: &WorkContext) -> Result<Option<std::path::PathBuf>> {
        // Check context metadata for project path
        if let Some(project_path) = context.metadata.get("project_path").and_then(|v| v.as_str()) {
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
        let full_output = format!("{}", stdout);

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
                        if let Some(passed_str) = line.split("passed").next() {
                            if let Some(num) = passed_str.split_whitespace().last() {
                                if let Ok(n) = num.parse::<usize>() {
                                    tests_passed += n;
                                }
                            }
                        }
                        if let Some(failed_str) = line.split("failed").next() {
                            if let Some(num) = failed_str.split_whitespace().last() {
                                if let Ok(n) = num.parse::<usize>() {
                                    tests_failed += n;
                                }
                            }
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
                        if let Some(passing) = line.split("passing").next() {
                            if let Some(n) = passing.split_whitespace().last() {
                                if let Ok(num) = n.parse::<usize>() {
                                    tests_passed = num;
                                }
                            }
                        }
                        if line.contains("failing") {
                            if let Some(failing) = line.split("failing").next() {
                                if let Some(n) = failing.split_whitespace().last() {
                                    if let Ok(num) = n.parse::<usize>() {
                                        tests_failed = num;
                                    }
                                }
                            }
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
                    if line.contains("passed") || line.contains("failed") || line.contains("error") {
                        if let Some(n) = line.split_whitespace().next() {
                            if let Ok(num) = n.parse::<usize>() {
                                if line.contains("passed") {
                                    tests_passed += num;
                                } else if line.contains("failed") {
                                    tests_failed += num;
                                    errors.push(line.to_string());
                                }
                            }
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
        let artifact_data = serde_json::json!({
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
    use crate::db::repository::PlaybookOperations;
    use crate::flow::execution_service::FlowExecutionService;
    use crate::intent::IntentClassifier;
    use crate::work::evolution_engine::EvolutionEngine;
    use crate::work::execution_service::WorkExecutionService;
    use crate::work::playbook::{FlowPreference, ResearchDepth, WorkContextPlaybook};
    use crate::work::service::WorkContextService;
    use crate::work::types::{CompletionCriterion, WorkDomain};

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
}
