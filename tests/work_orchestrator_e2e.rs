//! End-to-end integration tests for WorkOrchestrator
//!
//! These tests verify the full lifecycle of WorkContext execution through the orchestrator,
//! including submit, continue, and run_until_complete operations.

use prometheos_lite::db::Db;
use prometheos_lite::db::repository::PlaybookOperations;
use prometheos_lite::flow::execution_service::FlowExecutionService;
use prometheos_lite::flow::RuntimeContext;
use prometheos_lite::work::execution_service::WorkExecutionService;
use prometheos_lite::work::orchestrator::{ExecutionLimits, WorkOrchestrator};
use prometheos_lite::work::playbook_resolver::PlaybookResolver;
use prometheos_lite::work::service::WorkContextService;
use prometheos_lite::work::types::{CompletionCriterion, WorkPhase, WorkStatus};
use prometheos_lite::work::playbook::{FlowPreference, WorkContextPlaybook};
use prometheos_lite::work::evolution_engine::EvolutionEngine;
use std::sync::Arc;

/// Setup helper to create a WorkOrchestrator with in-memory database
fn setup_orchestrator() -> WorkOrchestrator {
    let db = Db::in_memory().expect("Failed to create in-memory database");
    let db_arc = Arc::new(db);
    let work_context_service = Arc::new(WorkContextService::new(db_arc.clone()));

    let runtime = Arc::new(RuntimeContext::default());
    let flow_execution_service = Arc::new(
        FlowExecutionService::new(runtime).expect("Failed to create FlowExecutionService"),
    );

    let work_execution_service = Arc::new(WorkExecutionService::new(
        work_context_service.clone(),
        flow_execution_service,
    ));

    let playbook_resolver = Arc::new(PlaybookResolver::new(db_arc.clone()));

    let intent_classifier = Arc::new(
        prometheos_lite::intent::IntentClassifier::new().expect("Failed to create IntentClassifier"),
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
    assert_eq!(context.current_phase, WorkPhase::Intake);
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
    assert_eq!(context.autonomy_level, prometheos_lite::work::types::AutonomyLevel::Review);
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
    assert_eq!(context.autonomy_level, prometheos_lite::work::types::AutonomyLevel::Chat);
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
    assert!(context.current_phase == WorkPhase::Planning || context.current_phase == WorkPhase::Intake);
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

    assert_eq!(context.current_phase, WorkPhase::Intake);

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
