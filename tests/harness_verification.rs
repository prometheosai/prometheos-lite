//! Issue 17: Verification Strength Levels Tests
//!
//! Comprehensive tests for Verification Strength Levels including:
//! - VerificationStrength enum (7 levels from None to Full)
//! - VerificationLevel enum (FormatCheck, StaticCheck, LintCheck, etc.)
//! - VerificationAssessment struct (strength, achieved_levels, coverage, etc.)
//! - VerificationAssessor for determining achieved levels
//! - description() and requirements() methods
//! - is_sufficient_for() comparison
//! - Level mapping from commands

use prometheos_lite::harness::verification::{
    VerificationAssessment, VerificationLevel, VerificationStrength, VerificationAssessor,
};

// ============================================================================
// VerificationStrength Tests
// ============================================================================

#[test]
fn test_verification_strength_variants() {
    assert!(matches!(VerificationStrength::None, VerificationStrength::None));
    assert!(matches!(VerificationStrength::FormatOnly, VerificationStrength::FormatOnly));
    assert!(matches!(VerificationStrength::StaticOnly, VerificationStrength::StaticOnly));
    assert!(matches!(VerificationStrength::LintOnly, VerificationStrength::LintOnly));
    assert!(matches!(VerificationStrength::Tests, VerificationStrength::Tests));
    assert!(matches!(VerificationStrength::Reproduction, VerificationStrength::Reproduction));
    assert!(matches!(VerificationStrength::Full, VerificationStrength::Full));
}

#[test]
fn test_verification_strength_ordering() {
    assert!(VerificationStrength::None < VerificationStrength::FormatOnly);
    assert!(VerificationStrength::FormatOnly < VerificationStrength::StaticOnly);
    assert!(VerificationStrength::StaticOnly < VerificationStrength::LintOnly);
    assert!(VerificationStrength::LintOnly < VerificationStrength::Tests);
    assert!(VerificationStrength::Tests < VerificationStrength::Reproduction);
    assert!(VerificationStrength::Reproduction < VerificationStrength::Full);
}

#[test]
fn test_verification_strength_description() {
    assert_eq!(VerificationStrength::None.description(), "No verification performed");
    assert_eq!(VerificationStrength::FormatOnly.description(), "Code formatting verified only");
    assert_eq!(VerificationStrength::Tests.description(), "Unit tests passed");
    assert_eq!(VerificationStrength::Full.description(), "Full verification including integration tests and coverage");
}

#[test]
fn test_verification_strength_requirements() {
    let none_reqs = VerificationStrength::None.requirements();
    assert!(none_reqs.is_empty());

    let format_reqs = VerificationStrength::FormatOnly.requirements();
    assert_eq!(format_reqs, vec!["format_check"]);

    let full_reqs = VerificationStrength::Full.requirements();
    assert!(full_reqs.contains(&"format_check"));
    assert!(full_reqs.contains(&"unit_tests"));
    assert!(full_reqs.contains(&"integration_tests"));
    assert!(full_reqs.contains(&"coverage_check"));
}

#[test]
fn test_verification_strength_is_sufficient_for() {
    assert!(VerificationStrength::Full.is_sufficient_for(VerificationStrength::Tests));
    assert!(VerificationStrength::Tests.is_sufficient_for(VerificationStrength::Tests));
    assert!(!VerificationStrength::FormatOnly.is_sufficient_for(VerificationStrength::Tests));
}

// ============================================================================
// VerificationLevel Tests
// ============================================================================

#[test]
fn test_verification_level_variants() {
    assert!(matches!(VerificationLevel::FormatCheck, VerificationLevel::FormatCheck));
    assert!(matches!(VerificationLevel::StaticCheck, VerificationLevel::StaticCheck));
    assert!(matches!(VerificationLevel::LintCheck, VerificationLevel::LintCheck));
    assert!(matches!(VerificationLevel::UnitTests, VerificationLevel::UnitTests));
    assert!(matches!(VerificationLevel::IntegrationTests, VerificationLevel::IntegrationTests));
    assert!(matches!(VerificationLevel::CoverageCheck, VerificationLevel::CoverageCheck));
    assert!(matches!(VerificationLevel::ReproductionTest, VerificationLevel::ReproductionTest));
}

#[test]
fn test_verification_level_name() {
    assert_eq!(VerificationLevel::FormatCheck.name(), "Format Check");
    assert_eq!(VerificationLevel::UnitTests.name(), "Unit Tests");
    assert_eq!(VerificationLevel::CoverageCheck.name(), "Coverage Check");
}

#[test]
fn test_verification_level_description() {
    assert_eq!(VerificationLevel::FormatCheck.description(), "Code formatting and style compliance");
    assert_eq!(VerificationLevel::UnitTests.description(), "Isolated component testing");
    assert_eq!(VerificationLevel::ReproductionTest.description(), "Verification that specific issue is fixed");
}

// ============================================================================
// VerificationAssessment Tests
// ============================================================================

#[test]
fn test_verification_assessment_default() {
    let assessment = VerificationAssessment {
        strength: VerificationStrength::None,
        achieved_levels: vec![],
        missing_levels: vec![],
        coverage_percent: None,
        test_count: 0,
        passed_tests: 0,
        failed_tests: 0,
        duration_ms: 0,
    };

    assert!(matches!(assessment.strength, VerificationStrength::None));
    assert!(assessment.achieved_levels.is_empty());
    assert!(assessment.missing_levels.is_empty());
}

#[test]
fn test_verification_assessment_with_data() {
    let assessment = VerificationAssessment {
        strength: VerificationStrength::Full,
        achieved_levels: vec![
            VerificationLevel::FormatCheck,
            VerificationLevel::UnitTests,
            VerificationLevel::CoverageCheck,
        ],
        missing_levels: vec![],
        coverage_percent: Some(85.5),
        test_count: 150,
        passed_tests: 148,
        failed_tests: 2,
        duration_ms: 5000,
    };

    assert_eq!(assessment.achieved_levels.len(), 3);
    assert_eq!(assessment.coverage_percent, Some(85.5));
    assert_eq!(assessment.test_count, 150);
    assert_eq!(assessment.passed_tests, 148);
}

// ============================================================================
// VerificationAssessor Tests
// ============================================================================

#[test]
fn test_verification_assessor_new() {
    let assessor = VerificationAssessor::new();
    // Should have default level mappings
    assert!(!assessor.level_mapping.is_empty());
}

#[test]
fn test_verification_assessor_default() {
    let assessor = VerificationAssessor::default();
    assert!(!assessor.level_mapping.is_empty());
}
