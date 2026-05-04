//! Issue 9: Reproduction-First Mode Tests
//!
//! Comprehensive tests for the Reproduction-First Mode including:
//! - ReproductionRequest struct creation
//! - ReproductionResult struct (success, failures, reproductions)
//! - ReproductionType enum (CompileError, RuntimeError, TestFailure, LintError)
//! - Reproduction struct (command, output, success)
//! - ReproductionContext struct (environment, dependencies)
//! - reproduce_issue function
//! - validate_reproduction function
//! - gather_reproduction_context function

use std::collections::HashMap;
use std::path::PathBuf;

use prometheos_lite::harness::reproduction::{
    Reproduction, ReproductionContext, ReproductionRequest, ReproductionResult,
    ReproductionType, validate_reproduction,
};

// ============================================================================
// ReproductionRequest Tests
// ============================================================================

#[test]
fn test_reproduction_request_creation() {
    let request = ReproductionRequest {
        issue_description: "Test fails with panic".to_string(),
        reproduction_type: ReproductionType::TestFailure,
        target_file: Some(PathBuf::from("tests/test.rs")),
        command_hint: Some("cargo test".to_string()),
        environment: None,
    };

    assert_eq!(request.issue_description, "Test fails with panic");
    assert!(matches!(request.reproduction_type, ReproductionType::TestFailure));
    assert_eq!(request.target_file, Some(PathBuf::from("tests/test.rs")));
    assert_eq!(request.command_hint, Some("cargo test".to_string()));
}

#[test]
fn test_reproduction_request_compile_error() {
    let request = ReproductionRequest {
        issue_description: "Build fails with type error".to_string(),
        reproduction_type: ReproductionType::CompileError,
        target_file: Some(PathBuf::from("src/main.rs")),
        command_hint: Some("cargo build".to_string()),
        environment: None,
    };

    assert!(matches!(request.reproduction_type, ReproductionType::CompileError));
}

#[test]
fn test_reproduction_request_runtime_error() {
    let request = ReproductionRequest {
        issue_description: "Application crashes on startup".to_string(),
        reproduction_type: ReproductionType::RuntimeError,
        target_file: None,
        command_hint: Some("cargo run".to_string()),
        environment: None,
    };

    assert!(matches!(request.reproduction_type, ReproductionType::RuntimeError));
    assert!(request.target_file.is_none());
}

#[test]
fn test_reproduction_request_lint_error() {
    let request = ReproductionRequest {
        issue_description: "Clippy warnings".to_string(),
        reproduction_type: ReproductionType::LintError,
        target_file: None,
        command_hint: Some("cargo clippy".to_string()),
        environment: None,
    };

    assert!(matches!(request.reproduction_type, ReproductionType::LintError));
}

// ============================================================================
// ReproductionType Tests
// ============================================================================

#[test]
fn test_reproduction_type_variants() {
    assert!(matches!(ReproductionType::CompileError, ReproductionType::CompileError));
    assert!(matches!(ReproductionType::RuntimeError, ReproductionType::RuntimeError));
    assert!(matches!(ReproductionType::TestFailure, ReproductionType::TestFailure));
    assert!(matches!(ReproductionType::LintError, ReproductionType::LintError));
}

#[test]
fn test_reproduction_type_display() {
    assert_eq!(format!("{:?}", ReproductionType::CompileError), "CompileError");
    assert_eq!(format!("{:?}", ReproductionType::RuntimeError), "RuntimeError");
    assert_eq!(format!("{:?}", ReproductionType::TestFailure), "TestFailure");
    assert_eq!(format!("{:?}", ReproductionType::LintError), "LintError");
}

// ============================================================================
// ReproductionResult Tests
// ============================================================================

