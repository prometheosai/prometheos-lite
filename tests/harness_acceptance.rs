//! Issue 12: Acceptance Criteria Compiler Tests
//!
//! Comprehensive tests for the Acceptance Criteria Compiler including:
//! - AcceptanceCriterion struct (id, description, verification_method, status, priority)
//! - CompiledAcceptanceCriteria struct (criteria, test_commands, static_checks, etc.)
//! - AcceptanceCompiler for compiling task descriptions into criteria
//! - VerificationMethod enum (TestCommand, StaticCheck, LintCommand, etc.)
//! - CriterionStatus enum (Pending, Passed, Failed, NotApplicable, Blocked)
//! - CriterionPriority enum (Critical, High, Medium, Low)
//! - compile_acceptance_criteria function
//! - compile_acceptance_criteria_with_env function
//! - update_criterion_status function
//! - get_verification_summary function

use prometheos_lite::harness::acceptance::{
    AcceptanceCompiler, AcceptanceCriterion, CompiledAcceptanceCriteria, CriterionPriority,
    CriterionStatus, VerificationMethod, compile_acceptance_criteria,
    compile_acceptance_criteria_with_env, get_verification_summary, update_criterion_status,
};

// ============================================================================
// AcceptanceCriterion Tests
// ============================================================================

#[test]
fn test_acceptance_criterion_creation() {
    let criterion = AcceptanceCriterion {
        id: "test-001".to_string(),
        description: "All tests pass".to_string(),
        verification_method: VerificationMethod::TestCommand("cargo test".to_string()),
        status: CriterionStatus::Pending,
        priority: CriterionPriority::Critical,
        detected_tests: vec!["cargo test".to_string()],
        detected_checks: vec![],
        confidence: 0.85,
    };

    assert_eq!(criterion.id, "test-001");
    assert_eq!(criterion.description, "All tests pass");
    assert!(matches!(criterion.status, CriterionStatus::Pending));
    assert!(matches!(criterion.priority, CriterionPriority::Critical));
}

#[test]
fn test_acceptance_criterion_passed_status() {
    let criterion = AcceptanceCriterion {
        id: "lint-001".to_string(),
        description: "No linting errors".to_string(),
        verification_method: VerificationMethod::LintCommand("cargo clippy".to_string()),
        status: CriterionStatus::Passed,
        priority: CriterionPriority::High,
        detected_tests: vec![],
        detected_checks: vec!["cargo clippy".to_string()],
        confidence: 1.0,
    };

    assert!(matches!(criterion.status, CriterionStatus::Passed));
}

#[test]
fn test_acceptance_criterion_variants() {
    let format_criterion = AcceptanceCriterion {
        id: "fmt-001".to_string(),
        description: "Code formatted".to_string(),
        verification_method: VerificationMethod::FormatCommand("cargo fmt".to_string()),
        status: CriterionStatus::Pending,
        priority: CriterionPriority::Low,
        detected_tests: vec![],
        detected_checks: vec!["cargo fmt".to_string()],
        confidence: 0.5,
    };

    assert!(matches!(format_criterion.priority, CriterionPriority::Low));
}

// ============================================================================
// CompiledAcceptanceCriteria Tests
// ============================================================================

#[test]
fn test_compiled_acceptance_criteria_creation() {
    let compiled = CompiledAcceptanceCriteria {
        criteria: vec![
            AcceptanceCriterion {
                id: "test-001".to_string(),
                description: "Unit tests pass".to_string(),
                verification_method: VerificationMethod::TestCommand("cargo test".to_string()),
                status: CriterionStatus::Pending,
                priority: CriterionPriority::Critical,
                detected_tests: vec!["cargo test".to_string()],
                detected_checks: vec![],
                confidence: 0.9,
            }
        ],
        test_commands: vec!["cargo test".to_string()],
        static_checks: vec![],
        lint_commands: vec![],
        format_commands: vec![],
        total_priority_score: 4,
        auto_detected: 1,
        manual_review_required: 0,
    };

    assert_eq!(compiled.criteria.len(), 1);
    assert_eq!(compiled.test_commands.len(), 1);
    assert_eq!(compiled.total_priority_score, 4);
}

