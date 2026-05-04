//! Issue 12: Acceptance Criteria Compiler Tests
//!
//! Comprehensive tests for the Acceptance Criteria Compiler including:
//! - ConfidenceScore struct (score, factors, explanation, recommendation)
//! - ConfidenceFactor struct (name, weight, score, description, impact)
//! - FactorImpact enum (Positive, Neutral, Negative)
//! - AcceptanceCriteria struct (criteria, must_pass_all)
//! - CriteriaItem struct (description, weight, check_function)
//! - evaluate_acceptance function
//! - calculate_confidence function
//! - format_confidence_report function
//! - AcceptanceLevel enum (High, Medium, Low, Rejected)

use prometheos_lite::harness::acceptance::{
    AcceptanceCriteria, AcceptanceLevel, ConfidenceFactor, ConfidenceScore, CriteriaItem,
    FactorImpact, calculate_confidence, evaluate_acceptance, format_confidence_report,
};

// ============================================================================
// ConfidenceScore Tests
// ============================================================================

#[test]
fn test_confidence_score_creation() {
    let score = ConfidenceScore {
        score: 0.85,
        factors: vec![],
        explanation: "High confidence".to_string(),
        recommendation: None,
    };

    assert_eq!(score.score, 0.85);
    assert_eq!(score.explanation, "High confidence");
    assert!(score.recommendation.is_none());
}

#[test]
fn test_confidence_score_with_factors() {
    let score = ConfidenceScore {
        score: 0.75,
        factors: vec![
            ConfidenceFactor {
                name: "test_coverage".to_string(),
                weight: 0.5,
                score: 0.9,
                description: "Good test coverage".to_string(),
                impact: FactorImpact::Positive,
            },
            ConfidenceFactor {
                name: "complexity".to_string(),
                weight: 0.3,
                score: 0.6,
                description: "High complexity".to_string(),
                impact: FactorImpact::Negative,
            },
        ],
        explanation: "Mixed confidence".to_string(),
        recommendation: Some("Reduce complexity".to_string()),
    };

    assert_eq!(score.score, 0.75);
    assert_eq!(score.factors.len(), 2);
    assert_eq!(score.recommendation, Some("Reduce complexity".to_string()));
}

#[test]
fn test_confidence_score_default() {
    let score = ConfidenceScore::default();

    assert_eq!(score.score, 0.0);
    assert!(score.factors.is_empty());
    assert!(score.recommendation.is_none());
}

// ============================================================================
// ConfidenceFactor Tests
// ============================================================================

#[test]
fn test_confidence_factor_creation() {
    let factor = ConfidenceFactor {
        name: "code_quality".to_string(),
        weight: 0.4,
        score: 0.85,
        description: "Code follows best practices".to_string(),
        impact: FactorImpact::Positive,
    };

    assert_eq!(factor.name, "code_quality");
    assert_eq!(factor.weight, 0.4);
    assert_eq!(factor.score, 0.85);
    assert_eq!(factor.description, "Code follows best practices");
    assert!(matches!(factor.impact, FactorImpact::Positive));
}

#[test]
fn test_confidence_factor_negative_impact() {
    let factor = ConfidenceFactor {
        name: "bug_risk".to_string(),
        weight: 0.3,
        score: 0.2,
        description: "Potential for bugs".to_string(),
        impact: FactorImpact::Negative,
    };

    assert!(matches!(factor.impact, FactorImpact::Negative));
}

#[test]
fn test_confidence_factor_neutral_impact() {
    let factor = ConfidenceFactor {
        name: "documentation".to_string(),
        weight: 0.1,
        score: 0.5,
        description: "Documentation is adequate".to_string(),
        impact: FactorImpact::Neutral,
    };

    assert!(matches!(factor.impact, FactorImpact::Neutral));
}

// ============================================================================
// FactorImpact Tests
// ============================================================================