#[test]
fn test_reproduction_result_success() {
    let result = ReproductionResult {
        success: true,
        reproductions: vec![
            Reproduction {
                command: "cargo test".to_string(),
                output: "test result: FAILED".to_string(),
                success: true,
                exit_code: Some(101),
            },
        ],
        failures: vec![],
        context: None,
    };

    assert!(result.success);
    assert_eq!(result.reproductions.len(), 1);
    assert!(result.failures.is_empty());
}

#[test]
fn test_reproduction_result_failure() {
    let result = ReproductionResult {
        success: false,
        reproductions: vec![],
        failures: vec!["Could not reproduce issue".to_string()],
        context: None,
    };

    assert!(!result.success);
    assert!(!result.failures.is_empty());
}

#[test]
fn test_reproduction_result_multiple_reproductions() {
    let result = ReproductionResult {
        success: true,
        reproductions: vec![
            Reproduction {
                command: "cargo build".to_string(),
                output: "error: ...".to_string(),
                success: true,
                exit_code: Some(101),
            },
            Reproduction {
                command: "cargo test".to_string(),
                output: "test result: ok".to_string(),
                success: true,
                exit_code: Some(0),
            },
        ],
        failures: vec![],
        context: None,
    };

    assert_eq!(result.reproductions.len(), 2);
}

// ============================================================================
// Reproduction Tests
// ============================================================================

#[test]
fn test_reproduction_creation() {
    let repro = Reproduction {
        command: "cargo test test_name".to_string(),
        output: "running 1 test\ntest test_name ... FAILED".to_string(),
        success: true,
        exit_code: Some(101),
    };

    assert_eq!(repro.command, "cargo test test_name");
    assert!(repro.output.contains("FAILED"));
    assert!(repro.success);
    assert_eq!(repro.exit_code, Some(101));
}

#[test]
fn test_reproduction_no_exit_code() {
    let repro = Reproduction {
        command: "timeout 5s cargo test".to_string(),
        output: "timeout".to_string(),
        success: true,
        exit_code: None,
    };

    assert_eq!(repro.exit_code, None);
}

#[test]
fn test_reproduction_failed() {
    let repro = Reproduction {
        command: "cargo test".to_string(),
        output: String::new(),
        success: false,
        exit_code: None,
    };

    assert!(!repro.success);
}

// ============================================================================
// ReproductionContext Tests
// ============================================================================

#[test]
fn test_reproduction_context_creation() {
    let mut env = HashMap::new();
    env.insert("RUST_BACKTRACE".to_string(), "1".to_string());

    let ctx = ReproductionContext {
        environment: env,
        dependencies: vec!["rustc".to_string(), "cargo".to_string()],
        working_directory: PathBuf::from("/test/repo"),
    };

    assert_eq!(ctx.environment.get("RUST_BACKTRACE"), Some(&"1".to_string()));
    assert_eq!(ctx.dependencies.len(), 2);
    assert_eq!(ctx.working_directory, PathBuf::from("/test/repo"));
}

#[test]
fn test_reproduction_context_empty() {
    let ctx = ReproductionContext {
        environment: HashMap::new(),
        dependencies: vec![],
        working_directory: PathBuf::from("."),
    };

    assert!(ctx.environment.is_empty());
    assert!(ctx.dependencies.is_empty());
}

// ============================================================================
// validate_reproduction Tests
// ============================================================================

#[test]
fn test_validate_reproduction_valid() {
    let repro = Reproduction {
        command: "cargo test".to_string(),
        output: "test result: FAILED".to_string(),
        success: true,
        exit_code: Some(101),
    };

    let result = validate_reproduction(&repro, ReproductionType::TestFailure);
    assert!(result.is_ok());
}

#[test]
fn test_validate_reproduction_invalid() {
    let repro = Reproduction {
        command: "cargo test".to_string(),
        output: "test result: ok".to_string(),
        success: true,
        exit_code: Some(0),
    };

    let result = validate_reproduction(&repro, ReproductionType::TestFailure);
    // Should fail because we expect a test failure but tests passed
    assert!(result.is_err());
}
