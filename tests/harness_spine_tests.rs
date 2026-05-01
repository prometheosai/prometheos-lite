//! Integration tests for Harness Spine MVP (Slice 1)
//!
//! Tests for WorkOrchestrator, PlaybookResolver, and ContextLoaderNode fix

use prometheos_lite::db::Db;
use prometheos_lite::flow::RuntimeContext;
use prometheos_lite::flow::execution_service::FlowExecutionService;
use prometheos_lite::intent::IntentClassifier;
use prometheos_lite::work::{
    ExecutionLimits, PlaybookResolver, WorkContextService, WorkOrchestrator,
};
use std::sync::Arc;

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
        .with_max_runtime_ms(600_000)
        .with_max_tool_calls(100)
        .with_max_cost(2.0);

    assert_eq!(limits.max_iterations, 20);
    assert_eq!(limits.max_runtime_ms, 600_000);
    assert_eq!(limits.max_tool_calls, 100);
    assert_eq!(limits.max_cost, 2.0);
}

#[test]
fn test_playbook_resolver_resolve_playbook() {
    let db = Db::in_memory().unwrap();
    let resolver = PlaybookResolver::new(Arc::new(db));

    let context = prometheos_lite::work::WorkContext::new(
        "ctx-1".to_string(),
        "user-1".to_string(),
        "Build API".to_string(),
        prometheos_lite::work::WorkDomain::Software,
        "Create a REST API".to_string(),
    );

    // Should return None when no playbooks exist
    let result = resolver.resolve_playbook(&context).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_work_orchestrator_route_to_context() {
    let db = Arc::new(Db::in_memory().unwrap());
    let work_context_service = Arc::new(WorkContextService::new(db.clone()));
    let playbook_resolver = Arc::new(PlaybookResolver::new(db.clone()));
    let runtime = Arc::new(RuntimeContext::default());
    let flow_execution_service = Arc::new(FlowExecutionService::new(runtime).unwrap());
    let intent_classifier = IntentClassifier::new().unwrap();
    let orchestrator = WorkOrchestrator::new(
        work_context_service,
        playbook_resolver,
        flow_execution_service,
        intent_classifier,
    );

    // Test routing with no existing context
    let result = orchestrator.route_to_context("user-1", None, None).unwrap();
    assert!(result.is_none());

    // Test routing with explicit context ID (should return None if context doesn't exist)
    let result = orchestrator
        .route_to_context("user-1", None, Some("nonexistent"))
        .unwrap();
    assert!(result.is_none());
}

#[test]
fn test_work_context_service_create_and_get() {
    let db = Arc::new(Db::in_memory().unwrap());
    let service = WorkContextService::new(db);

    let context = service
        .create_context(
            "user-1".to_string(),
            "Test Context".to_string(),
            prometheos_lite::work::WorkDomain::Software,
            "Test goal".to_string(),
        )
        .unwrap();

    assert_eq!(context.title, "Test Context");
    assert_eq!(context.domain, prometheos_lite::work::WorkDomain::Software);
    assert_eq!(context.status, prometheos_lite::work::WorkStatus::Draft);

    let retrieved = service.get_context(&context.id).unwrap().unwrap();
    assert_eq!(retrieved.id, context.id);
    assert_eq!(retrieved.title, context.title);
}

#[test]
fn test_work_context_service_update_status() {
    let db = Arc::new(Db::in_memory().unwrap());
    let service = WorkContextService::new(db);

    let mut context = service
        .create_context(
            "user-1".to_string(),
            "Test Context".to_string(),
            prometheos_lite::work::WorkDomain::Software,
            "Test goal".to_string(),
        )
        .unwrap();

    service
        .update_status(&mut context, prometheos_lite::work::WorkStatus::InProgress)
        .unwrap();

    assert_eq!(
        context.status,
        prometheos_lite::work::WorkStatus::InProgress
    );

    let retrieved = service.get_context(&context.id).unwrap().unwrap();
    assert_eq!(
        retrieved.status,
        prometheos_lite::work::WorkStatus::InProgress
    );
}

#[test]
fn test_playbook_repository_increment_usage() {
    let db = Db::in_memory().unwrap();
    let playbook = prometheos_lite::work::WorkContextPlaybook::new(
        "pb-1".to_string(),
        "user-1".to_string(),
        "software".to_string(),
        "Software Playbook".to_string(),
        "For software work".to_string(),
    );

    let created =
        prometheos_lite::db::repository::PlaybookOperations::create_playbook(&db, &playbook)
            .unwrap();
    assert_eq!(created.usage_count, 0);

    prometheos_lite::db::repository::PlaybookOperations::increment_usage_count(&db, &created.id)
        .unwrap();

    let updated =
        prometheos_lite::db::repository::PlaybookOperations::get_playbook(&db, &created.id)
            .unwrap()
            .unwrap();
    assert_eq!(updated.usage_count, 1);
}

#[test]
fn test_playbook_repository_update_confidence() {
    let db = Db::in_memory().unwrap();
    let playbook = prometheos_lite::work::WorkContextPlaybook::new(
        "pb-1".to_string(),
        "user-1".to_string(),
        "software".to_string(),
        "Software Playbook".to_string(),
        "For software work".to_string(),
    );

    let created =
        prometheos_lite::db::repository::PlaybookOperations::create_playbook(&db, &playbook)
            .unwrap();
    assert_eq!(created.confidence, 0.5);

    prometheos_lite::db::repository::PlaybookOperations::update_confidence(&db, &created.id, 0.8)
        .unwrap();

    let updated =
        prometheos_lite::db::repository::PlaybookOperations::get_playbook(&db, &created.id)
            .unwrap()
            .unwrap();
    assert_eq!(updated.confidence, 0.8);
}
