//! Integration tests for WorkContextPlaybook functionality
//!
//! This test file validates:
//! - Playbook creation with patterns and preferences
//! - FlowPreference weights
//! - PatternRecord storage
//! - WorkContextPlaybook CRUD operations

use chrono::Utc;
use prometheos_lite::work::playbook::{
    CreativityLevel, FlowPreference, NodePreference, PatternRecord, PatternType, ResearchDepth,
    WorkContextPlaybook,
};
use prometheos_lite::work::types::ApprovalPolicy;

#[test]
fn test_playbook_creation_with_patterns_and_preferences() {
    let playbook = WorkContextPlaybook::new(
        "test-playbook-1".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Software Development Playbook".to_string(),
        "Optimized for software development tasks".to_string(),
    );

    assert_eq!(playbook.id, "test-playbook-1");
    assert_eq!(playbook.user_id, "test-user");
    assert_eq!(playbook.domain_profile_id, "domain-1");
    assert_eq!(playbook.name, "Software Development Playbook");
    assert_eq!(
        playbook.description,
        "Optimized for software development tasks"
    );
    assert_eq!(playbook.confidence, 0.5); // Default confidence
    assert_eq!(playbook.usage_count, 0); // Default usage count
}

#[test]
fn test_playbook_with_flow_preferences() {
    let flow_preferences = vec![
        FlowPreference {
            flow_id: "planning.flow.yaml".to_string(),
            weight: 0.8,
            confidence: 0.9,
        },
        FlowPreference {
            flow_id: "coding.flow.yaml".to_string(),
            weight: 0.6,
            confidence: 0.7,
        },
    ];

    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-2".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Weighted Flow Playbook".to_string(),
        "Uses weighted flow selection".to_string(),
    );

    playbook.preferred_flows = flow_preferences.clone();

    assert_eq!(playbook.preferred_flows.len(), 2);
    assert_eq!(playbook.preferred_flows[0].flow_id, "planning.flow.yaml");
    assert_eq!(playbook.preferred_flows[0].weight, 0.8);
    assert_eq!(playbook.preferred_flows[0].confidence, 0.9);
    assert_eq!(playbook.preferred_flows[1].flow_id, "coding.flow.yaml");
    assert_eq!(playbook.preferred_flows[1].weight, 0.6);
}

#[test]
fn test_playbook_with_node_preferences() {
    let node_preferences = vec![
        NodePreference {
            node_type: "planner".to_string(),
            params: serde_json::json!({"temperature": 0.7}),
        },
        NodePreference {
            node_type: "coder".to_string(),
            params: serde_json::json!({"temperature": 0.5}),
        },
    ];

    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-3".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Node-Weighted Playbook".to_string(),
        "Uses weighted node selection".to_string(),
    );

    playbook.preferred_nodes = node_preferences.clone();

    assert_eq!(playbook.preferred_nodes.len(), 2);
    assert_eq!(playbook.preferred_nodes[0].node_type, "planner");
    assert_eq!(playbook.preferred_nodes[1].node_type, "coder");
}

#[test]
fn test_playbook_with_success_patterns() {
    let success_patterns = vec![
        PatternRecord {
            pattern_type: PatternType::Success,
            signal: "Quick completion".to_string(),
            weight: 0.8,
            created_at: Utc::now(),
        },
        PatternRecord {
            pattern_type: PatternType::Success,
            signal: "High quality output".to_string(),
            weight: 0.9,
            created_at: Utc::now(),
        },
    ];

    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-4".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Success Pattern Playbook".to_string(),
        "Tracks successful execution patterns".to_string(),
    );

    playbook.success_patterns = success_patterns.clone();

    assert_eq!(playbook.success_patterns.len(), 2);
    assert_eq!(
        playbook.success_patterns[0].pattern_type,
        PatternType::Success
    );
    assert_eq!(playbook.success_patterns[0].signal, "Quick completion");
    assert_eq!(playbook.success_patterns[0].weight, 0.8);
}

