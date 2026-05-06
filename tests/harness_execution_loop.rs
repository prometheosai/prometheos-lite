//! Issue 6: Harness Execution Loop Tests
//!
//! Comprehensive tests for the Harness Execution Loop including:
//! - HarnessExecutionRequest struct creation and validation
//! - HarnessMode enum variants (Review, ReviewOnly, Assisted)
//! - ValidationFailurePolicy (KeepPatchAndRequestApproval, RollbackPatch)
//! - HarnessLimits struct (max_steps, max_time_ms, max_cost_usd)
//! - HarnessExecutionResult struct (success, failure, partial)
//! - ExecutionMetrics struct (timing breakdown)
//! - HarnessProgress enum variants
//! - Cost estimation functions
//! - Resource limit checking

use std::collections::HashMap;
use std::path::PathBuf;
use chrono::Utc;

use prometheos_lite::harness::execution_loop::{
    check_resource_limits, estimate_execution_cost, ExecutionMetrics,
    HarnessExecutionRequest, HarnessExecutionResult, HarnessLimits, HarnessProgress, ValidationFailurePolicy,
};
use prometheos_lite::harness::{
    RiskAssessment, ConfidenceScore, VerificationStrength, CompletionDecision, Trajectory,
    EvidenceLog, FailureKind, ValidationFailurePolicy as HarnessValidationFailurePolicy,
    RiskLevel, RepoContext, EnvironmentProfile, FileSet, DependencyGraph,
};
use prometheos_lite::harness::mode_policy::HarnessMode;

// ============================================================================
// HarnessExecutionRequest Tests
// ============================================================================

#[test]
fn test_harness_execution_request_creation() {
    let request = HarnessExecutionRequest {
        work_context_id: "ctx-123".to_string(),
        repo_root: PathBuf::from("/test/repo"),
        task: "Fix bug in main.rs".to_string(),
        requirements: vec![],
        acceptance_criteria: vec![],
        mode: HarnessMode::Review,
        limits: HarnessLimits::default(),
        mentioned_files: vec![],
        mentioned_symbols: vec![],
        proposed_edits: vec![],
        patch_provider: None,
        provider_context: None,
        progress_callback: None,
        validation_failure_policy: HarnessValidationFailurePolicy::RollbackAutomatically,
    };

    assert_eq!(request.work_context_id, "ctx-123");
    assert_eq!(request.repo_root, PathBuf::from("/test/repo"));
    assert_eq!(request.task, "Fix bug in main.rs");
    assert_eq!(request.mode, HarnessMode::Review);
}

#[test]
fn test_harness_execution_request_with_hints() {
    let request = HarnessExecutionRequest {
        work_context_id: "ctx-456".to_string(),
        repo_root: PathBuf::from("/test/repo"),
        task: "Refactor code".to_string(),
        requirements: vec![],
        acceptance_criteria: vec![],
        mode: HarnessMode::Assisted,
        limits: HarnessLimits::default(),
        mentioned_files: vec![PathBuf::from("src/main.rs"), PathBuf::from("src/lib.rs")],
        mentioned_symbols: vec![],
        proposed_edits: vec![],
        patch_provider: None,
        provider_context: None,
        progress_callback: None,
        validation_failure_policy: HarnessValidationFailurePolicy::RollbackAutomatically,
    };

    assert_eq!(request.mentioned_files.len(), 2);
}

// ============================================================================
// HarnessMode Tests
// ============================================================================

#[test]
fn test_harness_mode_variants() {
    assert!(matches!(HarnessMode::Review, HarnessMode::Review));
    assert!(matches!(HarnessMode::ReviewOnly, HarnessMode::ReviewOnly));
    assert!(matches!(HarnessMode::Assisted, HarnessMode::Assisted));
}

#[test]
fn test_harness_mode_display() {
    assert_eq!(format!("{:?}", HarnessMode::Review), "Review");
    assert_eq!(format!("{:?}", HarnessMode::ReviewOnly), "ReviewOnly");
    assert_eq!(format!("{:?}", HarnessMode::Assisted), "Assisted");
}

