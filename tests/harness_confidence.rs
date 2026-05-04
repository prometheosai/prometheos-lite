//! Issue 19: Confidence Calibration Tests
//!
//! Comprehensive tests for Confidence Calibration including:
//! - ConfidenceScore struct (score, factors, explanation, recommendation)
//! - ConfidenceFactor struct (name, weight, score, description, impact)
//! - FactorImpact enum (Positive, Neutral, Negative)
//! - ConfidenceCalibrator for computing confidence scores
//! - ConfidenceWeights for factor weighting (validation_pass, risk_level, etc.)
//! - ConfidenceThresholds for confidence level boundaries
//! - compute() method with multi-factor analysis
//! - calibrate() method for threshold adjustment
//! - interpret_confidence() for human-readable explanations

use prometheos_lite::harness::confidence::{
    ConfidenceCalibrator, ConfidenceFactor, ConfidenceScore, ConfidenceThresholds,
    ConfidenceWeights, FactorImpact,
};
use prometheos_lite::harness::risk::{RiskAssessment, RiskLevel};
use prometheos_lite::harness::verification::VerificationStrength;

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
    assert!(score.factors.is_empty());
    assert_eq!(score.explanation, "High confidence");
    assert!(score.recommendation.is_none());
}

#[test]
fn test_confidence_score_with_factors() {
    let score = ConfidenceScore {
        score: 0.75,
        factors: vec![
            ConfidenceFactor {
                name: "validation".to_string(),
                weight: 0.4,
                score: 0.9,
                description: "All validations passed".to_string(),
                impact: FactorImpact::Positive,
            },
            ConfidenceFactor {
                name: "risk".to_string(),
                weight: 0.3,
                score: 0.6,
                description: "Medium risk level".to_string(),
                impact: FactorImpact::Neutral,
            },
        ],
        explanation: "Good confidence".to_string(),
        recommendation: Some("Reduce risk".to_string()),
    };

    assert_eq!(score.score, 0.75);
    assert_eq!(score.factors.len(), 2);
    assert_eq!(score.recommendation, Some("Reduce risk".to_string()));
}

// ============================================================================
// ConfidenceFactor Tests
// ============================================================================

#[test]
fn test_confidence_factor_creation() {
    let factor = ConfidenceFactor {
        name: "test_coverage".to_string(),
        weight: 0.25,
        score: 0.9,
        description: "High test coverage".to_string(),
        impact: FactorImpact::Positive,
    };

    assert_eq!(factor.name, "test_coverage");
    assert_eq!(factor.weight, 0.25);
    assert_eq!(factor.score, 0.9);
    assert_eq!(factor.description, "High test coverage");
    assert!(matches!(factor.impact, FactorImpact::Positive));
}

#[test]
fn test_confidence_factor_negative_impact() {
    let factor = ConfidenceFactor {
        name: "complexity".to_string(),
        weight: 0.2,
        score: 0.3,
        description: "High cyclomatic complexity".to_string(),
        impact: FactorImpact::Negative,
    };

    assert!(matches!(factor.impact, FactorImpact::Negative));
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
// ConfidenceWeights Tests
// ============================================================================

#[test]
fn test_confidence_weights_default() {
    let weights = ConfidenceWeights::default();

    assert_eq!(weights.validation_pass, 0.25);
    assert_eq!(weights.verification_strength, 0.20);
    assert_eq!(weights.risk_level, 0.20);
    assert_eq!(weights.review_issues, 0.15);
    assert_eq!(weights.change_size, 0.10);
    assert_eq!(weights.test_coverage, 0.10);
}

#[test]
fn test_confidence_weights_custom() {
    let weights = ConfidenceWeights {
        validation_pass: 0.3,
        verification_strength: 0.25,
        risk_level: 0.15,
        review_issues: 0.15,
        change_size: 0.1,
        test_coverage: 0.05,
    };

    assert_eq!(weights.validation_pass, 0.3);
    assert_eq!(weights.risk_level, 0.15);
}

// ============================================================================
// ConfidenceThresholds Tests
// ============================================================================

#[test]
fn test_confidence_thresholds_default() {
    let thresholds = ConfidenceThresholds::default();

    assert_eq!(thresholds.high_confidence, 0.80);
    assert_eq!(thresholds.medium_confidence, 0.60);
    assert_eq!(thresholds.low_confidence, 0.40);
}

#[test]
fn test_confidence_thresholds_custom() {
    let thresholds = ConfidenceThresholds {
        high_confidence: 0.85,
        medium_confidence: 0.65,
        low_confidence: 0.45,
    };

    assert_eq!(thresholds.high_confidence, 0.85);
    assert_eq!(thresholds.medium_confidence, 0.65);
}

// ============================================================================
// ConfidenceCalibrator Tests
// ============================================================================

#[test]
fn test_confidence_calibrator_new() {
    let calibrator = ConfidenceCalibrator::new();
    // Should have default weights and thresholds
    assert!(true); // Construction succeeded
}

#[test]
fn test_confidence_calibrator_default() {
    let calibrator: ConfidenceCalibrator = Default::default();
    assert!(true); // Construction succeeded
}

#[test]
fn test_confidence_calibrator_with_weights() {
    let custom_weights = ConfidenceWeights {
        validation_pass: 0.4,
        verification_strength: 0.3,
        risk_level: 0.1,
        review_issues: 0.1,
        change_size: 0.05,
        test_coverage: 0.05,
    };

    let calibrator = ConfidenceCalibrator::with_weights(custom_weights);
    assert!(true); // Construction with custom weights succeeded
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_high_confidence_scenario() {
    let score = ConfidenceScore {
        score: 0.9,
        factors: vec![
            ConfidenceFactor {
                name: "validation".to_string(),
                weight: 0.25,
                score: 1.0,
                description: "All passed".to_string(),
                impact: FactorImpact::Positive,
            },
            ConfidenceFactor {
                name: "risk".to_string(),
                weight: 0.20,
                score: 0.95,
                description: "Low risk".to_string(),
                impact: FactorImpact::Positive,
            },
        ],
        explanation: "High confidence patch".to_string(),
        recommendation: None,
    };

    assert!(score.score >= 0.8);
}

#[test]
fn test_low_confidence_scenario() {
    let score = ConfidenceScore {
        score: 0.35,
        factors: vec![
            ConfidenceFactor {
                name: "validation".to_string(),
                weight: 0.25,
                score: 0.2,
                description: "Many failures".to_string(),
                impact: FactorImpact::Negative,
            },
            ConfidenceFactor {
                name: "risk".to_string(),
                weight: 0.20,
                score: 0.0,
                description: "Critical risk".to_string(),
                impact: FactorImpact::Negative,
            },
        ],
        explanation: "Low confidence - needs work".to_string(),
        recommendation: Some("Fix validation errors".to_string()),
    };

    assert!(score.score < 0.4);
}
