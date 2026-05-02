use crate::harness::{
    review::{ReviewIssue, ReviewSeverity},
    risk::{RiskAssessment, RiskLevel},
    validation::ValidationResult,
    verification::VerificationStrength,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfidenceScore {
    pub score: f32,
    pub factors: Vec<ConfidenceFactor>,
    pub explanation: String,
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfidenceFactor {
    pub name: String,
    pub weight: f32,
    pub score: f32,
    pub description: String,
    pub impact: FactorImpact,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FactorImpact {
    Positive,
    Negative,
    Neutral,
}

#[derive(Debug, Clone)]
pub struct ConfidenceCalibrator {
    weights: ConfidenceWeights,
    thresholds: ConfidenceThresholds,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfidenceWeights {
    pub validation_pass: f32,
    pub verification_strength: f32,
    pub risk_level: f32,
    pub review_issues: f32,
    pub change_size: f32,
    pub test_coverage: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfidenceThresholds {
    pub high_confidence: f32,
    pub medium_confidence: f32,
    pub low_confidence: f32,
}

impl Default for ConfidenceWeights {
    fn default() -> Self {
        Self {
            validation_pass: 0.25,
            verification_strength: 0.20,
            risk_level: 0.20,
            review_issues: 0.15,
            change_size: 0.10,
            test_coverage: 0.10,
        }
    }
}

impl Default for ConfidenceThresholds {
    fn default() -> Self {
        Self {
            high_confidence: 0.80,
            medium_confidence: 0.60,
            low_confidence: 0.40,
        }
    }
}

impl Default for ConfidenceCalibrator {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfidenceCalibrator {
    pub fn new() -> Self {
        Self {
            weights: ConfidenceWeights::default(),
            thresholds: ConfidenceThresholds::default(),
        }
    }

    pub fn with_weights(weights: ConfidenceWeights) -> Self {
        Self {
            weights,
            thresholds: ConfidenceThresholds::default(),
        }
    }

    pub fn compute(
        &self,
        validation: Option<&ValidationResult>,
        issues: &[ReviewIssue],
        risk: &RiskAssessment,
        verification: VerificationStrength,
        lines_changed: usize,
        test_files_touched: usize,
    ) -> ConfidenceScore {
        let mut factors = vec![];

        // Factor 1: Validation Result (25%)
        let validation_score = if validation.is_some_and(|v| v.passed) {
            1.0
        } else if validation.is_some() {
            0.3
        } else {
            0.0
        };
        let validation_contribution = validation_score * self.weights.validation_pass;
        factors.push(ConfidenceFactor {
            name: "Validation Pass".to_string(),
            weight: self.weights.validation_pass,
            score: validation_score,
            description: if validation.is_some_and(|v| v.passed) {
                "All validation commands passed".to_string()
            } else if validation.is_some() {
                "Some validation commands failed".to_string()
            } else {
                "No validation performed".to_string()
            },
            impact: if validation_score > 0.7 { FactorImpact::Positive } else { FactorImpact::Negative },
        });

        // Factor 2: Verification Strength (20%)
        let verification_score = match verification {
            VerificationStrength::Full => 1.0,
            VerificationStrength::Tests => 0.9,
            VerificationStrength::Reproduction => 0.85,
            VerificationStrength::LintOnly => 0.7,
            VerificationStrength::FormatOnly => 0.6,
            VerificationStrength::StaticOnly => 0.8,
            VerificationStrength::None => 0.0,
        };
        let verification_contribution = verification_score * self.weights.verification_strength;
        factors.push(ConfidenceFactor {
            name: "Verification Strength".to_string(),
            weight: self.weights.verification_strength,
            score: verification_score,
            description: format!("Verification level: {:?}", verification),
            impact: if verification_score > 0.7 { FactorImpact::Positive } else { FactorImpact::Negative },
        });

        // Factor 3: Risk Level (20%)
        let risk_score = match risk.level {
            RiskLevel::None => 1.0,
            RiskLevel::Low => 0.9,
            RiskLevel::Medium => 0.7,
            RiskLevel::High => 0.4,
            RiskLevel::Critical => 0.0,
        };
        let risk_contribution = risk_score * self.weights.risk_level;
        factors.push(ConfidenceFactor {
            name: "Risk Level".to_string(),
            weight: self.weights.risk_level,
            score: risk_score,
            description: format!("Risk assessment: {:?}", risk.level),
            impact: if risk_score > 0.7 { FactorImpact::Positive } else { FactorImpact::Negative },
        });

        // Factor 4: Review Issues (15%)
        let critical_count = issues.iter().filter(|i| i.severity == ReviewSeverity::Critical).count();
        let high_count = issues.iter().filter(|i| i.severity == ReviewSeverity::High).count();
        let medium_count = issues.iter().filter(|i| i.severity == ReviewSeverity::Medium).count();
        
        let issue_penalty = (critical_count as f32 * 0.4) + (high_count as f32 * 0.2) + (medium_count as f32 * 0.1);
        let issue_score = (1.0 - issue_penalty).max(0.0);
        let issue_contribution = issue_score * self.weights.review_issues;
        factors.push(ConfidenceFactor {
            name: "Review Issues".to_string(),
            weight: self.weights.review_issues,
            score: issue_score,
            description: format!("{} critical, {} high, {} medium issues", critical_count, high_count, medium_count),
            impact: if issue_score > 0.7 { FactorImpact::Positive } else { FactorImpact::Negative },
        });

        // Factor 5: Change Size (10%)
        let size_score = if lines_changed < 50 {
            1.0
        } else if lines_changed < 200 {
            0.8
        } else if lines_changed < 500 {
            0.6
        } else {
            0.4
        };
        let size_contribution = size_score * self.weights.change_size;
        factors.push(ConfidenceFactor {
            name: "Change Size".to_string(),
            weight: self.weights.change_size,
            score: size_score,
            description: format!("{} lines changed", lines_changed),
            impact: FactorImpact::Neutral,
        });

        // Factor 6: Test Coverage (10%)
        let test_score = if test_files_touched > 0 {
            1.0
        } else if validation.is_some_and(|v| !v.passed) {
            0.0
        } else {
            0.5
        };
        let test_contribution = test_score * self.weights.test_coverage;
        factors.push(ConfidenceFactor {
            name: "Test Coverage".to_string(),
            weight: self.weights.test_coverage,
            score: test_score,
            description: if test_files_touched > 0 {
                format!("{} test files modified", test_files_touched)
            } else {
                "No test files modified".to_string()
            },
            impact: if test_score > 0.7 { FactorImpact::Positive } else { FactorImpact::Negative },
        });

        // Calculate final score
        let total_score = validation_contribution +
                         verification_contribution +
                         risk_contribution +
                         issue_contribution +
                         size_contribution +
                         test_contribution;

        let explanation = self.generate_explanation(&factors, total_score);
        let recommendation = self.generate_recommendation(total_score, risk, issues);

        ConfidenceScore {
            score: total_score,
            factors,
            explanation,
            recommendation,
        }
    }

    fn generate_explanation(&self, factors: &[ConfidenceFactor], total_score: f32) -> String {
        let mut explanation = format!("Overall confidence score: {:.0}%\n\n", total_score * 100.0);
        
        explanation.push_str("Factor breakdown:\n");
        for factor in factors {
            let contribution = factor.score * factor.weight * 100.0;
            explanation.push_str(&format!(
                "  - {}: {:.0}% contribution ({:.0}% of {:.0}% weight)\n",
                factor.name, contribution, factor.score * 100.0, factor.weight * 100.0
            ));
        }

        explanation
    }

    fn generate_recommendation(&self, score: f32, risk: &RiskAssessment, issues: &[ReviewIssue]) -> Option<String> {
        if score >= self.thresholds.high_confidence {
            if risk.level <= RiskLevel::Low {
                Some("High confidence - safe to proceed".to_string())
            } else {
                Some("High confidence but monitor risk factors".to_string())
            }
        } else if score >= self.thresholds.medium_confidence {
            let has_issues = !issues.is_empty();
            if has_issues {
                Some("Medium confidence - review issues before proceeding".to_string())
            } else {
                Some("Medium confidence - consider additional validation".to_string())
            }
        } else if score >= self.thresholds.low_confidence {
            Some("Low confidence - requires additional review and testing".to_string())
        } else {
            Some("Very low confidence - significant improvements needed".to_string())
        }
    }

    pub fn classify(&self, score: f32) -> ConfidenceClassification {
        if score >= self.thresholds.high_confidence {
            ConfidenceClassification::High
        } else if score >= self.thresholds.medium_confidence {
            ConfidenceClassification::Medium
        } else if score >= self.thresholds.low_confidence {
            ConfidenceClassification::Low
        } else {
            ConfidenceClassification::VeryLow
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConfidenceClassification {
    VeryLow,
    Low,
    Medium,
    High,
}

pub fn compute_confidence(
    validation: Option<&ValidationResult>,
    issues: &[ReviewIssue],
    risk: &RiskAssessment,
    verification: VerificationStrength,
) -> ConfidenceScore {
    let calibrator = ConfidenceCalibrator::new();
    calibrator.compute(validation, issues, risk, verification, 100, 0)
}