#[test]
fn test_factor_impact_variants() {
    assert!(matches!(FactorImpact::Positive, FactorImpact::Positive));
    assert!(matches!(FactorImpact::Neutral, FactorImpact::Neutral));
    assert!(matches!(FactorImpact::Negative, FactorImpact::Negative));
}

#[test]
fn test_factor_impact_display() {
    assert_eq!(format!("{:?}", FactorImpact::Positive), "Positive");
    assert_eq!(format!("{:?}", FactorImpact::Neutral), "Neutral");
    assert_eq!(format!("{:?}", FactorImpact::Negative), "Negative");
}

// ============================================================================
// AcceptanceCriteria Tests
// ============================================================================

#[test]
fn test_acceptance_criteria_creation() {
    let criteria = AcceptanceCriteria {
        criteria: vec![
            CriteriaItem {
                description: "All tests pass".to_string(),
                weight: 0.5,
            },
            CriteriaItem {
                description: "Code compiles without warnings".to_string(),
                weight: 0.3,
            },
        ],
        must_pass_all: true,
    };

    assert_eq!(criteria.criteria.len(), 2);
    assert!(criteria.must_pass_all);
}

#[test]
fn test_acceptance_criteria_partial_pass() {
    let criteria = AcceptanceCriteria {
        criteria: vec![
            CriteriaItem {
                description: "Critical criterion".to_string(),
                weight: 0.8,
            },
            CriteriaItem {
                description: "Optional criterion".to_string(),
                weight: 0.2,
            },
        ],
        must_pass_all: false,
    };

    assert!(!criteria.must_pass_all);
}

#[test]
fn test_acceptance_criteria_empty() {
    let criteria = AcceptanceCriteria {
        criteria: vec![],
        must_pass_all: true,
    };

    assert!(criteria.criteria.is_empty());
}

// ============================================================================
// CriteriaItem Tests
// ============================================================================

#[test]
fn test_criteria_item_creation() {
    let item = CriteriaItem {
        description: "Code review approved".to_string(),
        weight: 0.6,
    };

    assert_eq!(item.description, "Code review approved");
    assert_eq!(item.weight, 0.6);
}

#[test]
fn test_criteria_item_high_weight() {
    let item = CriteriaItem {
        description: "Security scan passed".to_string(),
        weight: 1.0,
    };

    assert_eq!(item.weight, 1.0);
}

#[test]
fn test_criteria_item_low_weight() {
    let item = CriteriaItem {
        description: "Documentation updated".to_string(),
        weight: 0.1,
    };

    assert_eq!(item.weight, 0.1);
}

// ============================================================================
// AcceptanceLevel Tests
// ============================================================================

#[test]
fn test_acceptance_level_variants() {
    assert!(matches!(AcceptanceLevel::High, AcceptanceLevel::High));
    assert!(matches!(AcceptanceLevel::Medium, AcceptanceLevel::Medium));
    assert!(matches!(AcceptanceLevel::Low, AcceptanceLevel::Low));
    assert!(matches!(AcceptanceLevel::Rejected, AcceptanceLevel::Rejected));
}

#[test]
fn test_acceptance_level_display() {
    assert_eq!(format!("{:?}", AcceptanceLevel::High), "High");
    assert_eq!(format!("{:?}", AcceptanceLevel::Medium), "Medium");
    assert_eq!(format!("{:?}", AcceptanceLevel::Low), "Low");
    assert_eq!(format!("{:?}", AcceptanceLevel::Rejected), "Rejected");
}

// ============================================================================
// calculate_confidence Tests
// ============================================================================

#[test]
fn test_calculate_confidence_high() {
    let factors = vec![
        ConfidenceFactor {
            name: "tests".to_string(),
            weight: 0.5,
            score: 0.95,
            description: "All tests pass".to_string(),
            impact: FactorImpact::Positive,
        },
        ConfidenceFactor {
            name: "coverage".to_string(),
            weight: 0.3,
            score: 0.9,
            description: "High coverage".to_string(),
            impact: FactorImpact::Positive,
        },
    ];

    let score = calculate_confidence(&factors);
    assert!(score > 0.8);
}

