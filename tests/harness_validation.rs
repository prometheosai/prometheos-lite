//! Issue 7: Validation Layer Tests
//!
//! Comprehensive tests for the Validation Layer including:
//! - ValidationPlan struct creation (format, lint, test commands)
//! - ValidationResult struct (passed/failed, command results, errors)
//! - ValidationCategory enum (Format, Lint, Test)
//! - CategoryResult struct
//! - CommandResult struct (command, exit_code, stdout, stderr)
//! - FlakyTestInfo struct (test_name, command, attempt_results)
//! - TestAttempt struct (attempt, passed, duration_ms)
//! - get_validation_summary function
//! - Command execution and result collection

use std::path::PathBuf;

use prometheos_lite::harness::validation::{
    CategoryResult, CommandResult, FlakyTestInfo, TestAttempt, ValidationCategory,
    ValidationPlan, ValidationResult, get_validation_summary,
};

// ============================================================================
// ValidationPlan Tests
// ============================================================================

#[test]
fn test_validation_plan_default() {
    let plan = ValidationPlan::default();

    assert!(plan.format_commands.is_empty());
    assert!(plan.lint_commands.is_empty());
    assert!(plan.test_commands.is_empty());
}

#[test]
fn test_validation_plan_creation() {
    let plan = ValidationPlan {
        format_commands: vec!["cargo fmt".to_string()],
        lint_commands: vec!["cargo clippy".to_string()],
        test_commands: vec!["cargo test".to_string()],
    };

    assert_eq!(plan.format_commands, vec!["cargo fmt"]);
    assert_eq!(plan.lint_commands, vec!["cargo clippy"]);
    assert_eq!(plan.test_commands, vec!["cargo test"]);
}

#[test]
fn test_validation_plan_with_multiple_commands() {
    let plan = ValidationPlan {
        format_commands: vec!["cargo fmt".to_string(), "prettier --write .".to_string()],
        lint_commands: vec!["cargo clippy".to_string(), "eslint .".to_string()],
        test_commands: vec!["cargo test".to_string(), "jest".to_string()],
    };

    assert_eq!(plan.format_commands.len(), 2);
    assert_eq!(plan.lint_commands.len(), 2);
    assert_eq!(plan.test_commands.len(), 2);
}

// ============================================================================
// ValidationResult Tests
// ============================================================================

#[test]
fn test_validation_result_passed() {
    let result = ValidationResult {
        passed: true,
        command_results: vec![
            CommandResult {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "test result: ok".to_string(),
                stderr: String::new(),
                duration_ms: 5000,
            },
        ],
        errors: vec![],
    };

    assert!(result.passed);
    assert!(result.errors.is_empty());
    assert_eq!(result.command_results.len(), 1);
}

#[test]
fn test_validation_result_failed() {
    let result = ValidationResult {
        passed: false,
        command_results: vec![
            CommandResult {
                command: "cargo test".to_string(),
                exit_code: Some(1),
                stdout: String::new(),
                stderr: "test failed".to_string(),
                duration_ms: 3000,
            },
        ],
        errors: vec!["Tests failed".to_string()],
    };

    assert!(!result.passed);
    assert!(!result.errors.is_empty());
    assert_eq!(result.errors[0], "Tests failed");
}

#[test]
fn test_validation_result_empty() {
    let result = ValidationResult {
        passed: true,
        command_results: vec![],
        errors: vec![],
    };

    assert!(result.passed);
    assert!(result.command_results.is_empty());
}

// ============================================================================
// ValidationCategory Tests
// ============================================================================

#[test]
fn test_validation_category_variants() {
    assert!(matches!(ValidationCategory::Format, ValidationCategory::Format));
    assert!(matches!(ValidationCategory::Lint, ValidationCategory::Lint));
    assert!(matches!(ValidationCategory::Test, ValidationCategory::Test));
}

#[test]
fn test_validation_category_display() {
    assert_eq!(format!("{:?}", ValidationCategory::Format), "Format");
    assert_eq!(format!("{:?}", ValidationCategory::Lint), "Lint");
    assert_eq!(format!("{:?}", ValidationCategory::Test), "Test");
}

// ============================================================================
// CategoryResult Tests
// ============================================================================

#[test]
fn test_category_result_passed() {
    let result = CategoryResult {
        category: ValidationCategory::Test,
        passed: true,
        commands: vec![
            CommandResult {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                duration_ms: 1000,
            },
        ],
    };

    assert!(matches!(result.category, ValidationCategory::Test));
    assert!(result.passed);
    assert_eq!(result.commands.len(), 1);
}

