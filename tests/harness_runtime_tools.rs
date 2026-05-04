//! Issue 27: Runtime Tools Tests
//!
//! Comprehensive tests for Runtime Tools including:
//! - RuntimeTool struct (id, name, version, type, path, args, env, timeout, memory)
//! - ToolType enum (Linter, Formatter, Compiler, TestRunner, etc.)
//! - RuntimeToolRegistry for tool management
//! - ToolExecution struct (tool_id, input, args, timing, output, success)
//! - ToolResult struct (success, exit_code, stdout, stderr, duration, issues)
//! - ToolIssue struct (severity, file, line, message, code, fix)
//! - IssueSeverity enum for issue classification

use std::collections::HashMap;
use std::path::PathBuf;

use prometheos_lite::harness::runtime_tools::{
    IssueSeverity, RuntimeTool, ToolExecution, ToolIssue, ToolResult, ToolType,
};

// ============================================================================
// RuntimeTool Tests
// ============================================================================

#[test]
fn test_runtime_tool_creation() {
    let tool = RuntimeTool {
        id: "clippy-1".to_string(),
        name: "Clippy".to_string(),
        version: "0.1.75".to_string(),
        tool_type: ToolType::Linter,
        executable_path: PathBuf::from("/usr/bin/cargo"),
        args_template: vec!["clippy".to_string(), "--".to_string()],
        env_vars: HashMap::new(),
        timeout_ms: 60000,
        max_memory_mb: 512,
        description: "Rust linter".to_string(),
        supported_extensions: vec!["rs".to_string()],
        health_check_cmd: Some("cargo clippy --version".to_string()),
    };

    assert_eq!(tool.id, "clippy-1");
    assert_eq!(tool.name, "Clippy");
    assert!(matches!(tool.tool_type, ToolType::Linter));
    assert_eq!(tool.timeout_ms, 60000);
}

#[test]
fn test_runtime_tool_compiler() {
    let tool = RuntimeTool {
        id: "rustc-1".to_string(),
        name: "Rustc".to_string(),
        version: "1.75.0".to_string(),
        tool_type: ToolType::Compiler,
        executable_path: PathBuf::from("/usr/bin/rustc"),
        args_template: vec!["--crate-type".to_string(), "lib".to_string()],
        env_vars: {
            let mut env = HashMap::new();
            env.insert("RUST_BACKTRACE".to_string(), "1".to_string());
            env
        },
        timeout_ms: 120000,
        max_memory_mb: 1024,
        description: "Rust compiler".to_string(),
        supported_extensions: vec!["rs".to_string()],
        health_check_cmd: Some("rustc --version".to_string()),
    };

    assert!(matches!(tool.tool_type, ToolType::Compiler));
    assert_eq!(tool.env_vars.get("RUST_BACKTRACE"), Some(&"1".to_string()));
}

// ============================================================================
// ToolType Tests
// ============================================================================

#[test]
fn test_tool_type_variants() {
    assert!(matches!(ToolType::Linter, ToolType::Linter));
    assert!(matches!(ToolType::Formatter, ToolType::Formatter));
    assert!(matches!(ToolType::Compiler, ToolType::Compiler));
    assert!(matches!(ToolType::TestRunner, ToolType::TestRunner));
    assert!(matches!(ToolType::StaticAnalyzer, ToolType::StaticAnalyzer));
    assert!(matches!(ToolType::SecurityScanner, ToolType::SecurityScanner));
    assert!(matches!(ToolType::DocumentationGenerator, ToolType::DocumentationGenerator));
    assert!(matches!(ToolType::Custom, ToolType::Custom));
}

// ============================================================================
// ToolExecution Tests
// ============================================================================

#[test]
fn test_tool_execution_success() {
    let exec = ToolExecution {
        tool_id: "cargo-test".to_string(),
        input_file: Some(PathBuf::from("src/lib.rs")),
        args: vec!["test".to_string()],
        start_time: chrono::Utc::now(),
        end_time: Some(chrono::Utc::now()),
        exit_code: Some(0),
        stdout: "test result: ok".to_string(),
        stderr: String::new(),
        success: true,
    };

    assert!(exec.success);
    assert_eq!(exec.exit_code, Some(0));
    assert_eq!(exec.stdout, "test result: ok");
}

