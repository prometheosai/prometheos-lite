use crate::harness::{
    review::{ReviewIssue, ReviewSeverity},
    semantic_diff::SemanticDiff,
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RiskAssessment {
    pub level: RiskLevel,
    pub reasons: Vec<String>,
    pub requires_approval: bool,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}
pub fn assess_risk(d: &SemanticDiff, i: &[ReviewIssue]) -> RiskAssessment {
    let level = if i.iter().any(|x| x.severity == ReviewSeverity::Critical) {
        RiskLevel::Critical
    } else if d.auth_changes || d.database_changes {
        RiskLevel::High
    } else {
        RiskLevel::Low
    };
    RiskAssessment {
        level,
        reasons: vec![],
        requires_approval: level >= RiskLevel::High,
    }
}
