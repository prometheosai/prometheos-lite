use crate::harness::{
    confidence::ConfidenceScore, execution_loop::HarnessMode, verification::VerificationStrength,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionEvidence {
    pub patch_exists: bool,
    pub validation_ran: bool,
    pub validation_passed: bool,
    pub review_ran: bool,
    pub critical_issues: usize,
    pub confidence: ConfidenceScore,
    pub verification_strength: VerificationStrength,
    pub requires_approval: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompletionDecision {
    Complete,
    Blocked(String),
    NeedsRepair(String),
    NeedsApproval(String),
}
pub fn evaluate_completion(e: &CompletionEvidence, _: HarnessMode) -> Result<CompletionDecision> {
    if !e.patch_exists {
        Ok(CompletionDecision::Blocked("no patch".into()))
    } else if !e.validation_passed {
        Ok(CompletionDecision::NeedsRepair("validation failed".into()))
    } else if e.requires_approval {
        Ok(CompletionDecision::NeedsApproval(
            "approval required".into(),
        ))
    } else {
        Ok(CompletionDecision::Complete)
    }
}
