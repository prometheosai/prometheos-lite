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

use prometheos_lite::harness::execution_loop::{
    check_resource_limits, estimate_execution_cost, ExecutionMetrics,
    HarnessExecutionRequest, HarnessExecutionResult, HarnessLimits, HarnessMode,
    HarnessProgress, ValidationFailurePolicy,
};

// ============================================================================
// HarnessExecutionRequest Tests
// ============================================================================

#[test]
fn test_harness_execution_request_creation() {
    let request = HarnessExecutionRequest {
        work_context_id: "ctx-123".to_string(),
        repo_root: PathBuf::from("/test/repo"),
        task: "Fix bug in main.rs".to_string(),
        hints: None,
        preferred_mode: Some(HarnessMode::Review),
    };

    assert_eq!(request.work_context_id, "ctx-123");
    assert_eq!(request.repo_root, PathBuf::from("/test/repo"));
    assert_eq!(request.task, "Fix bug in main.rs");
    assert_eq!(request.preferred_mode, Some(HarnessMode::Review));
}

#[test]
fn test_harness_execution_request_with_hints() {
    let request = HarnessExecutionRequest {
        work_context_id: "ctx-456".to_string(),
        repo_root: PathBuf::from("/test/repo"),
        task: "Refactor code".to_string(),
        hints: Some(vec!["src/main.rs".to_string(), "src/lib.rs".to_string()]),
        preferred_mode: Some(HarnessMode::Assisted),
    };

    assert_eq!(request.hints.as_ref().unwrap().len(), 2);
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
        ValidationFailurePolicy::KeepPatchAndRequestApproval,
        ValidationFailurePolicy::KeepPatchAndRequestApproval
    ));
    assert!(matches!(
        ValidationFailurePolicy::RollbackPatch,
        ValidationFailurePolicy::RollbackPatch
    ));
}

#[test]
fn test_validation_failure_policy_default() {
    let policy: ValidationFailurePolicy = Default::default();
    assert!(matches!(
        policy,
        ValidationFailurePolicy::KeepPatchAndRequestApproval
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
        success: true,
        patches_applied: vec![PathBuf::from("src/main.rs")],
        validation_results: vec![],
        execution_time_ms: 5000,
        error_message: None,
    };

    assert!(result.success);
    assert_eq!(result.work_context_id, "ctx-123");
    assert_eq!(result.trace_id, Some("trace-456".to_string()));
    assert_eq!(result.patches_applied.len(), 1);
    assert_eq!(result.execution_time_ms, 5000);
    assert!(result.error_message.is_none());
}

#[test]
fn test_harness_execution_result_failure() {
    let result = HarnessExecutionResult {
        work_context_id: "ctx-789".to_string(),
        trace_id: None,
        success: false,
        patches_applied: vec![],
        validation_results: vec![],
        execution_time_ms: 1000,
        error_message: Some("Validation failed".to_string()),
    };

    assert!(!result.success);
    assert!(result.patches_applied.is_empty());
    assert_eq!(result.error_message, Some("Validation failed".to_string()));
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
        validation_ms: Some(3_000),
    };

    assert_eq!(metrics.total_duration_ms, 10_000);
    assert_eq!(metrics.repo_analysis_ms, 2_000);
    assert_eq!(metrics.patch_generation_ms, 5_000);
    assert_eq!(metrics.validation_ms, Some(3_000));
}

// ============================================================================
// HarnessProgress Tests
// ============================================================================

#[test]
fn test_harness_progress_started() {
    let progress = HarnessProgress::Started {
        work_context_id: "ctx-123".to_string(),
        step: 1,
        message: "Initializing".to_string(),
    };

    assert!(matches!(progress, HarnessProgress::Started { .. }));
}

#[test]
fn test_harness_progress_repo_analyzed() {
    let progress = HarnessProgress::RepoAnalyzed {
        work_context_id: "ctx-123".to_string(),
        files_analyzed: 42,
    };

    assert!(matches!(progress, HarnessProgress::RepoAnalyzed { .. }));
}

#[test]
fn test_harness_progress_patches_generated() {
    let progress = HarnessProgress::PatchesGenerated {
        work_context_id: "ctx-123".to_string(),
        patch_count: 3,
    };

    assert!(matches!(progress, HarnessProgress::PatchesGenerated { .. }));
}

#[test]
fn test_harness_progress_completed() {
    let progress = HarnessProgress::Completed {
        work_context_id: "ctx-123".to_string(),
        success: true,
        patches_applied: 2,
    };

    assert!(matches!(progress, HarnessProgress::Completed { .. }));
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
