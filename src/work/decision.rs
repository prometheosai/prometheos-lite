//! Decision system for WorkContext

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// DecisionRecord - a record of a decision made during work
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub id: String,
    pub description: String,
    pub chosen_option: String,
    pub alternatives: Vec<String>,
    pub approved: bool,
    pub created_at: DateTime<Utc>,
}

impl DecisionRecord {
    /// Create a new decision record
    pub fn new(
        id: String,
        description: String,
        chosen_option: String,
        alternatives: Vec<String>,
    ) -> Self {
        Self {
            id,
            description,
            chosen_option,
            alternatives,
            approved: false,
            created_at: Utc::now(),
        }
    }

    /// Approve the decision
    pub fn approve(&mut self) {
        self.approved = true;
    }

    /// Reject the decision
    pub fn reject(&mut self) {
        self.approved = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decision_creation() {
        let decision = DecisionRecord::new(
            "dec-1".to_string(),
            "Choose database".to_string(),
            "PostgreSQL".to_string(),
            vec![
                "PostgreSQL".to_string(),
                "MySQL".to_string(),
                "SQLite".to_string(),
            ],
        );

        assert_eq!(decision.id, "dec-1");
        assert_eq!(decision.chosen_option, "PostgreSQL");
        assert!(!decision.approved);
    }

    #[test]
    fn test_decision_approve() {
        let mut decision = DecisionRecord::new(
            "dec-1".to_string(),
            "Choose database".to_string(),
            "PostgreSQL".to_string(),
            vec!["PostgreSQL".to_string()],
        );

        decision.approve();
        assert!(decision.approved);
    }

    #[test]
    fn test_decision_reject() {
        let mut decision = DecisionRecord::new(
            "dec-1".to_string(),
            "Choose database".to_string(),
            "PostgreSQL".to_string(),
            vec!["PostgreSQL".to_string()],
        );

        decision.approve();
        decision.reject();
        assert!(!decision.approved);
    }
}
