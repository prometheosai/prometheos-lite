//! V1.5 Evaluation System Tests
//!
//! Tests for enhanced evaluation with structural validation and semantic evaluation.

use chrono::Utc;
use prometheos::work::evaluation::{EvaluationDimensions, EvaluationEngine, StructuralValidation};
use prometheos::work::types::{WorkContext, WorkContextStatus};

fn create_test_work_context(task: &str) -> WorkContext {
    WorkContext {
        id: uuid::Uuid::new_v4().to_string(),
        task: task.to_string(),
        status: WorkContextStatus::Completed,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        completed_at: None,
        input: serde_json::json!({}),
        output: Some(serde_json::json!({})),
        artifacts: Vec::new(),
        playbook_id: None,
        evaluation_result: None,
        execution_metadata: None,
    }
}

#[test]
fn test_evaluation_dimensions_overall_score() {
    let dimensions = EvaluationDimensions {
        correctness: 0.8,
        completeness: 0.9,
        efficiency: 0.7,
        reliability: 0.85,
    };

    let overall = dimensions.overall_score();
    assert!(overall > 0.0 && overall <= 1.0);
}

#[test]
fn test_evaluation_dimensions_default() {
    let dimensions = EvaluationDimensions::default();
    assert_eq!(dimensions.correctness, 0.5);
    assert_eq!(dimensions.completeness, 0.5);
    assert_eq!(dimensions.efficiency, 0.5);
    assert_eq!(dimensions.reliability, 0.5);
}

#[test]
fn test_structural_validation_valid() {
    let engine = EvaluationEngine::default();
    
    let context = create_test_work_context("Test task");
    let execution_metadata = serde_json::json!({});
    let validation = engine.validate_structure(&context, &execution_metadata).unwrap();
    
    assert!(validation.is_valid);
    assert!(validation.errors.is_empty());
}

#[test]
fn test_structural_validation_invalid() {
    let engine = EvaluationEngine::default();
    
    let context = create_test_work_context(""); // Empty task
    let execution_metadata = serde_json::json!({});
    let validation = engine.validate_structure(&context, &execution_metadata).unwrap();
    
    assert!(!validation.is_valid);
    assert!(!validation.errors.is_empty());
}

#[test]
fn test_structural_validation_invalid_patch() {
    let engine = EvaluationEngine::default();
    
    let context = create_test_work_context("Test task");
    let execution_metadata = serde_json::json!({
        "patch": "not an object"
    });
    let validation = engine.validate_structure(&context, &execution_metadata).unwrap();
    
    assert!(!validation.is_valid);
    assert!(!validation.errors.is_empty());
}

#[test]
fn test_evaluate_tool_consistency() {
    let engine = EvaluationEngine::default();
    
    let metadata = serde_json::json!({
        "tool_calls": [
            {"success": true},
            {"success": true},
            {"success": false}
        ]
    });

    let score = engine.evaluate_tool_consistency(&metadata).unwrap();
    assert!(score < 1.0); // Should be less than 1.0 due to failure
}

#[test]
fn test_evaluate_tool_consistency_all_success() {
    let engine = EvaluationEngine::default();
    
    let metadata = serde_json::json!({
        "tool_calls": [
            {"success": true},
            {"success": true}
        ]
    });

    let score = engine.evaluate_tool_consistency(&metadata).unwrap();
    assert_eq!(score, 1.0); // Perfect consistency
}

#[test]
fn test_evaluate_artifact_completeness() {
    let engine = EvaluationEngine::default();
    
    let context = WorkContext {
        id: uuid::Uuid::new_v4().to_string(),
        task: "Test task".to_string(),
        status: WorkContextStatus::Completed,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        completed_at: None,
        input: serde_json::json!({}),
        output: Some(serde_json::json!({})),
        artifacts: vec![
            prometheos::work::Artifact {
                id: "1".to_string(),
                kind: "code".to_string(),
                content: "test".to_string(),
                created_at: Utc::now(),
            },
            prometheos::work::Artifact {
                id: "2".to_string(),
                kind: "".to_string(), // Incomplete
                content: "".to_string(),
                created_at: Utc::now(),
            },
        ],
        playbook_id: None,
        evaluation_result: None,
        execution_metadata: None,
    };

    let score = engine.evaluate_artifact_completeness(&context).unwrap();
    assert!(score == 0.5); // 1 out of 2 complete
}

#[test]
fn test_evaluate_artifact_completeness_no_artifacts() {
    let engine = EvaluationEngine::default();
    
    let context = create_test_work_context("Test task");
    let score = engine.evaluate_artifact_completeness(&context).unwrap();
    
    assert_eq!(score, 0.5); // Neutral if no artifacts
}

#[test]
fn test_evaluation_engine_default() {
    let engine = EvaluationEngine::default();
    // Should create without errors
}

#[test]
fn test_evaluation_scoring_correctness() {
    let engine = EvaluationEngine::default();
    
    let context = create_test_work_context("Test task");
    let execution_metadata = serde_json::json!({
        "retry_count": 0,
        "test_results": {"failed": 0}
    });

    let result = engine.evaluate(&context, &execution_metadata).await.unwrap();
    
    assert!(result.overall_score > 0.0);
    assert!(result.semantic_score > 0.0);
    assert!(result.structural_score > 0.0);
    assert!(result.tool_consistency_score > 0.0);
    assert!(result.artifact_completeness_score > 0.0);
}

#[test]
fn test_evaluation_with_high_retries() {
    let engine = EvaluationEngine::default();
    
    let context = create_test_work_context("Test task");
    let execution_metadata = serde_json::json!({
        "retry_count": 5,
        "test_results": {"failed": 0}
    });

    let result = engine.evaluate(&context, &execution_metadata).await.unwrap();
    
    // Should have penalty for high retries
    assert!(!result.penalties.is_empty());
    assert!(result.overall_score < 1.0);
}

#[test]
fn test_evaluation_with_failed_tests() {
    let engine = EvaluationEngine::default();
    
    let context = create_test_work_context("Test task");
    let execution_metadata = serde_json::json!({
        "retry_count": 0,
        "test_results": {"failed": 3}
    });

    let result = engine.evaluate(&context, &execution_metadata).await.unwrap();
    
    // Should have penalty for failed tests
    assert!(!result.penalties.is_empty());
    assert!(result.overall_score < 1.0);
}