#[test]
fn test_calculate_confidence_low() {
    let factors = vec![
        ConfidenceFactor {
            name: "tests".to_string(),
            weight: 0.5,
            score: 0.3,
            description: "Many tests fail".to_string(),
            impact: FactorImpact::Negative,
        },
        ConfidenceFactor {
            name: "complexity".to_string(),
            weight: 0.5,
            score: 0.2,
            description: "High complexity".to_string(),
            impact: FactorImpact::Negative,
        },
    ];

    let score = calculate_confidence(&factors);
    assert!(score < 0.5);
}

#[test]
fn test_calculate_confidence_empty() {
    let factors = vec![];
    let score = calculate_confidence(&factors);
    assert_eq!(score, 0.0);
}

// ============================================================================
// evaluate_acceptance Tests
// ============================================================================

#[test]
fn test_evaluate_acceptance_pass() {
    let criteria = AcceptanceCriteria {
        criteria: vec![
            CriteriaItem {
                description: "Tests pass".to_string(),
                weight: 0.5,
            },
        ],
        must_pass_all: true,
    };

    let results = vec![true];
    let level = evaluate_acceptance(&criteria, &results);
    assert!(matches!(level, AcceptanceLevel::High | AcceptanceLevel::Medium));
}

#[test]
fn test_evaluate_acceptance_fail() {
    let criteria = AcceptanceCriteria {
        criteria: vec![
            CriteriaItem {
                description: "Tests pass".to_string(),
                weight: 1.0,
            },
        ],
        must_pass_all: true,
    };

    let results = vec![false];
    let level = evaluate_acceptance(&criteria, &results);
    assert!(matches!(level, AcceptanceLevel::Rejected));
}

#[test]
fn test_evaluate_acceptance_partial() {
    let criteria = AcceptanceCriteria {
        criteria: vec![
            CriteriaItem {
                description: "Critical".to_string(),
                weight: 0.7,
            },
            CriteriaItem {
                description: "Optional".to_string(),
                weight: 0.3,
            },
        ],
        must_pass_all: false,
    };

    let results = vec![true, false];
    let level = evaluate_acceptance(&criteria, &results);
    assert!(matches!(level, AcceptanceLevel::Medium | AcceptanceLevel::Low));
}

// ============================================================================
// format_confidence_report Tests
// ============================================================================

#[test]
fn test_format_confidence_report_high() {
    let score = ConfidenceScore {
        score: 0.9,
        factors: vec![
            ConfidenceFactor {
                name: "quality".to_string(),
                weight: 0.5,
                score: 0.95,
                description: "Excellent".to_string(),
                impact: FactorImpact::Positive,
            },
        ],
        explanation: "High confidence".to_string(),
        recommendation: None,
    };

    let report = format_confidence_report(&score);
    assert!(!report.is_empty());
    assert!(report.contains("High confidence") || report.contains("0.9"));
}

#[test]
fn test_format_confidence_report_with_recommendation() {
    let score = ConfidenceScore {
        score: 0.6,
        factors: vec![],
        explanation: "Medium confidence".to_string(),
        recommendation: Some("Add more tests".to_string()),
    };

    let report = format_confidence_report(&score);
    assert!(!report.is_empty());
}

#[test]
fn test_format_confidence_report_with_factors() {
    let score = ConfidenceScore {
        score: 0.75,
        factors: vec![
            ConfidenceFactor {
                name: "factor1".to_string(),
                weight: 0.5,
                score: 0.8,
                description: "Good".to_string(),
                impact: FactorImpact::Positive,
            },
            ConfidenceFactor {
                name: "factor2".to_string(),
                weight: 0.5,
                score: 0.7,
                description: "Okay".to_string(),
                impact: FactorImpact::Neutral,
            },
        ],
        explanation: "Good confidence".to_string(),
        recommendation: None,
    };

    let report = format_confidence_report(&score);
    assert!(!report.is_empty());
}
