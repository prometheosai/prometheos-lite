use crate::harness::{
    review::ReviewIssue, risk::RiskAssessment, validation::ValidationResult,
    verification::VerificationStrength,
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfidenceScore {
    pub score: f32,
    pub factors: Vec<String>,
}
pub fn compute_confidence(
    v: Option<&ValidationResult>,
    _: &[ReviewIssue],
    r: &RiskAssessment,
    _: VerificationStrength,
) -> ConfidenceScore {
    let mut s = 0.4;
    if v.is_some_and(|x| x.passed) {
        s += 0.4
    }
    if !r.requires_approval {
        s += 0.2
    }
    ConfidenceScore {
        score: s,
        factors: vec![],
    }
}
