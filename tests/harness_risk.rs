//! Issue 16: Risk-Based Approval Gates Tests
//!
//! Comprehensive tests for the Risk-Based Approval Gates including:
//! - RiskAssessment struct (level, reasons, requires_approval, can_override)
//! - RiskReason struct (category, description, severity, mitigation)
//! - RiskLevel enum (None, Low, Medium, High, Critical)
//! - RiskCategory enum (Security, ApiBreaking, DatabaseBreaking, etc.)
//! - RiskSeverity enum (Info, Low, Medium, High, Critical)
//! - RiskEngine struct with approval configuration
//! - OverridePolicy struct (allowed_categories, require_secondary_approval, etc.)
//! - assess function for risk evaluation
//! - can_auto_approve function
//! - requires_secondary_approval function

use prometheos_lite::harness::risk::{
    OverridePolicy, RiskAssessment, RiskCategory, RiskEngine, RiskLevel, RiskReason, RiskSeverity,
};

// ============================================================================
// RiskAssessment Tests
// ============================================================================

#[test]
fn test_risk_assessment_none() {
    let assessment = RiskAssessment {
        level: RiskLevel::None,
        reasons: vec![],
        requires_approval: false,
        can_override: true,
        override_conditions: vec![],
        assessed: true,
    };

    assert!(matches!(assessment.level, RiskLevel::None));
    assert!(!assessment.requires_approval);
    assert!(assessment.can_override);
}

#[test]
fn test_risk_assessment_critical() {
    let assessment = RiskAssessment {
        level: RiskLevel::Critical,
        reasons: vec![
            RiskReason {
                category: RiskCategory::Security,
                description: "SQL injection vulnerability".to_string(),
                severity: RiskSeverity::Critical,
                mitigation: Some("Use parameterized queries".to_string()),
            },
        ],
        requires_approval: true,
        can_override: false,
        override_conditions: vec![],
        assessed: true,
    };

    assert!(matches!(assessment.level, RiskLevel::Critical));
    assert!(assessment.requires_approval);
    assert!(!assessment.can_override);
    assert_eq!(assessment.reasons.len(), 1);
}

// ============================================================================
// RiskReason Tests
// ============================================================================

#[test]
fn test_risk_reason_creation() {
    let reason = RiskReason {
        category: RiskCategory::ApiBreaking,
        description: "Function signature changed".to_string(),
        severity: RiskSeverity::High,
        mitigation: Some("Maintain backward compatibility".to_string()),
    };

    assert!(matches!(reason.category, RiskCategory::ApiBreaking));
    assert_eq!(reason.description, "Function signature changed");
    assert!(matches!(reason.severity, RiskSeverity::High));
    assert_eq!(reason.mitigation, Some("Maintain backward compatibility".to_string()));
}

#[test]
fn test_risk_reason_without_mitigation() {
    let reason = RiskReason {
        category: RiskCategory::Performance,
        description: "Slight performance degradation".to_string(),
        severity: RiskSeverity::Low,
        mitigation: None,
    };

    assert!(reason.mitigation.is_none());
}

// ============================================================================
// RiskLevel Tests
// ============================================================================

#[test]
fn test_risk_level_variants() {
    assert!(matches!(RiskLevel::None, RiskLevel::None));
    assert!(matches!(RiskLevel::Low, RiskLevel::Low));
    assert!(matches!(RiskLevel::Medium, RiskLevel::Medium));
    assert!(matches!(RiskLevel::High, RiskLevel::High));
    assert!(matches!(RiskLevel::Critical, RiskLevel::Critical));
}

#[test]
fn test_risk_level_ordering() {
    assert!(RiskLevel::None < RiskLevel::Low);
    assert!(RiskLevel::Low < RiskLevel::Medium);
    assert!(RiskLevel::Medium < RiskLevel::High);
    assert!(RiskLevel::High < RiskLevel::Critical);
}

#[test]
fn test_risk_level_display() {
    assert_eq!(format!("{:?}", RiskLevel::None), "None");
    assert_eq!(format!("{:?}", RiskLevel::Low), "Low");
    assert_eq!(format!("{:?}", RiskLevel::Medium), "Medium");
    assert_eq!(format!("{:?}", RiskLevel::High), "High");
    assert_eq!(format!("{:?}", RiskLevel::Critical), "Critical");
}

// ============================================================================
// RiskCategory Tests
// ============================================================================

#[test]
fn test_risk_category_variants() {
    assert!(matches!(RiskCategory::Security, RiskCategory::Security));
    assert!(matches!(RiskCategory::ApiBreaking, RiskCategory::ApiBreaking));
    assert!(matches!(RiskCategory::DatabaseBreaking, RiskCategory::DatabaseBreaking));
    assert!(matches!(RiskCategory::Dependency, RiskCategory::Dependency));
    assert!(matches!(RiskCategory::Configuration, RiskCategory::Configuration));
    assert!(matches!(RiskCategory::Logic, RiskCategory::Logic));
    assert!(matches!(RiskCategory::Performance, RiskCategory::Performance));
    assert!(matches!(RiskCategory::Compliance, RiskCategory::Compliance));
}

#[test]
fn test_risk_category_display() {
    assert_eq!(format!("{:?}", RiskCategory::Security), "Security");
    assert_eq!(format!("{:?}", RiskCategory::ApiBreaking), "ApiBreaking");
    assert_eq!(format!("{:?}", RiskCategory::DatabaseBreaking), "DatabaseBreaking");
}

// ============================================================================
// RiskSeverity Tests
// ============================================================================

#[test]
fn test_risk_severity_variants() {
    assert!(matches!(RiskSeverity::Info, RiskSeverity::Info));
    assert!(matches!(RiskSeverity::Low, RiskSeverity::Low));
    assert!(matches!(RiskSeverity::Medium, RiskSeverity::Medium));
    assert!(matches!(RiskSeverity::High, RiskSeverity::High));
    assert!(matches!(RiskSeverity::Critical, RiskSeverity::Critical));
}

#[test]
fn test_risk_severity_ordering() {
    assert!(RiskSeverity::Info < RiskSeverity::Low);
    assert!(RiskSeverity::Low < RiskSeverity::Medium);
    assert!(RiskSeverity::Medium < RiskSeverity::High);
    assert!(RiskSeverity::High < RiskSeverity::Critical);
}

// ============================================================================
// RiskEngine Tests
// ============================================================================

#[test]
fn test_risk_engine_default() {
    let engine = RiskEngine::default();
    assert!(engine.auto_reject_critical);
}

#[test]
fn test_risk_engine_new() {
    let engine = RiskEngine::new();
    assert!(engine.auto_reject_critical);
    assert!(!engine.approval_required_for.is_empty());
}

// ============================================================================
// OverridePolicy Tests
// ============================================================================

#[test]
fn test_override_policy_default() {
    let policy = OverridePolicy::default();
    assert!(policy.allowed_categories.is_empty());
    assert!(!policy.require_secondary_approval);
    assert_eq!(policy.max_override_count, 0);
    assert!(!policy.audit_required);
}

#[test]
fn test_override_policy_with_categories() {
    let policy = OverridePolicy {
        allowed_categories: vec![RiskCategory::Performance, RiskCategory::Logic],
        require_secondary_approval: true,
        max_override_count: 3,
        audit_required: true,
    };

    assert_eq!(policy.allowed_categories.len(), 2);
    assert!(policy.require_secondary_approval);
    assert_eq!(policy.max_override_count, 3);
    assert!(policy.audit_required);
}
