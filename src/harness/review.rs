use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewIssue {
    pub issue_type: ReviewIssueType,
    pub severity: ReviewSeverity,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub message: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReviewIssueType {
    Bug,
    Security,
    Performance,
    Maintainability,
    TestGap,
    Style,
    Documentation,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReviewSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}
pub fn review_diff(diff: &str) -> Vec<ReviewIssue> {
    if diff.to_lowercase().contains("secret") {
        vec![ReviewIssue {
            issue_type: ReviewIssueType::Security,
            severity: ReviewSeverity::Critical,
            file: None,
            line: None,
            message: "secret marker".into(),
        }]
    } else {
        vec![]
    }
}
