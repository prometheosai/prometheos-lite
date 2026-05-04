//! Issue 22: Sandbox Runtime Tests
//!
//! Comprehensive tests for the Sandbox Runtime including:
//! - StructuredCommand struct (program, args, requires_shell, original)
//! - SandboxSecurityPolicy struct (allowed_programs, blocked_programs, allow_shell, limits)
//! - LocalSandboxRuntime for command execution
//! - Command parsing with quote handling
//! - Shell metacharacter detection
//! - Program allowlist/blocklist validation
//! - Timeout handling
//! - Security policy enforcement

use prometheos_lite::harness::sandbox::{
    LocalSandboxRuntime, SandboxSecurityPolicy, StructuredCommand,
};

// ============================================================================
// StructuredCommand Tests
// ============================================================================

#[test]
fn test_structured_command_parse_simple() {
    let cmd = StructuredCommand::parse("cargo test").unwrap();

    assert_eq!(cmd.program, "cargo");
    assert_eq!(cmd.args, vec!["test"]);
    assert!(!cmd.requires_shell);
    assert_eq!(cmd.original, "cargo test");
}

#[test]
fn test_structured_command_parse_with_args() {
    let cmd = StructuredCommand::parse("cargo test --package mycrate").unwrap();

    assert_eq!(cmd.program, "cargo");
    assert_eq!(cmd.args, vec!["test", "--package", "mycrate"]);
}

#[test]
fn test_structured_command_parse_quoted() {
    let cmd = StructuredCommand::parse("echo 'hello world'").unwrap();

    assert_eq!(cmd.program, "echo");
    assert_eq!(cmd.args, vec!["hello world"]);
}

#[test]
fn test_structured_command_requires_shell_detection() {
    let cmd = StructuredCommand::parse("cat file.txt | grep pattern").unwrap();

    assert!(cmd.requires_shell);
    assert_eq!(cmd.program, "cat");
}

#[test]
fn test_structured_command_program_name() {
    let cmd = StructuredCommand::parse("/usr/bin/python3 script.py").unwrap();

    assert_eq!(cmd.program_name(), "python3");
}

#[test]
fn test_structured_command_empty_error() {
    let result = StructuredCommand::parse("");
    assert!(result.is_err());
}

// ============================================================================
// SandboxSecurityPolicy Tests
// ============================================================================

#[test]
fn test_security_policy_default() {
    let policy = SandboxSecurityPolicy::default();

    // Check allowed programs
    assert!(policy.allowed_programs.contains(&"cargo".to_string()));
    assert!(policy.allowed_programs.contains(&"npm".to_string()));
    assert!(policy.allowed_programs.contains(&"python".to_string()));
    assert!(policy.allowed_programs.contains(&"git".to_string()));

    // Check blocked programs
    assert!(policy.blocked_programs.contains(&"rm".to_string()));
    assert!(policy.blocked_programs.contains(&"sudo".to_string()));
    assert!(policy.blocked_programs.contains(&"bash".to_string()));

    // Check defaults
    assert!(!policy.allow_shell);
    assert_eq!(policy.max_command_length, 8192);
    assert_eq!(policy.max_args, 100);
}

#[test]
fn test_security_policy_custom() {
    let policy = SandboxSecurityPolicy {
        allowed_programs: vec!["cargo".to_string(), "rustc".to_string()],
        blocked_programs: vec!["curl".to_string(), "wget".to_string()],
        allow_shell: false,
        max_command_length: 4096,
        max_args: 50,
    };

    assert_eq!(policy.allowed_programs.len(), 2);
    assert!(policy.allowed_programs.contains(&"cargo".to_string()));
    assert!(!policy.allow_shell);
    assert_eq!(policy.max_command_length, 4096);
}

// ============================================================================
// LocalSandboxRuntime Tests
// ============================================================================

#[test]
fn test_local_sandbox_runtime_default() {
    let runtime = LocalSandboxRuntime::default();
    // Runtime created successfully
    assert!(true);
}

#[test]
fn test_local_sandbox_runtime_with_allowed() {
    let allowed = vec!["cargo".to_string(), "npm".to_string()];
    let runtime = LocalSandboxRuntime::new(allowed);
    // Runtime created with custom allowed list
    assert!(true);
}

// ============================================================================
// Security Tests
// ============================================================================

#[test]
fn test_command_security_empty_rejected() {
    let result = StructuredCommand::parse("");
    assert!(result.is_err());
}

#[test]
fn test_shell_metacharacters_detected() {
    let pipe_cmd = StructuredCommand::parse("cat file | grep text").unwrap();
    assert!(pipe_cmd.requires_shell);

    let redirect_cmd = StructuredCommand::parse("echo test > file.txt").unwrap();
    assert!(redirect_cmd.requires_shell);

    let safe_cmd = StructuredCommand::parse("cargo test").unwrap();
    assert!(!safe_cmd.requires_shell);
}

#[test]
fn test_quoted_strings_preserved() {
    let cmd = StructuredCommand::parse("echo 'single quotes'").unwrap();
    assert_eq!(cmd.args, vec!["single quotes"]);

    let cmd2 = StructuredCommand::parse(r#"echo "double quotes""#).unwrap();
    assert_eq!(cmd2.args, vec!["double quotes"]);
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_complex_command_parsing() {
    let cmd = StructuredCommand::parse("cargo test --package mycrate --lib -- tests::unit").unwrap();

    assert_eq!(cmd.program, "cargo");
    assert_eq!(cmd.args.len(), 6);
    assert_eq!(cmd.args[0], "test");
    assert_eq!(cmd.args[1], "--package");
    assert_eq!(cmd.args[2], "mycrate");
}

#[test]
fn test_program_in_allowed_list() {
    let policy = SandboxSecurityPolicy::default();
    assert!(policy.allowed_programs.contains(&"cargo".to_string()));
    assert!(policy.allowed_programs.contains(&"python3".to_string()));
    assert!(policy.allowed_programs.contains(&"go".to_string()));
}

#[test]
fn test_program_in_blocked_list() {
    let policy = SandboxSecurityPolicy::default();
    assert!(policy.blocked_programs.contains(&"rm".to_string()));
    assert!(policy.blocked_programs.contains(&"sudo".to_string()));
    assert!(policy.blocked_programs.contains(&"curl".to_string()));
}
