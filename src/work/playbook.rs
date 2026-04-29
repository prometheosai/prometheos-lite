//! Playbook system for WorkContext (V1.2: store/retrieve only, no auto-evolution)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::types::ApprovalPolicy;

/// WorkContextPlaybook - a playbook for personalizing work execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkContextPlaybook {
    pub id: String,
    pub user_id: String,
    pub domain_profile_id: String,
    pub name: String,
    pub description: String,
    pub preferred_flows: Vec<FlowPreference>,
    pub preferred_nodes: Vec<NodePreference>,
    pub default_approval_policy: ApprovalPolicy,
    pub default_research_depth: ResearchDepth,
    pub default_creativity_level: CreativityLevel,
    pub evaluation_rules: Vec<String>,
    pub success_patterns: Vec<PatternRecord>,
    pub failure_patterns: Vec<PatternRecord>,
    pub confidence: f32,
    pub usage_count: u32,
    pub updated_at: DateTime<Utc>,
}

impl WorkContextPlaybook {
    /// Create a new playbook
    pub fn new(
        id: String,
        user_id: String,
        domain_profile_id: String,
        name: String,
        description: String,
    ) -> Self {
        Self {
            id,
            user_id,
            domain_profile_id,
            name,
            description,
            preferred_flows: Vec::new(),
            preferred_nodes: Vec::new(),
            default_approval_policy: ApprovalPolicy::Auto,
            default_research_depth: ResearchDepth::Standard,
            default_creativity_level: CreativityLevel::Balanced,
            evaluation_rules: Vec::new(),
            success_patterns: Vec::new(),
            failure_patterns: Vec::new(),
            confidence: 0.5,
            usage_count: 0,
            updated_at: Utc::now(),
        }
    }

    /// Increment usage count
    pub fn record_usage(&mut self) {
        self.usage_count += 1;
        self.updated_at = Utc::now();
    }

    /// Update confidence score
    pub fn update_confidence(&mut self, confidence: f32) {
        self.confidence = confidence.clamp(0.0, 1.0);
        self.updated_at = Utc::now();
    }
}

/// ResearchDepth - how deep to research
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResearchDepth {
    Minimal,
    Standard,
    Deep,
    Exhaustive,
}

/// CreativityLevel - how creative to be
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CreativityLevel {
    Conservative,
    Balanced,
    Creative,
}

/// PatternRecord - extracted success/failure patterns for learning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternRecord {
    pub pattern_type: PatternType,
    pub signal: String,
    pub weight: f32,
    pub created_at: DateTime<Utc>,
}

/// PatternType - classification of pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PatternType {
    Success,
    Failure,
}

/// FlowPreference - weighted flow selection preference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowPreference {
    pub flow_id: String,
    pub weight: f32,
    pub confidence: f32,
}

/// NodePreference - node-specific parameter preferences
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodePreference {
    pub node_type: String,
    pub params: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_playbook_creation() {
        let playbook = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "My Software Playbook".to_string(),
            "Personal preferences for software work".to_string(),
        );

        assert_eq!(playbook.id, "pb-1");
        assert_eq!(playbook.usage_count, 0);
        assert_eq!(playbook.confidence, 0.5);
    }

    #[test]
    fn test_record_usage() {
        let mut playbook = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "My Software Playbook".to_string(),
            "Personal preferences".to_string(),
        );

        playbook.record_usage();
        assert_eq!(playbook.usage_count, 1);

        playbook.record_usage();
        assert_eq!(playbook.usage_count, 2);
    }

    #[test]
    fn test_update_confidence() {
        let mut playbook = WorkContextPlaybook::new(
            "pb-1".to_string(),
            "user-1".to_string(),
            "software".to_string(),
            "My Software Playbook".to_string(),
            "Personal preferences".to_string(),
        );

        playbook.update_confidence(0.8);
        assert_eq!(playbook.confidence, 0.8);

        // Test clamping
        playbook.update_confidence(1.5);
        assert_eq!(playbook.confidence, 1.0);

        playbook.update_confidence(-0.5);
        assert_eq!(playbook.confidence, 0.0);
    }
}
