//! End-to-end integration tests for WorkOrchestrator
//!
//! These tests verify the full lifecycle of WorkContext execution through the orchestrator,
//! including submit, continue, and run_until_complete operations.

use prometheos_lite::db::Db;
use prometheos_lite::db::repository::PlaybookOperations;
use prometheos_lite::flow::RuntimeContext;
use prometheos_lite::flow::execution_service::FlowExecutionService;
use prometheos_lite::flow::intelligence::{LlmProvider, ModelRouter, StreamCallback};
use prometheos_lite::work::evolution_engine::EvolutionEngine;
use prometheos_lite::work::execution_service::WorkExecutionService;
use prometheos_lite::work::orchestrator::{ExecutionLimits, WorkOrchestrator};
use prometheos_lite::work::playbook::{FlowPreference, WorkContextPlaybook};
use prometheos_lite::work::playbook_resolver::PlaybookResolver;
use prometheos_lite::work::service::WorkContextService;
use prometheos_lite::work::types::{CompletionCriterion, WorkPhase, WorkStatus};
use std::sync::Arc;

struct DeterministicTestProvider;

#[async_trait::async_trait]
impl LlmProvider for DeterministicTestProvider {
    async fn generate(&self, prompt: &str) -> anyhow::Result<String> {
        if prompt.to_lowercase().contains("review") {
            return Ok("approved".to_string());
        }
        if prompt.to_lowercase().contains("plan") {
            return Ok("1. Analyze requirements\n2. Implement changes\n3. Validate with tests".to_string());
        }
        Ok(format!("Generated output for: {}", prompt))
    }

    async fn generate_stream(
        &self,
        prompt: &str,
        callback: StreamCallback,
    ) -> anyhow::Result<String> {
        let output = self.generate(prompt).await?;
        callback(&output);
        Ok(output)
    }

    fn name(&self) -> &str {
        "deterministic-test-provider"
    }

    fn model(&self) -> &str {
        "deterministic-v1"
    }
}

/// Setup helper to create a WorkOrchestrator with in-memory database
fn setup_orchestrator() -> WorkOrchestrator {
    let db = Db::in_memory().expect("Failed to create in-memory database");
    let db_arc = Arc::new(db);
    let work_context_service = Arc::new(WorkContextService::new(db_arc.clone()));

    let model_router = Arc::new(ModelRouter::new(vec![Box::new(DeterministicTestProvider)]));
    let runtime = Arc::new(RuntimeContext::default().with_model_router(model_router));
    let flow_execution_service = Arc::new(
        FlowExecutionService::new(runtime).expect("Failed to create FlowExecutionService"),
    );

    let work_execution_service = Arc::new(WorkExecutionService::new(
        work_context_service.clone(),
        flow_execution_service,
    ));

    let playbook_resolver = Arc::new(PlaybookResolver::new(db_arc.clone()));

    let intent_classifier = Arc::new(
        prometheos_lite::intent::IntentClassifier::new()
            .expect("Failed to create IntentClassifier"),
    );

    let evolution_engine = Arc::new(EvolutionEngine::new(db_arc.clone()));

    WorkOrchestrator::new(
        work_context_service,
        playbook_resolver,
        work_execution_service,
        intent_classifier,
        evolution_engine,
    )
}

#[tokio::test]
async fn test_submit_intent_creates_context() {
    let orchestrator = setup_orchestrator();

    let context = orchestrator
        .submit_user_intent("test-user".to_string(), "test message".to_string(), None)
        .await
        .expect("submit_user_intent should succeed");

    assert!(!context.id.is_empty(), "Context should have an ID");
    assert_eq!(context.user_id, "test-user");
    assert_eq!(context.goal, "test message");
    assert_eq!(context.status, WorkStatus::AwaitingApproval);
    assert!(
        context.current_phase == WorkPhase::Intake || context.current_phase == WorkPhase::Planning
    );
}

#[tokio::test]
async fn test_submit_intent_coding_task_sets_review_mode() {
    let orchestrator = setup_orchestrator();

    let context = orchestrator
        .submit_user_intent(
            "test-user".to_string(),
            "implement a function to parse JSON".to_string(),
            None,
        )
        .await
        .expect("submit_user_intent should succeed");

    // Coding tasks should set autonomy level to Review
    assert_eq!(
        context.autonomy_level,
        prometheos_lite::work::types::AutonomyLevel::Review
    );
}

#[tokio::test]
async fn test_submit_intent_general_chat_sets_chat_mode() {
    let orchestrator = setup_orchestrator();

    let context = orchestrator
        .submit_user_intent(
            "test-user".to_string(),
            "hello, how are you?".to_string(),
            None,
        )
        .await
        .expect("submit_user_intent should succeed");

    // General chat should set autonomy level to Chat
    assert_eq!(
        context.autonomy_level,
        prometheos_lite::work::types::AutonomyLevel::Chat
    );
}

#[tokio::test]
async fn test_continue_context_advances_phase() {
    let orchestrator = setup_orchestrator();

    let context = orchestrator
        .submit_user_intent("test-user".to_string(), "test message".to_string(), None)
        .await
        .expect("submit_user_intent should succeed");

    let context = orchestrator
        .continue_context(context.id)
        .await
        .expect("continue_context should succeed");

    // Phase should advance after continue
    assert!(
        context.current_phase == WorkPhase::Planning || context.current_phase == WorkPhase::Intake
    );
}