#[test]
fn test_playbook_with_failure_patterns() {
    let failure_patterns = vec![
        PatternRecord {
            pattern_type: PatternType::Failure,
            signal: "Timeout on large inputs".to_string(),
            weight: 0.7,
            created_at: Utc::now(),
        },
        PatternRecord {
            pattern_type: PatternType::Failure,
            signal: "API rate limit".to_string(),
            weight: 0.6,
            created_at: Utc::now(),
        },
    ];

    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-5".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Failure Pattern Playbook".to_string(),
        "Tracks failure patterns to avoid".to_string(),
    );

    playbook.failure_patterns = failure_patterns.clone();

    assert_eq!(playbook.failure_patterns.len(), 2);
    assert_eq!(
        playbook.failure_patterns[0].pattern_type,
        PatternType::Failure
    );
    assert_eq!(
        playbook.failure_patterns[0].signal,
        "Timeout on large inputs"
    );
    assert_eq!(playbook.failure_patterns[0].weight, 0.7);
}

#[test]
fn test_playbook_serialization() {
    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-6".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Serialization Test Playbook".to_string(),
        "Tests serialization".to_string(),
    );

    playbook.preferred_flows = vec![FlowPreference {
        flow_id: "test.flow.yaml".to_string(),
        weight: 0.5,
        confidence: 0.8,
    }];

    playbook.success_patterns = vec![PatternRecord {
        pattern_type: PatternType::Success,
        signal: "Test pattern".to_string(),
        weight: 0.9,
        created_at: Utc::now(),
    }];

    // Test serialization
    let json = serde_json::to_string(&playbook).expect("Failed to serialize playbook");
    assert!(!json.is_empty());

    // Test deserialization
    let deserialized: WorkContextPlaybook =
        serde_json::from_str(&json).expect("Failed to deserialize playbook");

    assert_eq!(deserialized.id, playbook.id);
    assert_eq!(deserialized.name, playbook.name);
    assert_eq!(
        deserialized.preferred_flows.len(),
        playbook.preferred_flows.len()
    );
    assert_eq!(
        deserialized.success_patterns.len(),
        playbook.success_patterns.len()
    );
}

#[test]
fn test_pattern_record_creation() {
    let pattern = PatternRecord {
        pattern_type: PatternType::Success,
        signal: "Fast execution".to_string(),
        weight: 0.9,
        created_at: Utc::now(),
    };

    assert_eq!(pattern.pattern_type, PatternType::Success);
    assert_eq!(pattern.signal, "Fast execution");
    assert_eq!(pattern.weight, 0.9);
}

#[test]
fn test_pattern_record_serialization() {
    let pattern = PatternRecord {
        pattern_type: PatternType::Failure,
        signal: "Error occurred".to_string(),
        weight: 0.7,
        created_at: Utc::now(),
    };

    // Test serialization
    let json = serde_json::to_string(&pattern).expect("Failed to serialize pattern");
    assert!(!json.is_empty());

    // Test deserialization
    let deserialized: PatternRecord =
        serde_json::from_str(&json).expect("Failed to deserialize pattern");

    assert_eq!(deserialized.pattern_type, PatternType::Failure);
    assert_eq!(deserialized.signal, "Error occurred");
    assert_eq!(deserialized.weight, 0.7);
}

#[test]
fn test_flow_preference_creation() {
    let preference = FlowPreference {
        flow_id: "test.flow.yaml".to_string(),
        weight: 0.75,
        confidence: 0.85,
    };

    assert_eq!(preference.flow_id, "test.flow.yaml");
    assert_eq!(preference.weight, 0.75);
    assert_eq!(preference.confidence, 0.85);
}

#[test]
fn test_flow_preference_weight_bounds() {
    // Test weight within valid range
    let preference = FlowPreference {
        flow_id: "test.flow.yaml".to_string(),
        weight: 0.5,
        confidence: 0.8,
    };
    assert!(preference.weight >= 0.0 && preference.weight <= 1.0);
}

#[test]
fn test_node_preference_creation() {
    let preference = NodePreference {
        node_type: "planner".to_string(),
        params: serde_json::json!({"temperature": 0.7}),
    };

    assert_eq!(preference.node_type, "planner");
}