// ============================================================================
// ValidationFailurePolicy Tests
// ============================================================================

#[test]
fn test_validation_failure_policy_variants() {
    assert!(matches!(
        HarnessValidationFailurePolicy::KeepPatchAndRequestApproval,
        HarnessValidationFailurePolicy::KeepPatchAndRequestApproval
    ));
    assert!(matches!(
        HarnessValidationFailurePolicy::RollbackAutomatically,
        HarnessValidationFailurePolicy::RollbackAutomatically
    ));
}

#[test]
fn test_validation_failure_policy_default() {
    let policy: HarnessValidationFailurePolicy = Default::default();
    assert!(matches!(
        policy,
        HarnessValidationFailurePolicy::KeepPatchAndRequestApproval
    ));
}

// ============================================================================
// HarnessLimits Tests
// ============================================================================

#[test]
fn test_harness_limits_default() {
    let limits = HarnessLimits::default();

    assert!(limits.max_steps > 0);
    assert!(limits.max_time_ms > 0);
    assert!(limits.max_cost_usd > 0.0);
}

#[test]
fn test_harness_limits_custom() {
    let limits = HarnessLimits {
        max_steps: 50,
        max_time_ms: 300_000, // 5 minutes
        max_cost_usd: 5.0,
        max_file_size_bytes: Some(1048576),
        max_patch_attempts: 3,
        max_tokens: Some(100000),
    };

    assert_eq!(limits.max_steps, 50);
    assert_eq!(limits.max_time_ms, 300_000);
    assert_eq!(limits.max_cost_usd, 5.0);
}

#[test]
fn test_harness_limits_clone() {
    let limits = HarnessLimits::default();
    let cloned = limits.clone();

    assert_eq!(limits.max_steps, cloned.max_steps);
    assert_eq!(limits.max_time_ms, cloned.max_time_ms);
}

// ============================================================================
// HarnessExecutionResult Tests
// ============================================================================

#[test]
fn test_harness_execution_result_success() {
    let result = HarnessExecutionResult {
        work_context_id: "ctx-123".to_string(),
        trace_id: Some("trace-456".to_string()),
        repo_context: RepoContext {
            root: PathBuf::from("/test"),
            ranked_files: vec![],
            symbols: vec![],
            relationships: vec![],
            compressed_context: String::new(),
            token_estimate: 0,
            language_breakdown: HashMap::new(),
            dependency_graph: DependencyGraph::default(),
        },
        environment: EnvironmentProfile::default(),
        file_set: FileSet::default(),
        acceptance: vec![],
        patch_result: None,
        validation_result: None,
        review_issues: vec![],
        risk_assessment: RiskAssessment {
            level: RiskLevel::None,
            reasons: vec![],
            requires_approval: false,
            can_override: true,
            override_conditions: vec![],
        },
        confidence: ConfidenceScore {
            score: 0.5,
            factors: vec![],
            explanation: "Default confidence".to_string(),
            recommendation: None,
        },
        verification_strength: VerificationStrength::None,
        completion_decision: CompletionDecision::Complete,
        trajectory: Trajectory {
            id: "test-trajectory".to_string(),
            work_context_id: "ctx-123".to_string(),
            steps: vec![],
            started_at: chrono::Utc::now(),
            completed_at: None,
            metadata: None,
        },
        git_checkpoint: None,
        rollback_handle: None,
        validation_failure_policy: HarnessValidationFailurePolicy::default(),
        artifacts: vec![],
        failures: vec![],
        summary: "Test execution".to_string(),
        execution_metrics: ExecutionMetrics::default(),
        step_count: 1,
        terminated_early: false,
        termination_reason: None,
        evidence_log: EvidenceLog::default(),
    };

    assert_eq!(result.work_context_id, "ctx-123");
    assert_eq!(result.trace_id, Some("trace-456".to_string()));
    assert_eq!(result.step_count, 1);
    assert!(!result.terminated_early);
    assert!(result.failures.is_empty());
}

