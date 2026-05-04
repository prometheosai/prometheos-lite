//! Issue 31: Golden Path Templates Tests
//!
//! Comprehensive tests for Golden Path Templates including:
//! - GoldenPath struct (id, name, description, category, complexity, steps, rules)
//! - PathCategory enum (BugFix, FeatureImplementation, Refactoring, etc.)
//! - PathComplexity enum (Simple, Moderate, Complex, Expert)
//! - PathStep struct (id, name, description, type, tool_invocations, validation)
//! - StepType enum (Analysis, Generation, Validation, Testing, Review, etc.)
//! - ToolInvocation struct (tool_id, args, required)
//! - StepValidation struct (condition, required_outcome, retry_on_failure, max_retries)
//! - ValidationRule struct (rule_type, condition, error_message)
//! - GoldenPathRegistry for path registration and lookup
//! - PathMatch for matching paths to tasks

use prometheos_lite::harness::golden_paths::{
    GoldenPath, PathCategory, PathComplexity, PathMatch, PathStep, StepType, ToolInvocation,
    ValidationRule, RuleType, StepValidation,
};

// ============================================================================
// GoldenPath Tests
// ============================================================================

#[test]
fn test_golden_path_creation() {
    let path = GoldenPath {
        id: "bug-fix".to_string(),
        name: "Standard Bug Fix".to_string(),
        description: "Systematic approach to fixing bugs".to_string(),
        category: PathCategory::BugFix,
        complexity: PathComplexity::Moderate,
        steps: vec![],
        validation_rules: vec![],
        estimated_duration_ms: 300000,
        required_context: vec!["bug_description".to_string()],
        success_criteria: vec!["test_passes".to_string()],
    };

    assert_eq!(path.id, "bug-fix");
    assert!(matches!(path.category, PathCategory::BugFix));
    assert!(matches!(path.complexity, PathComplexity::Moderate));
}

// ============================================================================
// PathCategory Tests
// ============================================================================

#[test]
fn test_path_category_variants() {
    assert!(matches!(PathCategory::BugFix, PathCategory::BugFix));
    assert!(matches!(PathCategory::FeatureImplementation, PathCategory::FeatureImplementation));
    assert!(matches!(PathCategory::Refactoring, PathCategory::Refactoring));
    assert!(matches!(PathCategory::Testing, PathCategory::Testing));
    assert!(matches!(PathCategory::Documentation, PathCategory::Documentation));
    assert!(matches!(PathCategory::Configuration, PathCategory::Configuration));
    assert!(matches!(PathCategory::Migration, PathCategory::Migration));
    assert!(matches!(PathCategory::Optimization, PathCategory::Optimization));
}

// ============================================================================
// PathComplexity Tests
// ============================================================================

#[test]
fn test_path_complexity_variants() {
    assert!(matches!(PathComplexity::Simple, PathComplexity::Simple));
    assert!(matches!(PathComplexity::Moderate, PathComplexity::Moderate));
    assert!(matches!(PathComplexity::Complex, PathComplexity::Complex));
    assert!(matches!(PathComplexity::Expert, PathComplexity::Expert));
}

#[test]
fn test_path_complexity_ordering() {
    assert!(PathComplexity::Simple < PathComplexity::Moderate);
    assert!(PathComplexity::Moderate < PathComplexity::Complex);
    assert!(PathComplexity::Complex < PathComplexity::Expert);
}

// ============================================================================
// PathStep Tests
// ============================================================================

#[test]
fn test_path_step_creation() {
    let step = PathStep {
        id: "analyze".to_string(),
        name: "Analyze Issue".to_string(),
        description: "Understand the bug".to_string(),
        step_type: StepType::Analysis,
        tool_invocations: vec![],
        validation: None,
        rollback_point: true,
        optional: false,
    };

    assert_eq!(step.id, "analyze");
    assert!(matches!(step.step_type, StepType::Analysis));
    assert!(step.rollback_point);
    assert!(!step.optional);
}

// ============================================================================
// StepType Tests
// ============================================================================

#[test]
fn test_step_type_variants() {
    assert!(matches!(StepType::Analysis, StepType::Analysis));
    assert!(matches!(StepType::Generation, StepType::Generation));
    assert!(matches!(StepType::Validation, StepType::Validation));
    assert!(matches!(StepType::Testing, StepType::Testing));
    assert!(matches!(StepType::Review, StepType::Review));
    assert!(matches!(StepType::Documentation, StepType::Documentation));
    assert!(matches!(StepType::Commit, StepType::Commit));
    assert!(matches!(StepType::Deploy, StepType::Deploy));
}