#[test]
fn test_playbook_with_all_fields() {
    let flow_preferences = vec![FlowPreference {
        flow_id: "planning.flow.yaml".to_string(),
        weight: 0.8,
        confidence: 0.9,
    }];

    let node_preferences = vec![NodePreference {
        node_type: "planner".to_string(),
        params: serde_json::json!({"temperature": 0.7}),
    }];

    let success_patterns = vec![PatternRecord {
        pattern_type: PatternType::Success,
        signal: "Quick completion".to_string(),
        weight: 0.8,
        created_at: Utc::now(),
    }];

    let failure_patterns = vec![PatternRecord {
        pattern_type: PatternType::Failure,
        signal: "Timeout".to_string(),
        weight: 0.7,
        created_at: Utc::now(),
    }];

    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-7".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Complete Playbook".to_string(),
        "Playbook with all fields populated".to_string(),
    );

    playbook.preferred_flows = flow_preferences;
    playbook.preferred_nodes = node_preferences;
    playbook.success_patterns = success_patterns;
    playbook.failure_patterns = failure_patterns;
    playbook.confidence = 0.85;
    playbook.usage_count = 10;
    playbook.default_approval_policy = ApprovalPolicy::RequireForSideEffects;
    playbook.default_research_depth = ResearchDepth::Standard;
    playbook.default_creativity_level = CreativityLevel::Balanced;

    assert_eq!(playbook.preferred_flows.len(), 1);
    assert_eq!(playbook.preferred_nodes.len(), 1);
    assert_eq!(playbook.success_patterns.len(), 1);
    assert_eq!(playbook.failure_patterns.len(), 1);
    assert_eq!(playbook.confidence, 0.85);
    assert_eq!(playbook.usage_count, 10);
}

#[test]
fn test_playbook_increment_usage_count() {
    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-8".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Usage Count Playbook".to_string(),
        "Tests usage count increment".to_string(),
    );

    assert_eq!(playbook.usage_count, 0);

    playbook.usage_count += 1;
    assert_eq!(playbook.usage_count, 1);

    playbook.usage_count += 1;
    assert_eq!(playbook.usage_count, 2);
}

#[test]
fn test_playbook_confidence_bounds() {
    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-9".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Confidence Playbook".to_string(),
        "Tests confidence bounds".to_string(),
    );

    // Test minimum confidence
    playbook.confidence = 0.0;
    assert_eq!(playbook.confidence, 0.0);

    // Test maximum confidence
    playbook.confidence = 1.0;
    assert_eq!(playbook.confidence, 1.0);

    // Test mid-range confidence
    playbook.confidence = 0.5;
    assert_eq!(playbook.confidence, 0.5);
}

#[test]
fn test_playbook_updated_at() {
    let playbook = WorkContextPlaybook::new(
        "test-playbook-10".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Timestamp Playbook".to_string(),
        "Tests timestamp tracking".to_string(),
    );

    // Verify updated_at is set
    assert!(playbook.updated_at <= Utc::now());
}

#[test]
fn test_playbook_evaluation_rules() {
    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-11".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Evaluation Rules Playbook".to_string(),
        "Tests evaluation rules".to_string(),
    );

    playbook.evaluation_rules = vec![
        "artifact_completeness".to_string(),
        "latency_check".to_string(),
    ];

    assert_eq!(playbook.evaluation_rules.len(), 2);
    assert_eq!(playbook.evaluation_rules[0], "artifact_completeness");
    assert_eq!(playbook.evaluation_rules[1], "latency_check");
}

#[test]
fn test_playbook_domain_profile_id() {
    let playbook = WorkContextPlaybook::new(
        "test-playbook-12".to_string(),
        "test-user".to_string(),
        "software-domain".to_string(),
        "Domain Profile Playbook".to_string(),
        "Tests domain profile association".to_string(),
    );

    assert_eq!(playbook.domain_profile_id, "software-domain");
}

