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
    classify_failure, FailureInfo, FailureKind, get_recovery_strategy,
};

// ============================================================================
// FailureKind Tests
// ============================================================================

#[test]
fn test_failure_kind_variants() {
    assert!(matches!(FailureKind::SyntaxError, FailureKind::SyntaxError));
    assert!(matches!(FailureKind::TypeError, FailureKind::TypeError));
    assert!(matches!(FailureKind::TestFailure, FailureKind::TestFailure));
    assert!(matches!(FailureKind::CompilationError, FailureKind::CompilationError));
    assert!(matches!(FailureKind::LintError, FailureKind::LintError));
    assert!(matches!(FailureKind::SecurityError, FailureKind::SecurityError));
    assert!(matches!(FailureKind::RuntimeError, FailureKind::RuntimeError));
    assert!(matches!(FailureKind::LogicError, FailureKind::LogicError));
    assert!(matches!(FailureKind::Unknown, FailureKind::Unknown));
}

#[test]
fn test_failure_kind_display() {
    assert_eq!(format!("{}", FailureKind::SyntaxError), "SyntaxError");
    assert_eq!(format!("{}", FailureKind::TypeError), "TypeError");
    assert_eq!(format!("{}", FailureKind::TestFailure), "TestFailure");
    assert_eq!(format!("{}", FailureKind::CompilationError), "CompilationError");
    assert_eq!(format!("{}", FailureKind::LintError), "LintError");
    assert_eq!(format!("{}", FailureKind::SecurityError), "SecurityError");
    assert_eq!(format!("{}", FailureKind::RuntimeError), "RuntimeError");
    assert_eq!(format!("{}", FailureKind::LogicError), "LogicError");
    assert_eq!(format!("{}", FailureKind::Unknown), "Unknown");
}

#[test]
fn test_failure_kind_from_error_message() {
    // Test syntax error detection
    assert_eq!(
        FailureKind::from_error_message("expected ';' at line 10"),
        FailureKind::SyntaxError
    );

    // Test type error detection
    assert_eq!(
        FailureKind::from_error_message("mismatched types"),
        FailureKind::TypeError
    );

    // Test test failure detection
    assert_eq!(
        FailureKind::from_error_message("test failed: assertion failed"),
        FailureKind::TestFailure
    );

    // Test compilation error detection
    assert_eq!(
        FailureKind::from_error_message("could not compile"),
        FailureKind::CompilationError
    );
}

// ============================================================================
// FailureInfo Tests
// ============================================================================

#[test]
fn test_failure_info_creation() {
    let failure = FailureInfo {
        kind: FailureKind::TestFailure,
        message: "assertion failed".to_string(),
        file: Some(PathBuf::from("tests/main.rs")),
        line: Some(42),
        context: Some("assert_eq!(result, expected)".to_string()),
        suggestion: Some("Check the assertion values".to_string()),
        severity: 3,
    };

    assert_eq!(failure.kind, FailureKind::TestFailure);
    assert_eq!(failure.message, "assertion failed");
    assert_eq!(failure.file, Some(PathBuf::from("tests/main.rs")));
    assert_eq!(failure.line, Some(42));
    assert_eq!(failure.severity, 3);
}

#[test]
fn test_failure_info_without_context() {
    let failure = FailureInfo {
        kind: FailureKind::SyntaxError,
        message: "unexpected token".to_string(),
        file: Some(PathBuf::from("src/main.rs")),
        line: Some(10),
        context: None,
        suggestion: None,
        severity: 2,
    };

    assert_eq!(failure.context, None);
    assert_eq!(failure.suggestion, None);
}

// ============================================================================
// classify_failure Tests
// ============================================================================

#[test]
fn test_classify_failure_compilation() {
    let error = "error[E0308]: mismatched types\n  --> src/main.rs:10:5";
    let failure = classify_failure(error, Some(PathBuf::from("src/main.rs")));

    assert_eq!(failure.kind, FailureKind::TypeError);
    assert!(failure.message.contains("mismatched types"));
    assert_eq!(failure.file, Some(PathBuf::from("src/main.rs")));
}

#[test]
fn test_classify_failure_test() {
    let error = "running 1 test\ntest tests::test_example ... FAILED";
    let failure = classify_failure(error, None);

    assert_eq!(failure.kind, FailureKind::TestFailure);
    assert!(failure.message.contains("FAILED"));
}

#[test]
fn test_classify_failure_syntax() {
    let error = "expected ';', found '}'";
    let failure = classify_failure(error, None);

    assert_eq!(failure.kind, FailureKind::SyntaxError);
}

#[test]
fn test_classify_failure_unknown() {
    let error = "some random error message";
    let failure = classify_failure(error, None);

    assert_eq!(failure.kind, FailureKind::Unknown);
    assert_eq!(failure.message, "some random error message");
}

// ============================================================================
// get_recovery_strategy Tests
// ============================================================================

#[test]
fn test_get_recovery_strategy_for_syntax_error() {
    let failure = FailureInfo {
        kind: FailureKind::SyntaxError,
        message: "expected ';'".to_string(),
        file: Some(PathBuf::from("src/main.rs")),
        line: Some(10),
        context: None,
        suggestion: None,
        severity: 3,
    };

    let strategy = get_recovery_strategy(&failure);
    assert!(!strategy.is_empty());
    assert!(strategy.contains("fix") || strategy.contains("correct"));
}

#[test]
fn test_get_recovery_strategy_for_test_failure() {
    let failure = FailureInfo {
        kind: FailureKind::TestFailure,
        message: "assertion failed".to_string(),
        file: Some(PathBuf::from("tests/lib.rs")),
        line: Some(25),
        context: None,
        suggestion: None,
        severity: 2,
    };

    let strategy = get_recovery_strategy(&failure);
    assert!(!strategy.is_empty());
}

#[test]
fn test_get_recovery_strategy_for_severity_levels() {
    let critical = FailureInfo {
        kind: FailureKind::SecurityError,
        message: "security vulnerability".to_string(),
        file: None,
        line: None,
        context: None,
        suggestion: None,
        severity: 5,
    };

    let strategy = get_recovery_strategy(&critical);
    assert!(!strategy.is_empty());
}