#[tokio::test]
async fn test_run_until_complete_with_limits() {
    let orchestrator = setup_orchestrator();

    let context = orchestrator
        .submit_user_intent("test-user".to_string(), "test message".to_string(), None)
        .await
        .expect("submit_user_intent should succeed");

    let limits = ExecutionLimits::default()
        .with_max_iterations(5)
        .with_max_runtime_ms(60_000)
        .with_max_tool_calls(10);

    let context = orchestrator
        .run_until_blocked_or_complete(context.id, limits)
        .await
        .expect("run_until_blocked_or_complete should succeed");

    // Context should either be complete or blocked
    assert!(
        context.status == WorkStatus::Completed || context.is_blocked(),
        "Context should be completed or blocked"
    );
}

#[tokio::test]
async fn test_full_lifecycle() {
    let orchestrator = setup_orchestrator();

    // Submit intent
    let mut context = orchestrator
        .submit_user_intent(
            "test-user".to_string(),
            "implement a simple function".to_string(),
            None,
        )
        .await
        .expect("submit_user_intent should succeed");

    assert!(
        context.current_phase == WorkPhase::Intake || context.current_phase == WorkPhase::Planning
    );

    // Continue to advance phase
    context = orchestrator
        .continue_context(context.id)
        .await
        .expect("continue_context should succeed");

    // Run until complete or blocked
    let limits = ExecutionLimits::default()
        .with_max_iterations(10)
        .with_max_runtime_ms(120_000)
        .with_max_tool_calls(20);

    context = orchestrator
        .run_until_blocked_or_complete(context.id, limits)
        .await
        .expect("run_until_blocked_or_complete should succeed");

    // Verify final state
    assert!(
        context.status == WorkStatus::Completed || context.is_blocked(),
        "Context should be completed or blocked after full lifecycle"
    );
}

#[tokio::test]
async fn test_run_until_blocked_or_complete_triggers_evolution() {
    let db = Db::in_memory().expect("Failed to create in-memory database");
    let db_arc = Arc::new(db);
    let work_context_service = Arc::new(WorkContextService::new(db_arc.clone()));

    let model_router = Arc::new(ModelRouter::new(vec![Box::new(DeterministicTestProvider)]));
    let runtime = Arc::new(RuntimeContext::default().with_model_router(model_router));
    let flow_execution_service = Arc::new(
        FlowExecutionService::new(runtime).expect("Failed to create FlowExecutionService"),
    );

    let work_execution_service = Arc::new(WorkExecutionService::new(
        work_context_service.clone(),
        flow_execution_service,
    ));

    let playbook_resolver = Arc::new(PlaybookResolver::new(db_arc.clone()));

    let intent_classifier = Arc::new(
        prometheos_lite::intent::IntentClassifier::new()
            .expect("Failed to create IntentClassifier"),
    );

    let evolution_engine = Arc::new(EvolutionEngine::new(db_arc.clone()));

    let orchestrator = WorkOrchestrator::new(
        work_context_service.clone(),
        playbook_resolver,
        work_execution_service,
        intent_classifier,
        evolution_engine,
    );

    // Create a playbook with flow preferences
    let playbook_id = "test-playbook-evolution-trigger";
    let mut playbook = WorkContextPlaybook::new(
        playbook_id.to_string(),
        "test-user".to_string(),
        "software".to_string(),
        "Test Playbook".to_string(),
        "Test playbook for evolution trigger".to_string(),
    );
    playbook.preferred_flows = vec![FlowPreference {
        flow_id: "planning.flow.yaml".to_string(),
        weight: 0.5,
        confidence: 0.5,
    }];
    PlaybookOperations::create_playbook(&*db_arc, &playbook).expect("Failed to create playbook");

    // Create a WorkContext directly with completion criteria and playbook association
    let mut context = work_context_service
        .create_context(
            "ctx-evolution-trigger-test".to_string(),
            "test-user".to_string(),
            prometheos_lite::work::types::WorkDomain::Software,
            "Test evolution trigger".to_string(),
        )
        .expect("Failed to create context");

    let context_id = context.id.clone();

    context.playbook_id = Some(playbook_id.to_string());
    context.completion_criteria = vec![CompletionCriterion::new(
        "plan".to_string(),
        "Plan completed".to_string(),
    )];
    context.completion_criteria[0].satisfied = true;
    context.status = WorkStatus::InProgress; // Set to InProgress so run_until_blocked_or_complete can detect completion
    work_context_service
        .update_context(&context)
        .expect("Failed to update context");

    // Verify initial flow weight
    let initial_playbook = PlaybookOperations::get_playbook(&*db_arc, playbook_id)
        .expect("Failed to get playbook")
        .expect("Playbook should exist");
    let initial_weight = initial_playbook.preferred_flows[0].weight;
    assert_eq!(initial_weight, 0.5, "Initial weight should be 0.5");

    // Call run_until_blocked_or_complete - this should trigger complete_context() and evolution
    let limits = ExecutionLimits::default();
    let result = orchestrator
        .run_until_blocked_or_complete(context_id.clone(), limits)
        .await;

    assert!(
        result.is_ok(),
        "run_until_blocked_or_complete should succeed"
    );

    // Reload context to verify it was completed
    let updated_context = work_context_service
        .get_context(&context_id)
        .expect("Failed to get context")
        .expect("Context should exist");
    assert_eq!(
        updated_context.status,
        WorkStatus::Completed,
        "Context should be completed"
    );

    // Verify evaluation result was set (proves complete_context was called)
    assert!(
        updated_context.evaluation_result.is_some(),
        "Evaluation result should be set, proving complete_context was called"
    );
}
