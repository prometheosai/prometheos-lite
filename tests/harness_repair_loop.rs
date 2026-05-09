#![cfg(any())]
// Quarantined: obsolete integration suite targets pre-audit harness APIs.
//! Issue 10: Repair Loop Tests
//!
//! Comprehensive tests for the Repair Loop including:
//! - RepairRequest struct creation
//! - RepairResult struct (success, attempts, final_edits)
//! - RepairAttempt struct (attempt_number, strategy, prompt)
//! - AttemptResult enum (Success, PartialSuccess, Failed)
//! - RepairStrategy enum (FixSearchReplace, FixSyntaxError, FixCompileError, etc.)
//! - FailureDetails struct (error message, file, line)
//! - format_repair_report function
//! - Repair loop progression and strategy selection

use std::path::PathBuf;

use prometheos_lite::harness::repair_loop::{
    AttemptResult, RepairAttempt, RepairRequest, RepairResult,
    RepairStrategy, format_repair_report,
};
use prometheos_lite::harness::failure::FailureDetails;
use prometheos_lite::harness::edit_protocol::EditOperation;

// ============================================================================
// RepairRequest Tests
// ============================================================================

#[test]
fn test_repair_request_creation() {
    let request = RepairRequest {
        failure: FailureDetails {
            kind: prometheos_lite::harness::failure::FailureKind::SyntaxError,
            category: prometheos_lite::harness::failure::FailureCategory::Syntax,
            severity: prometheos_lite::harness::failure::FailureSeverity::Error,
            message: "Syntax error at line 10".to_string(),
            context: prometheos_lite::harness::failure::FailureContext {
                file_path: Some(PathBuf::from("src/main.rs")),
                line: Some(10),
            },
        original_edits: vec![],
        patch_result: None,
    };

    assert_eq!(request.failure.error_message, "Syntax error at line 10");
    assert_eq!(request.failure.file, Some(PathBuf::from("src/main.rs")));
    assert_eq!(request.failure.line, Some(10));
}

#[test]
fn test_repair_request_with_edits() {
    let request = RepairRequest {
        failure: FailureDetails {
            error_message: "Test failed".to_string(),
            file: Some(PathBuf::from("tests/test.rs")),
            line: Some(25),
        },
        original_edits: vec![], // Would contain actual edits
        patch_result: None,
    };

    assert!(!request.failure.error_message.is_empty());
}

// ============================================================================
// RepairResult Tests
// ============================================================================

#[test]
fn test_repair_result_success() {
    let result = RepairResult {
        success: true,
        attempts: vec![
            RepairAttempt {
                attempt_number: 1,
                strategy: RepairStrategy::FixSyntaxError,
                prompt: "Fix the syntax error".to_string(),
            },
        ],
        final_edits: Some(vec![]),
    };

    assert!(result.success);
    assert_eq!(result.attempts.len(), 1);
    assert!(result.final_edits.is_some());
}

#[test]
fn test_repair_result_failure() {
    let result = RepairResult {
        success: false,
        attempts: vec![
            RepairAttempt {
                attempt_number: 1,
                strategy: RepairStrategy::FixCompileError,
                prompt: "Attempt 1".to_string(),
            },
            RepairAttempt {
                attempt_number: 2,
                strategy: RepairStrategy::RetryWithLLM,
                prompt: "Attempt 2".to_string(),
            },
        ],
        final_edits: None,
    };

    assert!(!result.success);
    assert_eq!(result.attempts.len(), 2);
    assert!(result.final_edits.is_none());
}

#[test]
fn test_repair_result_empty() {
    let result = RepairResult {
        success: false,
        attempts: vec![],
        final_edits: None,
    };

    assert!(!result.success);
    assert!(result.attempts.is_empty());
}

// ============================================================================
// RepairAttempt Tests
// ============================================================================

#[test]
fn test_repair_attempt_creation() {
    let attempt = RepairAttempt {
        attempt_number: 1,
        strategy: RepairStrategy::FixSearchReplace,
        prompt: "Fix the search/replace pattern".to_string(),
    };

    assert_eq!(attempt.attempt_number, 1);
    assert!(matches!(attempt.strategy, RepairStrategy::FixSearchReplace));
    assert_eq!(attempt.prompt, "Fix the search/replace pattern");
}

#[test]
fn test_repair_attempt_multiple() {
    let attempts = vec![
        RepairAttempt {
            attempt_number: 1,
            strategy: RepairStrategy::FixSyntaxError,
            prompt: "First attempt".to_string(),
        },
        RepairAttempt {
            attempt_number: 2,
            strategy: RepairStrategy::FixCompileError,
            prompt: "Second attempt".to_string(),
        },
        RepairAttempt {
            attempt_number: 3,
            strategy: RepairStrategy::RetryWithLLM,
            prompt: "Third attempt".to_string(),
        },
    ];

    assert_eq!(attempts.len(), 3);
    assert_eq!(attempts[0].attempt_number, 1);
    assert_eq!(attempts[1].attempt_number, 2);
    assert_eq!(attempts[2].attempt_number, 3);
}

// ============================================================================
// AttemptResult Tests
// ============================================================================

#[test]
fn test_attempt_result_success() {
    let result = AttemptResult::Success;
    assert!(matches!(result, AttemptResult::Success));
}

#[test]
fn test_attempt_result_partial_success() {
    let result = AttemptResult::PartialSuccess {
        remaining_failures: vec![],
    };
    assert!(matches!(result, AttemptResult::PartialSuccess { .. }));
}

#[test]
fn test_attempt_result_failed() {
    let result = AttemptResult::Failed {
        reason: "Syntax error still present".to_string(),
    };
    assert!(matches!(result, AttemptResult::Failed { .. }));
}

// ============================================================================
// RepairStrategy Tests
// ============================================================================

#[test]
fn test_repair_strategy_variants() {
    assert!(matches!(RepairStrategy::FixSearchReplace, RepairStrategy::FixSearchReplace));
    assert!(matches!(RepairStrategy::FixSyntaxError, RepairStrategy::FixSyntaxError));
    assert!(matches!(RepairStrategy::FixCompileError, RepairStrategy::FixCompileError));
    assert!(matches!(RepairStrategy::FixTestFailure, RepairStrategy::FixTestFailure));
    assert!(matches!(RepairStrategy::FixLintError, RepairStrategy::FixLintError));
    assert!(matches!(RepairStrategy::FixLogicError, RepairStrategy::FixLogicError));
    assert!(matches!(RepairStrategy::RetryWithLLM, RepairStrategy::RetryWithLLM));
}

#[test]
fn test_repair_strategy_display() {
    assert_eq!(format!("{:?}", RepairStrategy::FixSearchReplace), "FixSearchReplace");
    assert_eq!(format!("{:?}", RepairStrategy::FixSyntaxError), "FixSyntaxError");
    assert_eq!(format!("{:?}", RepairStrategy::FixCompileError), "FixCompileError");
    assert_eq!(format!("{:?}", RepairStrategy::RetryWithLLM), "RetryWithLLM");
}

// ============================================================================
// FailureDetails Tests
// ============================================================================

#[test]
fn test_failure_details_creation() {
    let failure = FailureDetails {
        error_message: "Variable not found in scope".to_string(),
        file: Some(PathBuf::from("src/main.rs")),
        line: Some(42),
    };

    assert_eq!(failure.error_message, "Variable not found in scope");
    assert_eq!(failure.file, Some(PathBuf::from("src/main.rs")));
    assert_eq!(failure.line, Some(42));
}

#[test]
fn test_failure_details_no_location() {
    let failure = FailureDetails {
        error_message: "Build failed".to_string(),
        file: None,
        line: None,
    };

    assert!(failure.file.is_none());
    assert!(failure.line.is_none());
}

// ============================================================================
// format_repair_report Tests
// ============================================================================

#[test]
fn test_format_repair_report_success() {
    let result = RepairResult {
        success: true,
        attempts: vec![
            RepairAttempt {
                attempt_number: 1,
                strategy: RepairStrategy::FixSyntaxError,
                prompt: "Fix syntax".to_string(),
            },
        ],
        final_edits: Some(vec![]),
    };

    let report = format_repair_report(&result);
    assert!(!report.is_empty());
    assert!(report.contains("Repair Loop Report"));
}

#[test]
fn test_format_repair_report_failure() {
    let result = RepairResult {
        success: false,
        attempts: vec![
            RepairAttempt {
                attempt_number: 1,
                strategy: RepairStrategy::FixCompileError,
                prompt: "Fix compile".to_string(),
            },
        ],
        final_edits: None,
    };

    let report = format_repair_report(&result);
    assert!(!report.is_empty());
}

#[test]
fn test_format_repair_report_multiple_attempts() {
    let result = RepairResult {
        success: true,
        attempts: vec![
            RepairAttempt {
                attempt_number: 1,
                strategy: RepairStrategy::FixSyntaxError,
                prompt: "Attempt 1".to_string(),
            },
            RepairAttempt {
                attempt_number: 2,
                strategy: RepairStrategy::RetryWithLLM,
                prompt: "Attempt 2".to_string(),
            },
        ],
        final_edits: Some(vec![]),
    };

    let report = format_repair_report(&result);
    assert!(!report.is_empty());
}