#[test]
fn test_harness_execution_result_failure() {
    let result = HarnessExecutionResult {
        work_context_id: "ctx-789".to_string(),
        trace_id: None,
        repo_context: RepoContext {
            root: PathBuf::from("/test"),
            ranked_files: vec![],
            symbols: vec![],
            relationships: vec![],
            compressed_context: String::new(),
            token_estimate: 0,
            language_breakdown: HashMap::new(),
            dependency_graph: DependencyGraph::default(),
        },
        environment: EnvironmentProfile::default(),
        file_set: FileSet::default(),
        acceptance: vec![],
        patch_result: None,
        validation_result: None,
        review_issues: vec![],
        risk_assessment: RiskAssessment {
            level: RiskLevel::None,
            reasons: vec![],
            requires_approval: false,
            can_override: true,
            override_conditions: vec![],
        },
        confidence: ConfidenceScore {
            score: 0.5,
            factors: vec![],
            explanation: "Default confidence".to_string(),
            recommendation: None,
        },
        verification_strength: VerificationStrength::None,
        completion_decision: CompletionDecision::Complete,
        trajectory: Trajectory {
            id: "test-trajectory".to_string(),
            work_context_id: "ctx-123".to_string(),
            steps: vec![],
            started_at: chrono::Utc::now(),
            completed_at: None,
            metadata: None,
        },
        git_checkpoint: None,
        rollback_handle: None,
        validation_failure_policy: HarnessValidationFailurePolicy::default(),
        artifacts: vec![],
        failures: vec![FailureKind::ValidationFailed],
        summary: "Validation failed".to_string(),
        execution_metrics: ExecutionMetrics::default(),
        step_count: 1,
        terminated_early: true,
        termination_reason: None,
        evidence_log: EvidenceLog::default(),
    };

    assert!(!result.failures.is_empty());
    assert!(result.terminated_early);
}

#[test]
fn test_harness_execution_result_failure_with_termination_reason() {
    let result = HarnessExecutionResult {
        work_context_id: "ctx-789".to_string(),
        trace_id: None,
        repo_context: RepoContext {
            root: PathBuf::from("/test"),
            ranked_files: vec![],
            symbols: vec![],
            relationships: vec![],
            compressed_context: String::new(),
            token_estimate: 0,
            language_breakdown: HashMap::new(),
            dependency_graph: DependencyGraph::default(),
        },
        environment: EnvironmentProfile::default(),
        file_set: FileSet::default(),
        acceptance: vec![],
        patch_result: None,
        validation_result: None,
        review_issues: vec![],
        risk_assessment: RiskAssessment {
            level: RiskLevel::None,
            reasons: vec![],
            requires_approval: false,
            can_override: true,
            override_conditions: vec![],
        },
        confidence: ConfidenceScore {
            score: 0.5,
            factors: vec![],
            explanation: "Default confidence".to_string(),
            recommendation: None,
        },
        verification_strength: VerificationStrength::None,
        completion_decision: CompletionDecision::Complete,
        trajectory: Trajectory {
            id: "test-trajectory".to_string(),
            work_context_id: "ctx-123".to_string(),
            steps: vec![],
            started_at: chrono::Utc::now(),
            completed_at: None,
            metadata: None,
        },
        git_checkpoint: None,
        rollback_handle: None,
        validation_failure_policy: HarnessValidationFailurePolicy::default(),
        artifacts: vec![],
        failures: vec![],
        summary: "Failed execution".to_string(),
        execution_metrics: ExecutionMetrics::default(),
        step_count: 0,
        terminated_early: true,
        termination_reason: Some("Test failure".to_string()),
        evidence_log: EvidenceLog::default(),
    };

    assert!(!result.failures.is_empty());
    assert!(result.terminated_early);
}

// ============================================================================
// ExecutionMetrics Tests
// ============================================================================

#[test]
fn test_execution_metrics_default() {
    let metrics = ExecutionMetrics::default();

    assert_eq!(metrics.total_duration_ms, 0);
    assert_eq!(metrics.repo_analysis_ms, 0);
    assert_eq!(metrics.patch_generation_ms, 0);
}

