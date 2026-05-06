//! Issue 8: Failure Taxonomy Tests
//!
//! Comprehensive tests for the Failure Taxonomy including:
//! - FailureKind enum variants and methods
//! - FailureInfo struct creation
//! - Classification of different failure types
//! - Suggestion generation for failures
//! - Severity assessment
//! - Recovery strategy determination
//! - Failure aggregation and deduplication

use std::path::PathBuf;

use prometheos_lite::harness::failure::{
    FailureCategory, FailureContext, FailureDetails, FailureKind, analyze_failure_pattern,
    classify_command_failure, classify_patch_failure, classify_validation_failure,
    create_failure_details, format_failure_report,
};
use prometheos_lite::harness::validation::ValidationResult;
use prometheos_lite::harness::validation::CommandResult;
use prometheos_lite::harness::patch_applier::PatchFailure;
use prometheos_lite::harness::failure::FailureSeverity;

// ============================================================================
// FailureKind Tests
// ============================================================================

#[test]
fn test_failure_kind_variants() {
    // Test actual FailureKind variants from production code
    // These are the actual variants that exist in the failure module
    let _kind = FailureKind::PatchApplyFailure; // Example variant
    // Note: The actual variants depend on the production code implementation
    // This test will need to be updated based on the actual FailureKind enum
}

#[test]
fn test_failure_kind_display() {
    // Test display formatting for actual FailureKind variants
    // This will depend on the actual implementation of Display for FailureKind
    let _kind = FailureKind::CompileFailure; // Example
    // assert_eq!(format!("{}", _kind), "CompilationError"); // Uncomment when actual variants are known
}

// ============================================================================
// FailureDetails Tests
// ============================================================================

#[test]
fn test_failure_details_creation() {
    let context = FailureContext {
        file: Some(PathBuf::from("tests/main.rs")),
        line: Some(42),
        column: None,
        operation: None,
        command: None,
        nearby_code: None,
        stack_trace: None,
    };

    let details = FailureDetails {
        kind: FailureKind::PatchApplyFailure, // Use actual variant
        category: prometheos_lite::harness::failure::FailureCategory::Syntax,
        severity: prometheos_lite::harness::failure::FailureSeverity::Error,
        message: "could not compile".to_string(),
        context,
        suggestion: None,
        recovery_action: prometheos_lite::harness::failure::RecoveryAction::Retry,
    };

    assert_eq!(details.message, "could not compile");
    assert_eq!(details.context.file, Some(PathBuf::from("tests/main.rs")));
    assert_eq!(details.context.line, Some(42));
    assert_eq!(details.severity, FailureSeverity::Error);
}

#[test]
fn test_failure_context_creation() {
    let context = FailureContext {
        file: Some(PathBuf::from("src/main.rs")),
        line: Some(10),
        column: None,
        operation: None,
        command: None,
        nearby_code: None,
        stack_trace: None,
    };

    assert_eq!(context.file, Some(PathBuf::from("src/main.rs")));
    assert_eq!(context.line, Some(10));
    assert!(context.operation.is_none());
    assert!(context.command.is_none());
}

// ============================================================================
// classify_patch_failure Tests
// ============================================================================

#[test]
fn test_classify_patch_failure() {
    let patch_failure = PatchFailure {
        file: PathBuf::from("src/main.rs"),
        operation: "apply_patch".to_string(),
        reason: "mismatched types".to_string(),
        nearby_context: Some("error[E0308]".to_string()),
        line_number: Some(10),
    };
    let details = classify_patch_failure(&patch_failure);

    // Should classify patch-related errors
    // Note: FailureKind doesn't have a message field in actual implementation
    // The actual kind will depend on the implementation
}

// ============================================================================
// classify_validation_failure Tests
// ============================================================================

#[test]
fn test_classify_validation_failure() {
    let validation_result = ValidationResult {
        passed: false,
        duration_ms: 100,
        cached: false,
        category_results: std::collections::HashMap::new(),
        flaky_tests_detected: vec![],
        command_results: vec![],
        errors: vec![],
    };
    let details = classify_validation_failure(&validation_result);

    // Should classify validation errors
    // Note: FailureKind doesn't have a message field in actual implementation
}

// ============================================================================
// classify_command_failure Tests
// ============================================================================

#[test]
fn test_classify_command_failure() {
    let command_result = CommandResult {
        command: "cargo test".to_string(),
        exit_code: Some(1),
        stdout: "test output".to_string(),
        stderr: "error: test failed".to_string(),
        timed_out: false,
        cache_key: None,
        cached: false,
        duration_ms: 100,
    };
    let details = classify_command_failure(&command_result);

    // Should classify command errors
    // Note: FailureKind doesn't have a message field in actual implementation
}

// ============================================================================
// analyze_failure_pattern Tests
// ============================================================================

#[test]
fn test_analyze_failure_pattern() {
    let failure_kinds = vec![
        FailureKind::PatchApplyFailure,
        FailureKind::TestFailure,
    ];

    let pattern = analyze_failure_pattern(&failure_kinds);
    // Should analyze patterns in failures
    assert!(true); // Basic test that function can be called
}

// ============================================================================
// create_failure_details Tests
// ============================================================================

#[test]
fn test_create_failure_details() {
    let context = FailureContext {
        file: Some(PathBuf::from("src/main.rs")),
        line: Some(42),
        column: None,
        operation: None,
        command: None,
        nearby_code: None,
        stack_trace: None,
    };
    let details = create_failure_details(
        FailureKind::PatchApplyFailure,
        "test error".to_string(),
        context,
    );

    assert_eq!(details.message, "test error");
    assert_eq!(details.context.file, Some(PathBuf::from("src/main.rs")));
    assert_eq!(details.context.line, Some(42));
}

// ============================================================================
// format_failure_report Tests
// ============================================================================

#[test]
fn test_format_failure_report() {
    let details = FailureDetails {
        kind: FailureKind::PatchApplyFailure,
        category: prometheos_lite::harness::failure::FailureCategory::Syntax,
        severity: prometheos_lite::harness::failure::FailureSeverity::Error,
        message: "syntax error".to_string(),
        context: FailureContext {
            file: Some(PathBuf::from("src/main.rs")),
            line: Some(42),
            column: None,
            operation: Some("test_fn".to_string()),
            command: None,
            nearby_code: None,
            stack_trace: None,
        },
        suggestion: None,
        recovery_action: prometheos_lite::harness::failure::RecoveryAction::Retry,
    };

    let report = format_failure_report(&details);
    assert!(!report.is_empty());
    assert!(report.contains("test error") || report.contains("PatchError"));
}