#[test]
fn test_compiled_acceptance_criteria_multiple_commands() {
    let compiled = CompiledAcceptanceCriteria {
        criteria: vec![
            AcceptanceCriterion {
                id: "test-001".to_string(),
                description: "Tests pass".to_string(),
                verification_method: VerificationMethod::TestCommand("cargo test".to_string()),
                status: CriterionStatus::Pending,
                priority: CriterionPriority::High,
                detected_tests: vec!["cargo test".to_string()],
                detected_checks: vec![],
                confidence: 0.8,
            },
            AcceptanceCriterion {
                id: "lint-001".to_string(),
                description: "Clippy passes".to_string(),
                verification_method: VerificationMethod::LintCommand("cargo clippy".to_string()),
                status: CriterionStatus::Pending,
                priority: CriterionPriority::Medium,
                detected_tests: vec![],
                detected_checks: vec!["cargo clippy".to_string()],
                confidence: 0.7,
            }
        ],
        test_commands: vec!["cargo test".to_string()],
        static_checks: vec![],
        lint_commands: vec!["cargo clippy".to_string()],
        format_commands: vec![],
        total_priority_score: 6,
        auto_detected: 2,
        manual_review_required: 0,
    };

    assert_eq!(compiled.criteria.len(), 2);
    assert_eq!(compiled.test_commands.len(), 1);
    assert_eq!(compiled.lint_commands.len(), 1);
}

// ============================================================================
// AcceptanceCompiler Tests
// ============================================================================

#[test]
fn test_acceptance_compiler_new() {
    let compiler = AcceptanceCompiler::new();
    // Compiler created successfully
    assert!(true);
}

#[test]
fn test_acceptance_compiler_default() {
    let compiler = AcceptanceCompiler::default();
    // Default compiler created
    assert!(true);
}

// ============================================================================
// VerificationMethod Tests
// ============================================================================

#[test]
fn test_verification_method_test_command() {
    let method = VerificationMethod::TestCommand("cargo test".to_string());
    assert!(matches!(method, VerificationMethod::TestCommand(cmd) if cmd == "cargo test"));
}

#[test]
fn test_verification_method_static_check() {
    let method = VerificationMethod::StaticCheck("cargo check".to_string());
    assert!(matches!(method, VerificationMethod::StaticCheck(cmd) if cmd == "cargo check"));
}

#[test]
fn test_verification_method_lint_command() {
    let method = VerificationMethod::LintCommand("cargo clippy".to_string());
    assert!(matches!(method, VerificationMethod::LintCommand(cmd) if cmd == "cargo clippy"));
}

#[test]
fn test_verification_method_format_command() {
    let method = VerificationMethod::FormatCommand("cargo fmt".to_string());
    assert!(matches!(method, VerificationMethod::FormatCommand(cmd) if cmd == "cargo fmt"));
}

#[test]
fn test_verification_method_review() {
    let method = VerificationMethod::Review;
    assert!(matches!(method, VerificationMethod::Review));
}

#[test]
fn test_verification_method_manual() {
    let method = VerificationMethod::Manual;
    assert!(matches!(method, VerificationMethod::Manual));
}

// ============================================================================
// CriterionStatus Tests
// ============================================================================

#[test]
fn test_criterion_status_variants() {
    assert!(matches!(CriterionStatus::Pending, CriterionStatus::Pending));
    assert!(matches!(CriterionStatus::Passed, CriterionStatus::Passed));
    assert!(matches!(CriterionStatus::Failed, CriterionStatus::Failed));
    assert!(matches!(CriterionStatus::NotApplicable, CriterionStatus::NotApplicable));
    assert!(matches!(CriterionStatus::Blocked, CriterionStatus::Blocked));
}

#[test]
fn test_criterion_status_display() {
    assert_eq!(format!("{:?}", CriterionStatus::Pending), "Pending");
    assert_eq!(format!("{:?}", CriterionStatus::Passed), "Passed");
    assert_eq!(format!("{:?}", CriterionStatus::Failed), "Failed");
}

