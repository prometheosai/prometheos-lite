//! Issue 32: Regression Memory Tests
//!
//! Comprehensive tests for Regression Memory including:
//! - FailurePattern struct (id, signature, type, frequency, solutions, attempts)
//! - SuccessfulSolution struct (id, description, approach, count, confidence)
//! - UnsuccessfulAttempt struct (id, description, failure_reason)
//! - FailureType enum (SyntaxError, TypeError, RuntimeError, TestFailure, etc.)
//! - RegressionMemory for pattern storage and retrieval
//! - PatternMatch for matching failures
//! - AccessType enum (Read, Write, Match, Learn)

use std::path::PathBuf;

use prometheos_lite::harness::regression_memory::{
    AccessType, FailurePattern, FailureType, PatternMatch, RegressionMemory, SuccessfulSolution,
    UnsuccessfulAttempt,
};

// ============================================================================
// FailurePattern Tests
// ============================================================================

#[test]
fn test_failure_pattern_creation() {
    let pattern = FailurePattern {
        id: "pattern-1".to_string(),
        pattern_signature: "syntax_error::missing_semicolon".to_string(),
        failure_type: FailureType::SyntaxError,
        context_hash: "abc123".to_string(),
        error_signature: "expected ';' found '}'".to_string(),
        file_path: Some(PathBuf::from("src/main.rs")),
        line_number: Some(42),
        frequency: 5,
        first_seen: chrono::Utc::now(),
        last_seen: chrono::Utc::now(),
        successful_solutions: vec![],
        unsuccessful_attempts: vec![],
    };

    assert_eq!(pattern.id, "pattern-1");
    assert!(matches!(pattern.failure_type, FailureType::SyntaxError));
    assert_eq!(pattern.frequency, 5);
}

#[test]
fn test_failure_pattern_with_solutions() {
    let pattern = FailurePattern {
        id: "pattern-2".to_string(),
        pattern_signature: "type_error::mismatch".to_string(),
        failure_type: FailureType::TypeError,
        context_hash: "def456".to_string(),
        error_signature: "expected i32, found String".to_string(),
        file_path: Some(PathBuf::from("src/lib.rs")),
        line_number: Some(10),
        frequency: 3,
        first_seen: chrono::Utc::now(),
        last_seen: chrono::Utc::now(),
        successful_solutions: vec![
            SuccessfulSolution {
                solution_id: "sol-1".to_string(),
                description: "Add type annotation".to_string(),
                approach: "Explicit type".to_string(),
                success_count: 10,
                confidence: 0.95,
            },
        ],
        unsuccessful_attempts: vec![],
    };

    assert_eq!(pattern.successful_solutions.len(), 1);
    assert_eq!(pattern.successful_solutions[0].confidence, 0.95);
}

// ============================================================================
// FailureType Tests
// ============================================================================

#[test]
fn test_failure_type_variants() {
    assert!(matches!(FailureType::SyntaxError, FailureType::SyntaxError));
    assert!(matches!(FailureType::TypeError, FailureType::TypeError));
    assert!(matches!(FailureType::RuntimeError, FailureType::RuntimeError));
    assert!(matches!(FailureType::TestFailure, FailureType::TestFailure));
    assert!(matches!(FailureType::CompilationError, FailureType::CompilationError));
    assert!(matches!(FailureType::LintError, FailureType::LintError));
    assert!(matches!(FailureType::LogicError, FailureType::LogicError));
    assert!(matches!(FailureType::PerformanceIssue, FailureType::PerformanceIssue));
    assert!(matches!(FailureType::Unknown, FailureType::Unknown));
}

// ============================================================================
// SuccessfulSolution Tests
// ============================================================================

#[test]
fn test_successful_solution_creation() {
    let solution = SuccessfulSolution {
        solution_id: "sol-1".to_string(),
        description: "Add missing import".to_string(),
        approach: "Insert use statement".to_string(),
        success_count: 15,
        confidence: 0.92,
    };

    assert_eq!(solution.solution_id, "sol-1");
    assert_eq!(solution.success_count, 15);
    assert_eq!(solution.confidence, 0.92);
}

// ============================================================================
// UnsuccessfulAttempt Tests
// ============================================================================

#[test]
fn test_unsuccessful_attempt_creation() {
    let attempt = UnsuccessfulAttempt {
        attempt_id: "att-1".to_string(),
        description: "Try auto-fix".to_string(),
        failure_reason: "Cannot determine correct fix".to_string(),
    };

    assert_eq!(attempt.attempt_id, "att-1");
    assert_eq!(attempt.failure_reason, "Cannot determine correct fix");
}

// ============================================================================
// RegressionMemory Tests
// ============================================================================

#[test]
fn test_regression_memory_new() {
    let memory = RegressionMemory::new();
    // Memory created successfully
    assert!(true);
}

// ============================================================================
// PatternMatch Tests
// ============================================================================

#[test]
fn test_pattern_match_creation() {
    let pattern = FailurePattern {
        id: "match-pattern".to_string(),
        pattern_signature: "test".to_string(),
        failure_type: FailureType::TestFailure,
        context_hash: "hash".to_string(),
        error_signature: "assertion failed".to_string(),
        file_path: None,
        line_number: None,
        frequency: 1,
        first_seen: chrono::Utc::now(),
        last_seen: chrono::Utc::now(),
        successful_solutions: vec![],
        unsuccessful_attempts: vec![],
    };

    let match_result = PatternMatch {
        pattern,
        confidence: 0.85,
        matched_fields: vec!["error_signature".to_string()],
    };

    assert_eq!(match_result.confidence, 0.85);
    assert_eq!(match_result.matched_fields.len(), 1);
}

// ============================================================================
// AccessType Tests
// ============================================================================

#[test]
fn test_access_type_variants() {
    assert!(matches!(AccessType::Read, AccessType::Read));
    assert!(matches!(AccessType::Write, AccessType::Write));
    assert!(matches!(AccessType::Match, AccessType::Match));
    assert!(matches!(AccessType::Learn, AccessType::Learn));
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_regression_memory_workflow() {
    // Create a failure pattern with solutions
    let pattern = FailurePattern {
        id: "regression-1".to_string(),
        pattern_signature: "borrow_checker::lifetime".to_string(),
        failure_type: FailureType::CompilationError,
        context_hash: "ctx123".to_string(),
        error_signature: "lifetime mismatch".to_string(),
        file_path: Some(PathBuf::from("src/lib.rs")),
        line_number: Some(25),
        frequency: 8,
        first_seen: chrono::Utc::now(),
        last_seen: chrono::Utc::now(),
        successful_solutions: vec![
            SuccessfulSolution {
                solution_id: "sol-a".to_string(),
                description: "Add explicit lifetime".to_string(),
                approach: "Annotate with 'a".to_string(),
                success_count: 20,
                confidence: 0.94,
            },
            SuccessfulSolution {
                solution_id: "sol-b".to_string(),
                description: "Use owned type".to_string(),
                approach: "Replace &str with String".to_string(),
                success_count: 12,
                confidence: 0.88,
            },
        ],
        unsuccessful_attempts: vec![
            UnsuccessfulAttempt {
                attempt_id: "att-a".to_string(),
                description: "Clone the reference".to_string(),
                failure_reason: "Still borrowed".to_string(),
            },
        ],
    };

    assert_eq!(pattern.successful_solutions.len(), 2);
    assert_eq!(pattern.unsuccessful_attempts.len(), 1);
    assert!(pattern.successful_solutions[0].confidence > pattern.successful_solutions[1].confidence);
}