#[test]
fn test_tool_execution_failure() {
    let exec = ToolExecution {
        tool_id: "cargo-test".to_string(),
        input_file: Some(PathBuf::from("src/lib.rs")),
        args: vec!["test".to_string()],
        start_time: chrono::Utc::now(),
        end_time: Some(chrono::Utc::now()),
        exit_code: Some(101),
        stdout: String::new(),
        stderr: "test failed".to_string(),
        success: false,
    };

    assert!(!exec.success);
    assert_eq!(exec.exit_code, Some(101));
}

// ============================================================================
// ToolResult Tests
// ============================================================================

#[test]
fn test_tool_result_success() {
    let result = ToolResult {
        tool_id: "clippy".to_string(),
        success: true,
        exit_code: 0,
        stdout: "Checking...".to_string(),
        stderr: String::new(),
        duration_ms: 5000,
        issues: vec![],
    };

    assert!(result.success);
    assert!(result.issues.is_empty());
}

#[test]
fn test_tool_result_with_issues() {
    let result = ToolResult {
        tool_id: "clippy".to_string(),
        success: false,
        exit_code: 1,
        stdout: String::new(),
        stderr: "warnings found".to_string(),
        duration_ms: 3000,
        issues: vec![
            ToolIssue {
                severity: IssueSeverity::Warning,
                file: Some(PathBuf::from("src/main.rs")),
                line: Some(42),
                column: Some(10),
                message: "unused variable".to_string(),
                code: Some("unused_variables".to_string()),
                fix_suggestion: Some("prefix with _".to_string()),
            },
        ],
    };

    assert!(!result.success);
    assert_eq!(result.issues.len(), 1);
}

// ============================================================================
// ToolIssue Tests
// ============================================================================

#[test]
fn test_tool_issue_creation() {
    let issue = ToolIssue {
        severity: IssueSeverity::Error,
        file: Some(PathBuf::from("src/lib.rs")),
        line: Some(10),
        column: Some(5),
        message: "type mismatch".to_string(),
        code: Some("E0308".to_string()),
        fix_suggestion: Some("check types".to_string()),
    };

    assert!(matches!(issue.severity, IssueSeverity::Error));
    assert_eq!(issue.message, "type mismatch");
    assert_eq!(issue.line, Some(10));
}

#[test]
fn test_tool_issue_without_location() {
    let issue = ToolIssue {
        severity: IssueSeverity::Warning,
        file: None,
        line: None,
        column: None,
        message: "general warning".to_string(),
        code: None,
        fix_suggestion: None,
    };

    assert!(issue.file.is_none());
    assert!(issue.fix_suggestion.is_none());
}

// ============================================================================
// IssueSeverity Tests
// ============================================================================

#[test]
fn test_issue_severity_variants() {
    assert!(matches!(IssueSeverity::Error, IssueSeverity::Error));
    assert!(matches!(IssueSeverity::Warning, IssueSeverity::Warning));
    assert!(matches!(IssueSeverity::Info, IssueSeverity::Info));
    assert!(matches!(IssueSeverity::Hint, IssueSeverity::Hint));
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_tool_workflow() {
    // Create a tool
    let tool = RuntimeTool {
        id: "rustfmt".to_string(),
        name: "Rustfmt".to_string(),
        version: "1.0".to_string(),
        tool_type: ToolType::Formatter,
        executable_path: PathBuf::from("rustfmt"),
        args_template: vec!["--check".to_string()],
        env_vars: HashMap::new(),
        timeout_ms: 30000,
        max_memory_mb: 256,
        description: "Rust formatter".to_string(),
        supported_extensions: vec!["rs".to_string()],
        health_check_cmd: None,
    };

    // Simulate execution result
    let result = ToolResult {
        tool_id: tool.id.clone(),
        success: true,
        exit_code: 0,
        stdout: "Formatting complete".to_string(),
        stderr: String::new(),
        duration_ms: 1000,
        issues: vec![],
    };

    assert_eq!(result.tool_id, tool.id);
    assert!(result.success);
}