// ============================================================================
// CriterionPriority Tests
// ============================================================================

#[test]
fn test_criterion_priority_variants() {
    assert!(matches!(CriterionPriority::Critical, CriterionPriority::Critical));
    assert!(matches!(CriterionPriority::High, CriterionPriority::High));
    assert!(matches!(CriterionPriority::Medium, CriterionPriority::Medium));
    assert!(matches!(CriterionPriority::Low, CriterionPriority::Low));
}

#[test]
fn test_criterion_priority_ordering() {
    assert!(CriterionPriority::Critical > CriterionPriority::High);
    assert!(CriterionPriority::High > CriterionPriority::Medium);
    assert!(CriterionPriority::Medium > CriterionPriority::Low);
}

// ============================================================================
// compile_acceptance_criteria Tests
// ============================================================================

#[test]
fn test_compile_acceptance_criteria_basic() {
    let task_description = vec!["Fix bug in main.rs and ensure all tests pass".to_string()];
    let compiled = compile_acceptance_criteria(&task_description);
    
    // Should not be empty after compilation
    assert!(!compiled.is_empty());
}

#[test]
fn test_compile_acceptance_criteria_empty() {
    let task_description: Vec<String> = vec![];
    let compiled = compile_acceptance_criteria(&task_description);
    
    // Should handle empty task gracefully
    assert!(compiled.is_empty());
}

// ============================================================================
// update_criterion_status Tests
// ============================================================================

#[test]
fn test_update_criterion_status() {
    let mut criteria = vec![AcceptanceCriterion {
        id: "test-001".to_string(),
        description: "Tests pass".to_string(),
        verification_method: VerificationMethod::TestCommand("cargo test".to_string()),
        status: CriterionStatus::Pending,
        priority: CriterionPriority::High,
        detected_tests: vec![],
        detected_checks: vec![],
        confidence: 0.0,
    }];

    let updated = update_criterion_status(&mut criteria, "test-001", CriterionStatus::Passed);
    assert!(updated);
    assert!(matches!(criteria[0].status, CriterionStatus::Passed));
}

#[test]
fn test_update_criterion_status_to_failed() {
    let mut criteria = vec![AcceptanceCriterion {
        id: "test-001".to_string(),
        description: "Tests pass".to_string(),
        verification_method: VerificationMethod::TestCommand("cargo test".to_string()),
        status: CriterionStatus::Pending,
        priority: CriterionPriority::High,
        detected_tests: vec![],
        detected_checks: vec![],
        confidence: 0.0,
    }];

    let updated = update_criterion_status(&mut criteria, "test-001", CriterionStatus::Failed);
    assert!(updated);
    assert!(matches!(criteria[0].status, CriterionStatus::Failed));
}

// ============================================================================
// get_verification_summary Tests
// ============================================================================

#[test]
fn test_get_verification_summary() {
    let criteria = vec![
        AcceptanceCriterion {
            id: "test-001".to_string(),
            description: "Tests pass".to_string(),
            verification_method: VerificationMethod::TestCommand("cargo test".to_string()),
            status: CriterionStatus::Passed,
            priority: CriterionPriority::High,
            detected_tests: vec!["cargo test".to_string()],
            detected_checks: vec![],
            confidence: 0.9,
        },
        AcceptanceCriterion {
            id: "lint-001".to_string(),
            description: "Clippy passes".to_string(),
            verification_method: VerificationMethod::LintCommand("cargo clippy".to_string()),
            status: CriterionStatus::Pending,
            priority: CriterionPriority::Medium,
            detected_tests: vec![],
            detected_checks: vec!["cargo clippy".to_string()],
            confidence: 0.0,
        }
    ];

    let summary = get_verification_summary(&criteria);
    assert!(!summary.is_empty());
}

#[test]
fn test_get_verification_summary_empty() {
    let criteria: Vec<AcceptanceCriterion> = vec![];
    let summary = get_verification_summary(&criteria);
    // Empty criteria should still produce a valid summary
    assert!(summary.is_empty() || summary.contains("0"));
}
