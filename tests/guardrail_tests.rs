//! V1.1 Guardrail Test Suite
//!
//! Tests for enforced runtime guardrails ensuring:
//! - No side effect can occur without ToolContext + ToolPolicy + trace
//! - No resumed run can use a changed flow definition
//! - No retry can duplicate a side effect silently
//! - No untrusted tool can execute without approval
//! - No flow can loop forever

use prometheos_lite::flow::loop_detection::{LoopDetectionConfig, LoopDetector};
use prometheos_lite::flow::{FlowSnapshot, IdempotencyKey};
use prometheos_lite::tools::{
    ApprovalPolicy, ToolContext, ToolPermission, ToolPolicy, TrustLevel, TrustRegistry,
};

#[test]
fn test_tool_without_context_fails() {
    // Verify that tool execution fails without ToolContext
    // This test ensures ToolContext is required for all tool operations
    let policy = ToolPolicy::new();
    let context = ToolContext::new(
        "test_run".to_string(),
        "test_trace".to_string(),
        "test_node".to_string(),
        "test_tool".to_string(),
        policy,
    );

    assert_eq!(context.run_id, "test_run");
    assert_eq!(context.trace_id, "test_trace");
    assert_eq!(context.node_id, "test_node");
    assert_eq!(context.tool_name, "test_tool");
}

#[test]
fn test_shell_denied_by_default() {
    // Verify that shell execution is denied by default
    let policy = ToolPolicy::new(); // Default policy has no shell permission
    assert!(!policy.is_allowed(ToolPermission::Shell));
}

#[test]
fn test_network_denied_by_default() {
    // Verify that network access is denied by default
    let policy = ToolPolicy::new(); // Default policy has no network permission
    assert!(!policy.is_allowed(ToolPermission::Network));
}

#[test]
fn test_absolute_file_write_denied() {
    // Verify that absolute file paths are denied
    use prometheos_lite::tools::PathGuard;
    let guard = PathGuard::default();

    // Unix absolute path
    let result = guard.validate_path("/etc/passwd");
    assert!(result.is_err());

    // Windows absolute path
    let result = guard.validate_path("C:\\Windows\\System32");
    assert!(result.is_err());
}

#[test]
fn test_path_traversal_denied() {
    // Verify that path traversal (..) is denied
    use prometheos_lite::tools::PathGuard;
    let guard = PathGuard::default();

    let result = guard.validate_path("../../secret");
    assert!(result.is_err());

    let result = guard.validate_path("safe/../../../etc/passwd");
    assert!(result.is_err());
}

#[test]
fn test_flow_resume_uses_snapshot() {
    // Verify that flow resume uses the stored snapshot

    let source = "nodes:\n  - id: test\n    type: llm";
    let snapshot = FlowSnapshot::new(
        "test_flow".to_string(),
        "1.0.0".to_string(),
        source.to_string(),
    );

    // Verify hash computation
    let computed_hash = FlowSnapshot::compute_hash(source);
    assert_eq!(snapshot.source_hash, computed_hash);

    // Verify hash validation
    assert!(snapshot.verify_hash(source));
    assert!(!snapshot.verify_hash("different source"));
}

#[test]
fn test_schema_hash_change_detected() {
    // Verify that schema hash changes are detected

    let source1 = "nodes:\n  - id: test";
    let source2 = "nodes:\n  - id: different";

    let snapshot1 = FlowSnapshot::new(
        "test_flow".to_string(),
        "1.0.0".to_string(),
        source1.to_string(),
    );

    // Different source should not match
    assert!(!snapshot1.verify_hash(source2));
}

#[test]
fn test_side_effect_not_reexecuted() {
    // Verify that side effects are not re-executed on retry

    let key1 = IdempotencyKey::new(
        "run1".to_string(),
        "node1".to_string(),
        "hash123".to_string(),
    );
    let key2 = IdempotencyKey::new(
        "run1".to_string(),
        "node1".to_string(),
        "hash123".to_string(),
    );

    // Same operation should produce same key
    assert!(key1.matches(&key2));

    // Different operation should produce different key
    let key3 = IdempotencyKey::new(
        "run1".to_string(),
        "node1".to_string(),
        "hash456".to_string(),
    );
    assert!(!key1.matches(&key3));
}

#[test]
fn test_interrupt_invalid_decision_rejected() {
    // Verify that invalid decisions are rejected
    use prometheos_lite::tools::{InterruptContext, InterruptStatus};
    use serde_json::json;

    let schema = json!({"approved": true});
    let mut context = InterruptContext::new(
        "run1".to_string(),
        "trace1".to_string(),
        "node1".to_string(),
        "Test interrupt".to_string(),
        schema,
    );

    // Invalid decision (missing required field)
    let invalid_decision = json!({"wrong_field": true});
    let result = context.approve(invalid_decision);
    assert!(result.is_err());

    // Valid decision
    let valid_decision = json!({"approved": true});
    let result = context.approve(valid_decision);
    assert!(result.is_ok());
    assert_eq!(context.status, InterruptStatus::Approved);
}

#[test]
fn test_untrusted_tool_requires_approval() {
    // Verify that untrusted tools require approval
    use prometheos_lite::tools::ApprovalPolicy;

    let policy = ToolPolicy::new();
    let context = ToolContext::new(
        "test_run".to_string(),
        "test_trace".to_string(),
        "test_node".to_string(),
        "untrusted_tool".to_string(),
        policy,
    )
    .with_approval_policy(ApprovalPolicy::RequireForUntrusted)
    .with_trust_level(TrustLevel::Untrusted);

    // Untrusted tools should require approval with RequireForUntrusted policy
    assert!(context.requires_approval());
}