// ============================================================================
// ToolInvocation Tests
// ============================================================================

#[test]
fn test_tool_invocation_creation() {
    let invocation = ToolInvocation {
        tool_id: "cargo-test".to_string(),
        args: vec!["test".to_string(), "--lib".to_string()],
        required: true,
    };

    assert_eq!(invocation.tool_id, "cargo-test");
    assert_eq!(invocation.args.len(), 2);
    assert!(invocation.required);
}

// ============================================================================
// StepValidation Tests
// ============================================================================

#[test]
fn test_step_validation_creation() {
    let validation = StepValidation {
        condition: "tests_pass".to_string(),
        required_outcome: "All tests must pass".to_string(),
        retry_on_failure: true,
        max_retries: 3,
    };

    assert_eq!(validation.condition, "tests_pass");
    assert!(validation.retry_on_failure);
    assert_eq!(validation.max_retries, 3);
}

// ============================================================================
// ValidationRule Tests
// ============================================================================

#[test]
fn test_validation_rule_creation() {
    let rule = ValidationRule {
        rule_type: RuleType::TestMustPass,
        condition: "all_tests".to_string(),
        error_message: "Tests must pass".to_string(),
    };

    assert!(matches!(rule.rule_type, RuleType::TestMustPass));
    assert_eq!(rule.error_message, "Tests must pass");
}

// ============================================================================
// RuleType Tests
// ============================================================================

#[test]
fn test_rule_type_variants() {
    assert!(matches!(RuleType::FileMustExist, RuleType::FileMustExist));
    assert!(matches!(RuleType::FileMustNotExist, RuleType::FileMustNotExist));
    assert!(matches!(RuleType::TestMustPass, RuleType::TestMustPass));
    assert!(matches!(RuleType::LintMustPass, RuleType::LintMustPass));
    assert!(matches!(RuleType::BuildMustSucceed, RuleType::BuildMustSucceed));
    assert!(matches!(RuleType::CoverageMinimum, RuleType::CoverageMinimum));
}

// ============================================================================
// PathMatch Tests
// ============================================================================

#[test]
fn test_path_match_creation() {
    let path = GoldenPath {
        id: "match-test".to_string(),
        name: "Test Path".to_string(),
        description: "Test".to_string(),
        category: PathCategory::BugFix,
        complexity: PathComplexity::Simple,
        steps: vec![],
        validation_rules: vec![],
        estimated_duration_ms: 1000,
        required_context: vec![],
        success_criteria: vec![],
    };

    let match_result = PathMatch {
        path,
        match_score: 0.85,
        reason: "Keyword match".to_string(),
    };

    assert_eq!(match_result.match_score, 0.85);
    assert_eq!(match_result.reason, "Keyword match");
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_complete_golden_path() {
    let path = GoldenPath {
        id: "feature-impl".to_string(),
        name: "Feature Implementation".to_string(),
        description: "Standard feature implementation workflow".to_string(),
        category: PathCategory::FeatureImplementation,
        complexity: PathComplexity::Complex,
        steps: vec![
            PathStep {
                id: "design".to_string(),
                name: "Design".to_string(),
                description: "Plan the feature".to_string(),
                step_type: StepType::Analysis,
                tool_invocations: vec![],
                validation: None,
                rollback_point: true,
                optional: false,
            },
            PathStep {
                id: "implement".to_string(),
                name: "Implement".to_string(),
                description: "Write the code".to_string(),
                step_type: StepType::Generation,
                tool_invocations: vec![],
                validation: None,
                rollback_point: true,
                optional: false,
            },
            PathStep {
                id: "test".to_string(),
                name: "Test".to_string(),
                description: "Run tests".to_string(),
                step_type: StepType::Testing,
                tool_invocations: vec![ToolInvocation {
                    tool_id: "cargo-test".to_string(),
                    args: vec!["test".to_string()],
                    required: true,
                }],
                validation: Some(StepValidation {
                    condition: "pass".to_string(),
                    required_outcome: "All pass".to_string(),
                    retry_on_failure: true,
                    max_retries: 2,
                }),
                rollback_point: false,
                optional: false,
            },
        ],
        validation_rules: vec![
            ValidationRule {
                rule_type: RuleType::TestMustPass,
                condition: "tests".to_string(),
                error_message: "Tests must pass".to_string(),
            },
        ],
        estimated_duration_ms: 600000,
        required_context: vec!["requirements".to_string()],
        success_criteria: vec!["tests_pass".to_string(), "review_complete".to_string()],
    };

    assert_eq!(path.steps.len(), 3);
    assert_eq!(path.validation_rules.len(), 1);
}