#[test]
fn test_category_result_failed() {
    let result = CategoryResult {
        category: ValidationCategory::Lint,
        passed: false,
        commands: vec![
            CommandResult {
                command: "cargo clippy".to_string(),
                exit_code: Some(1),
                stdout: String::new(),
                stderr: "error".to_string(),
                duration_ms: 2000,
            },
        ],
    };

    assert!(matches!(result.category, ValidationCategory::Lint));
    assert!(!result.passed);
}

// ============================================================================
// CommandResult Tests
// ============================================================================

#[test]
fn test_command_result_success() {
    let result = CommandResult {
        command: "cargo build".to_string(),
        exit_code: Some(0),
        stdout: "Compiling...".to_string(),
        stderr: String::new(),
        duration_ms: 5000,
    };

    assert_eq!(result.command, "cargo build");
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.stdout, "Compiling...");
    assert_eq!(result.duration_ms, 5000);
}

#[test]
fn test_command_result_failure() {
    let result = CommandResult {
        command: "cargo test".to_string(),
        exit_code: Some(101),
        stdout: String::new(),
        stderr: "test failed".to_string(),
        duration_ms: 3000,
    };

    assert_eq!(result.exit_code, Some(101));
    assert_eq!(result.stderr, "test failed");
}

#[test]
fn test_command_result_no_exit_code() {
    let result = CommandResult {
        command: "timeout 5s cargo test".to_string(),
        exit_code: None,
        stdout: String::new(),
        stderr: "timeout".to_string(),
        duration_ms: 5000,
    };

    assert_eq!(result.exit_code, None);
}

// ============================================================================
// FlakyTestInfo Tests
// ============================================================================

#[test]
fn test_flaky_test_info_creation() {
    let info = FlakyTestInfo {
        test_name: "test_flaky".to_string(),
        command: "cargo test test_flaky".to_string(),
        attempt_results: vec![
            TestAttempt {
                attempt: 1,
                passed: false,
                duration_ms: 1000,
            },
            TestAttempt {
                attempt: 2,
                passed: true,
                duration_ms: 1200,
            },
        ],
    };

    assert_eq!(info.test_name, "test_flaky");
    assert_eq!(info.command, "cargo test test_flaky");
    assert_eq!(info.attempt_results.len(), 2);
    assert!(!info.attempt_results[0].passed);
    assert!(info.attempt_results[1].passed);
}

#[test]
fn test_flaky_test_info_all_passed() {
    let info = FlakyTestInfo {
        test_name: "test_stable".to_string(),
        command: "cargo test test_stable".to_string(),
        attempt_results: vec![
            TestAttempt {
                attempt: 1,
                passed: true,
                duration_ms: 800,
            },
            TestAttempt {
                attempt: 2,
                passed: true,
                duration_ms: 750,
            },
            TestAttempt {
                attempt: 3,
                passed: true,
                duration_ms: 780,
            },
        ],
    };

    assert!(info.attempt_results.iter().all(|a| a.passed));
}

// ============================================================================
// TestAttempt Tests
// ============================================================================

#[test]
fn test_test_attempt_passed() {
    let attempt = TestAttempt {
        attempt: 1,
        passed: true,
        duration_ms: 1000,
    };

    assert_eq!(attempt.attempt, 1);
    assert!(attempt.passed);
    assert_eq!(attempt.duration_ms, 1000);
}

#[test]
fn test_test_attempt_failed() {
    let attempt = TestAttempt {
        attempt: 2,
        passed: false,
        duration_ms: 500,
    };

    assert_eq!(attempt.attempt, 2);
    assert!(!attempt.passed);
}

// ============================================================================
// get_validation_summary Tests
// ============================================================================

#[test]
fn test_get_validation_summary_all_passed() {
    let result = ValidationResult {
        passed: true,
        command_results: vec![
            CommandResult {
                command: "cargo fmt".to_string(),
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: 1000,
            },
            CommandResult {
                command: "cargo test".to_string(),
                exit_code: Some(0),
                stdout: "test result: ok".to_string(),
                stderr: String::new(),
                duration_ms: 5000,
            },
        ],
        errors: vec![],
    };

    let summary = get_validation_summary(&result);
    assert!(summary.contains("passed"));
}

#[test]
fn test_get_validation_summary_with_failures() {
    let result = ValidationResult {
        passed: false,
        command_results: vec![
            CommandResult {
                command: "cargo test".to_string(),
                exit_code: Some(1),
                stdout: String::new(),
                stderr: "test failed".to_string(),
                duration_ms: 3000,
            },
        ],
        errors: vec!["Test failure".to_string()],
    };

    let summary = get_validation_summary(&result);
    assert!(!summary.is_empty());
}

#[test]
fn test_get_validation_summary_empty() {
    let result = ValidationResult {
        passed: true,
        command_results: vec![],
        errors: vec![],
    };

    let summary = get_validation_summary(&result);
    assert!(!summary.is_empty());
}