#[test]
fn test_execution_metrics_with_values() {
    let metrics = ExecutionMetrics {
        total_duration_ms: 10_000,
        repo_analysis_ms: 2_000,
        patch_generation_ms: 5_000,
        validation_ms: 3_000,
        review_ms: 1_000,
        cost_estimate_usd: 0.50,
        tokens_used: 1000,
        files_modified: 5,
        lines_changed: 50,
    };

    assert_eq!(metrics.total_duration_ms, 10_000);
    assert_eq!(metrics.repo_analysis_ms, 2_000);
    assert_eq!(metrics.patch_generation_ms, 5_000);
    assert_eq!(metrics.validation_ms, 3_000);
}

// ============================================================================
// HarnessProgress Tests
// ============================================================================

#[test]
fn test_harness_progress_started() {
    let progress = HarnessProgress::Started {
        work_context_id: "ctx-123".to_string(),
        step: 1,
        total_steps: 10,
    };

    assert!(matches!(progress, HarnessProgress::Started { .. }));
}

#[test]
fn test_harness_progress_repo_analysis() {
    let progress = HarnessProgress::RepoAnalysis {
        files_found: 42,
        symbols_found: 100,
    };

    assert!(matches!(progress, HarnessProgress::RepoAnalysis { .. }));
}

#[test]
fn test_harness_progress_patch_generated() {
    let progress = HarnessProgress::PatchGenerated {
        files_changed: 3,
        total_files: 10,
    };

    assert!(matches!(progress, HarnessProgress::PatchGenerated { .. }));
}

#[test]
fn test_harness_progress_finished() {
    let progress = HarnessProgress::Finished {
        success: true,
        duration_ms: 1000,
    };

    assert!(matches!(progress, HarnessProgress::Finished { .. }));
}

// ============================================================================
// Cost Estimation Tests
// ============================================================================

#[test]
fn test_estimate_execution_cost_basic() {
    let limits = HarnessLimits::default();
    let cost = estimate_execution_cost(&limits, 10);

    assert!(cost > 0.0);
}

#[test]
fn test_estimate_execution_cost_zero_files() {
    let limits = HarnessLimits::default();
    let cost = estimate_execution_cost(&limits, 0);

    assert!(cost > 0.0); // Base cost still applies
}

#[test]
fn test_estimate_execution_cost_scales_with_files() {
    let limits = HarnessLimits::default();
    let cost_10 = estimate_execution_cost(&limits, 10);
    let cost_100 = estimate_execution_cost(&limits, 100);

    assert!(cost_100 > cost_10);
}

// ============================================================================
// Resource Limits Tests
// ============================================================================

#[test]
fn test_check_resource_limits_within_bounds() {
    let limits = HarnessLimits {
        max_steps: 100,
        max_time_ms: 60_000,
        max_cost_usd: 10.0,
        max_patch_attempts: 3,
        max_tokens: Some(100000),
        max_file_size_bytes: Some(1048576),
    };

    let files: Vec<PathBuf> = (0..10).map(|i| PathBuf::from(format!("file{}.rs", i))).collect();
    let result = check_resource_limits(&limits, &files);

    assert!(result.is_ok());
}

#[test]
fn test_check_resource_limits_too_many_files() {
    let limits = HarnessLimits {
        max_steps: 10,
        max_time_ms: 60_000,
        max_cost_usd: 1.0,
        max_patch_attempts: 1,
        max_tokens: Some(50000),
        max_file_size_bytes: Some(512000),
    };

    let files: Vec<PathBuf> = (0..500).map(|i| PathBuf::from(format!("file{}.rs", i))).collect();
    let result = check_resource_limits(&limits, &files);

    assert!(result.is_err());
}

#[test]
fn test_check_resource_limits_empty_files() {
    let limits = HarnessLimits::default();
    let files: Vec<PathBuf> = vec![];

    let result = check_resource_limits(&limits, &files);
    assert!(result.is_ok());
}