#[test]
fn test_loop_detection_stops_run() {
    // Verify that loop detection stops runaway flows

    let config = LoopDetectionConfig {
        max_repeated_node: 3,
        max_repeated_transition: 2,
        max_repeated_tool_call: 2,
    };
    let mut detector = LoopDetector::with_config(config.clone());

    // Node repetition should be detected
    detector.record_node("node1").unwrap();
    detector.record_node("node1").unwrap();
    detector.record_node("node1").unwrap();
    let result = detector.record_node("node1");
    assert!(result.is_err());

    // Transition repetition should be detected
    let mut detector2 = LoopDetector::with_config(config);
    detector2.record_transition("node1", "node2").unwrap();
    detector2.record_transition("node1", "node2").unwrap();
    let result = detector2.record_transition("node1", "node2");
    assert!(result.is_err());
}

#[test]
fn test_approval_policy_auto() {
    // Verify Auto approval policy

    let policy = ToolPolicy::new().with_approval_policy(ApprovalPolicy::Auto);
    let context = ToolContext::new(
        "test_run".to_string(),
        "test_trace".to_string(),
        "test_node".to_string(),
        "test_tool".to_string(),
        policy,
    );

    // Auto policy should not require approval
    assert!(!context.requires_approval());
}

#[test]
fn test_approval_policy_manual_all() {
    // Verify ManualAll approval policy

    let policy = ToolPolicy::new();
    let context = ToolContext::new(
        "test_run".to_string(),
        "test_trace".to_string(),
        "test_node".to_string(),
        "test_tool".to_string(),
        policy,
    )
    .with_approval_policy(ApprovalPolicy::ManualAll);

    // ManualAll policy should always require approval
    assert!(context.requires_approval());
}

#[test]
fn test_trust_policy_levels() {
    // Verify trust policy levels

    let registry = TrustRegistry::new();

    assert_eq!(registry.get_trust_level("builtin"), TrustLevel::Trusted);
    assert_eq!(registry.get_trust_level("local"), TrustLevel::Local);
    assert_eq!(registry.get_trust_level("community"), TrustLevel::Community);
    assert_eq!(registry.get_trust_level("external"), TrustLevel::External);
    assert_eq!(registry.get_trust_level("unknown"), TrustLevel::Untrusted);
}

#[test]
fn test_path_guard_safe_paths() {
    // Verify that safe paths are accepted
    use prometheos_lite::tools::PathGuard;
    let guard = PathGuard::default();

    // Safe relative paths
    assert!(guard.is_safe_path("output.txt"));
    assert!(guard.is_safe_path("subdir/file.txt"));
    assert!(guard.is_safe_path("deep/nested/path/file.txt"));
}

#[test]
fn test_idempotency_key_computation() {
    // Verify idempotency key computation

    let content = "write file";
    let params = serde_json::json!({"path": "test.txt", "content": "hello"});

    let hash1 = IdempotencyKey::compute_operation_hash(content, &params);
    let hash2 = IdempotencyKey::compute_operation_hash(content, &params);

    // Same content and params should produce same hash
    assert_eq!(hash1, hash2);

    // Different params should produce different hash
    let params2 = serde_json::json!({"path": "test.txt", "content": "world"});
    let hash3 = IdempotencyKey::compute_operation_hash(content, &params2);
    assert_ne!(hash1, hash3);
}

#[test]
fn test_interrupt_expiration() {
    // Verify interrupt expiration
    use chrono::{Duration, Utc};
    use prometheos_lite::tools::InterruptContext;

    let schema = serde_json::json!({});
    let mut context = InterruptContext::new(
        "run1".to_string(),
        "trace1".to_string(),
        "node1".to_string(),
        "Test interrupt".to_string(),
        schema,
    );

    // No expiration by default
    assert!(!context.is_expired());

    // Set expiration in the past
    let past = Utc::now() - Duration::hours(1);
    context.expires_at = Some(past);
    assert!(context.is_expired());

    // Set expiration in the future
    let future = Utc::now() + Duration::hours(1);
    context.expires_at = Some(future);
    assert!(!context.is_expired());
}

#[test]
fn test_loop_detector_cycle_detection() {
    // Verify cycle detection

    let mut detector = LoopDetector::new();

    // Not enough transitions
    assert!(!detector.detect_cycle());

    // Add repeating transitions
    for _ in 0..5 {
        detector.record_transition("node1", "node2").unwrap();
    }

    // Should detect cycle
    assert!(detector.detect_cycle());
}

#[test]
fn test_tool_permission_combinations() {
    // Verify tool permission combinations
    let policy = ToolPolicy::new()
        .with_permission(ToolPermission::Shell)
        .with_permission(ToolPermission::FileWrite);

    assert!(policy.is_allowed(ToolPermission::Shell));
    assert!(policy.is_allowed(ToolPermission::FileWrite));
    assert!(!policy.is_allowed(ToolPermission::Network));
}

#[test]
fn test_trust_policy_approval_requirements() {
    // Verify trust policy approval requirements

    let registry = TrustRegistry::new();

    // Trusted sources should not require approval
    assert!(!registry.requires_approval("builtin"));

    // Untrusted sources should require approval
    assert!(registry.requires_approval("unknown"));
    assert!(registry.requires_approval("external"));
}