#[test]
fn test_pattern_type_variants() {
    let success_pattern = PatternRecord {
        pattern_type: PatternType::Success,
        signal: "Success".to_string(),
        weight: 0.9,
        created_at: Utc::now(),
    };

    let failure_pattern = PatternRecord {
        pattern_type: PatternType::Failure,
        signal: "Failure".to_string(),
        weight: 0.7,
        created_at: Utc::now(),
    };

    assert_eq!(success_pattern.pattern_type, PatternType::Success);
    assert_eq!(failure_pattern.pattern_type, PatternType::Failure);
}

#[test]
fn test_pattern_type_serialization() {
    // Test that PatternType can be serialized and deserialized
    let pattern = PatternRecord {
        pattern_type: PatternType::Success,
        signal: "Test".to_string(),
        weight: 0.9,
        created_at: Utc::now(),
    };

    let json = serde_json::to_string(&pattern).expect("Failed to serialize");
    let deserialized: PatternRecord = serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.pattern_type, PatternType::Success);
}

#[test]
fn test_playbook_record_usage() {
    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-13".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Record Usage Playbook".to_string(),
        "Tests record_usage method".to_string(),
    );

    assert_eq!(playbook.usage_count, 0);

    playbook.record_usage();
    assert_eq!(playbook.usage_count, 1);

    playbook.record_usage();
    assert_eq!(playbook.usage_count, 2);
}

#[test]
fn test_playbook_update_confidence() {
    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-14".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Update Confidence Playbook".to_string(),
        "Tests update_confidence method".to_string(),
    );

    playbook.update_confidence(0.8);
    assert_eq!(playbook.confidence, 0.8);

    // Test clamping
    playbook.update_confidence(1.5);
    assert_eq!(playbook.confidence, 1.0);

    playbook.update_confidence(-0.5);
    assert_eq!(playbook.confidence, 0.0);
}

#[test]
fn test_research_depth_variants() {
    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-15".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Research Depth Playbook".to_string(),
        "Tests research depth variants".to_string(),
    );

    playbook.default_research_depth = ResearchDepth::Minimal;
    assert_eq!(playbook.default_research_depth, ResearchDepth::Minimal);

    playbook.default_research_depth = ResearchDepth::Standard;
    assert_eq!(playbook.default_research_depth, ResearchDepth::Standard);

    playbook.default_research_depth = ResearchDepth::Deep;
    assert_eq!(playbook.default_research_depth, ResearchDepth::Deep);

    playbook.default_research_depth = ResearchDepth::Exhaustive;
    assert_eq!(playbook.default_research_depth, ResearchDepth::Exhaustive);
}

#[test]
fn test_creativity_level_variants() {
    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-16".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Creativity Level Playbook".to_string(),
        "Tests creativity level variants".to_string(),
    );

    playbook.default_creativity_level = CreativityLevel::Conservative;
    assert_eq!(
        playbook.default_creativity_level,
        CreativityLevel::Conservative
    );

    playbook.default_creativity_level = CreativityLevel::Balanced;
    assert_eq!(playbook.default_creativity_level, CreativityLevel::Balanced);

    playbook.default_creativity_level = CreativityLevel::Creative;
    assert_eq!(playbook.default_creativity_level, CreativityLevel::Creative);
}

#[test]
fn test_approval_policy_variants() {
    let mut playbook = WorkContextPlaybook::new(
        "test-playbook-17".to_string(),
        "test-user".to_string(),
        "domain-1".to_string(),
        "Approval Policy Playbook".to_string(),
        "Tests approval policy variants".to_string(),
    );

    playbook.default_approval_policy = ApprovalPolicy::Auto;
    assert_eq!(playbook.default_approval_policy, ApprovalPolicy::Auto);

    playbook.default_approval_policy = ApprovalPolicy::ManualAll;
    assert_eq!(playbook.default_approval_policy, ApprovalPolicy::ManualAll);

    playbook.default_approval_policy = ApprovalPolicy::RequireForSideEffects;
    assert_eq!(
        playbook.default_approval_policy,
        ApprovalPolicy::RequireForSideEffects
    );
}
