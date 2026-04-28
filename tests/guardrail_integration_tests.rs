//! V1.1 Guardrail Integration Tests
//!
//! Integration tests covering resume/interrupt/outbox scenarios.
//! These tests ensure guardrails work end-to-end.

use prometheos_lite::tools::{InterruptContext, InterruptStatus, ToolContext, ToolPolicy, TrustLevel};
use prometheos_lite::flow::{FlowSnapshot, IdempotencyKey};
use serde_json::json;

#[test]
fn test_interrupt_lifecycle() {
    // Test full interrupt lifecycle: create, approve, resume
    let schema = json!({"approved": true, "reason": "User approved"});
    let mut context = InterruptContext::new(
        "run_123".to_string(),
        "trace_456".to_string(),
        "node_789".to_string(),
        "Tool execution requires approval".to_string(),
        schema.clone(),
    );

    // Initial state
    assert_eq!(context.status, InterruptStatus::Pending);
    assert!(context.decision.is_none());

    // Approve with valid decision
    let decision = json!({"approved": true, "reason": "User approved"});
    let result = context.approve(decision);
    assert!(result.is_ok());
    assert_eq!(context.status, InterruptStatus::Approved);
    assert!(context.decision.is_some());

    // Cannot approve again
    let result = context.approve(json!({"approved": false}));
    assert!(result.is_err());

    // Can deny
    let mut context2 = InterruptContext::new(
        "run_123".to_string(),
        "trace_456".to_string(),
        "node_789".to_string(),
        "Tool execution requires approval".to_string(),
        schema,
    );
    let result = context2.deny();
    assert!(result.is_ok());
    assert_eq!(context2.status, InterruptStatus::Denied);
}

#[test]
fn test_flow_snapshot_and_resume() {
    // Test flow snapshot creation and validation for resume
    let source = r#"
nodes:
  - id: planner
    type: llm
    model: gpt-4
  - id: executor
    type: tool
    tool: file_writer
"#;

    let snapshot = FlowSnapshot::new(
        "codegen_flow".to_string(),
        "1.0.0".to_string(),
        source.to_string(),
    );

    // Verify snapshot properties
    assert_eq!(snapshot.flow_name, "codegen_flow");
    assert_eq!(snapshot.flow_version, "1.0.0");
    assert_eq!(snapshot.source_text, source);
    assert!(!snapshot.source_hash.is_empty());

    // Simulate resume with same source
    let resume_valid = snapshot.verify_hash(source);
    assert!(resume_valid);

    // Simulate resume with changed source
    let changed_source = r#"
nodes:
  - id: planner
    type: llm
    model: gpt-4
  - id: executor
    type: tool
    tool: network_tool
"#;
    let resume_invalid = snapshot.verify_hash(changed_source);
    assert!(!resume_invalid);
}

#[test]
fn test_outbox_idempotency() {
    // Test outbox pattern for preventing duplicate side effects

    let run_id = "run_123".to_string();
    let node_id = "node_456".to_string();
    let tool_name = "file_writer".to_string();
    let input = json!({"path": "output.txt", "content": "hello"});

    // Generate idempotency key
    let operation_hash = IdempotencyKey::compute_operation_hash(
        &tool_name,
        &input,
    );
    let key = IdempotencyKey::new(
        run_id.clone(),
        node_id.clone(),
        operation_hash.clone(),
    );

    // First execution - should proceed
    let first_key = key.clone();
    assert_eq!(first_key.key, key.key);

    // Retry with same input - should detect duplicate
    let second_key = IdempotencyKey::new(
        run_id.clone(),
        node_id.clone(),
        operation_hash,
    );
    assert!(first_key.matches(&second_key));

    // Different input - should proceed
    let different_input = json!({"path": "output.txt", "content": "world"});
    let different_hash = IdempotencyKey::compute_operation_hash(
        &tool_name,
        &different_input,
    );
    let different_key = IdempotencyKey::new(
        run_id,
        node_id,
        different_hash,
    );
    assert!(!first_key.matches(&different_key));
}

#[test]
fn test_guardrail_chain() {
    // Test full guardrail chain: trust -> approval -> idempotency
    let policy = ToolPolicy::new();
    let context = ToolContext::new(
        "run_123".to_string(),
        "trace_456".to_string(),
        "node_789".to_string(),
        "file_writer".to_string(),
        policy.clone(),
    );

    // Check trust level
    assert_eq!(context.trust_level, TrustLevel::Local);

    // Check approval requirement
    let untrusted_context = context.clone().with_trust_level(TrustLevel::Untrusted);
    assert!(!untrusted_context.requires_approval()); // Auto policy by default

    // Generate idempotency key for the operation
    let operation_hash = IdempotencyKey::compute_operation_hash(
        "file_writer",
        &json!({"path": "test.txt"}),
    );
    let key = IdempotencyKey::new(
        context.run_id.clone(),
        context.node_id.clone(),
        operation_hash,
    );

    // Verify key is unique
    assert!(!key.key.is_empty());
}

#[test]
fn test_path_guard_integration() {
    // Test PathGuard integration with file operations
    use prometheos_lite::tools::PathGuard;

    let guard = PathGuard::default();

    // Safe paths
    let safe_paths = vec![
        "output.txt",
        "subdir/file.txt",
        "deep/nested/path/file.txt",
    ];

    for path in safe_paths {
        assert!(guard.is_safe_path(path), "Path should be safe: {}", path);
    }

    // Unsafe paths
    let unsafe_paths = vec![
        "/etc/passwd",
        "C:\\Windows\\System32",
        "../../secret",
        "safe/../../../etc/passwd",
    ];

    for path in unsafe_paths {
        assert!(!guard.is_safe_path(path), "Path should be unsafe: {}", path);
    }
}

#[test]
fn test_loop_detection_integration() {
    // Test loop detection in a realistic scenario
    use prometheos_lite::flow::loop_detection::{LoopDetectionConfig, LoopDetector};

    let config = LoopDetectionConfig {
        max_repeated_node: 5,
        max_repeated_transition: 5,
        max_repeated_tool_call: 2,
    };
    let mut detector = LoopDetector::with_config(config);

    // Simulate a flow with repeated nodes
    for _i in 0..2 {
        detector.record_node("planner").unwrap();
        detector.record_transition("planner", "executor").unwrap();
        detector.record_node("executor").unwrap();
        detector.record_transition("executor", "planner").unwrap();
    }

    // Should not trigger yet (not enough repetitions)
    assert!(!detector.detect_cycle());

    // Add more repetitions to trigger cycle detection
    for _i in 0..3 {
        detector.record_transition("planner", "executor").unwrap();
        detector.record_transition("executor", "planner").unwrap();
    }

    // Should detect cycle now
    assert!(detector.detect_cycle());
}

#[test]
fn test_tool_context_with_all_fields() {
    // Test ToolContext with all fields populated
    let policy = ToolPolicy::new()
        .with_permission(prometheos_lite::tools::ToolPermission::FileWrite);

    let context = ToolContext::new(
        "run_abc123".to_string(),
        "trace_def456".to_string(),
        "node_ghi789".to_string(),
        "file_writer".to_string(),
        policy,
    )
    .with_trust_level(prometheos_lite::tools::TrustLevel::Local)
    .with_approval_policy(prometheos_lite::tools::ApprovalPolicy::Auto)
    .with_idempotency_key("key_xyz".to_string());

    assert_eq!(context.run_id, "run_abc123");
    assert_eq!(context.trace_id, "trace_def456");
    assert_eq!(context.node_id, "node_ghi789");
    assert_eq!(context.tool_name, "file_writer");
    assert_eq!(context.trust_level, prometheos_lite::tools::TrustLevel::Local);
    assert_eq!(
        context.approval_policy,
        prometheos_lite::tools::ApprovalPolicy::Auto
    );
    assert_eq!(context.idempotency_key, Some("key_xyz".to_string()));
    assert!(!context.requires_approval());
}

#[test]
fn test_interrupt_with_expiration() {
    // Test interrupt with expiration time
    use chrono::{Duration, Utc};

    let schema = json!({});
    let mut context = InterruptContext::new(
        "run_123".to_string(),
        "trace_456".to_string(),
        "node_789".to_string(),
        "Test interrupt".to_string(),
        schema,
    );

    // Set expiration to 1 hour from now
    let expires_at = Utc::now() + Duration::hours(1);
    context = context.with_expiration(expires_at);

    assert!(!context.is_expired());

    // Set expiration to 1 hour ago
    let expires_at = Utc::now() - Duration::hours(1);
    context = context.with_expiration(expires_at);

    assert!(context.is_expired());
}

#[test]
fn test_multiple_interrupts_same_run() {
    // Test handling multiple interrupts in the same run
    let schema = json!({"approved": true});

    let interrupt1 = InterruptContext::new(
        "run_123".to_string(),
        "trace_1".to_string(),
        "node_1".to_string(),
        "First interrupt".to_string(),
        schema.clone(),
    );

    let interrupt2 = InterruptContext::new(
        "run_123".to_string(),
        "trace_2".to_string(),
        "node_2".to_string(),
        "Second interrupt".to_string(),
        schema,
    );

    // Each interrupt should have unique ID
    assert_ne!(interrupt1.interrupt_id, interrupt2.interrupt_id);

    // Both should be from same run
    assert_eq!(interrupt1.run_id, interrupt2.run_id);
}

#[test]
fn test_denied_write_via_path_guard() {
    // Test that PathGuard actually prevents unsafe file writes
    use prometheos_lite::tools::PathGuard;

    let guard = PathGuard::default();

    // Attempt to validate an absolute path - should fail
    let result = guard.validate_path("/etc/passwd");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Absolute paths not allowed"));

    // Attempt to validate a path with traversal - should fail
    let result = guard.validate_path("../../secret");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Parent directory traversal"));

    // Safe path check (quick check without filesystem)
    assert!(guard.is_safe_path("safe/output.txt"));
}

#[test]
fn test_denied_shell_via_tool_policy() {
    // Test that ToolPolicy prevents shell execution when not allowed
    use prometheos_lite::tools::{ToolContext, ToolPolicy, ToolPermission};

    // Create a policy that does NOT allow shell execution
    let policy = ToolPolicy::new(); // Conservative policy by default

    let context = ToolContext::new(
        "run_123".to_string(),
        "trace_456".to_string(),
        "node_789".to_string(),
        "shell".to_string(),
        policy,
    );

    // Shell permission should be denied
    assert!(!context.policy.is_allowed(ToolPermission::Shell));
}

#[test]
fn test_duplicate_side_effect_prevention() {
    // Test that idempotency keys prevent duplicate side effects
    use prometheos_lite::flow::IdempotencyKey;

    let run_id = "run_123".to_string();
    let node_id = "node_456".to_string();
    let tool_name = "file_writer".to_string();
    let input = json!({"path": "output.txt", "content": "hello"});

    // Generate idempotency key for first execution
    let operation_hash = IdempotencyKey::compute_operation_hash(&tool_name, &input);
    let key1 = IdempotencyKey::new(run_id.clone(), node_id.clone(), operation_hash.clone());

    // Same operation should generate same key
    let key2 = IdempotencyKey::new(run_id.clone(), node_id.clone(), operation_hash);
    assert_eq!(key1.key, key2.key);

    // Different operation should generate different key
    let different_input = json!({"path": "output.txt", "content": "world"});
    let different_hash = IdempotencyKey::compute_operation_hash(&tool_name, &different_input);
    let key3 = IdempotencyKey::new(run_id, node_id, different_hash);
    assert_ne!(key1.key, key3.key);
}

#[test]
fn test_loop_stopping_via_detector() {
    // Test that LoopDetector actually stops runaway flows
    use prometheos_lite::flow::loop_detection::{LoopDetectionConfig, LoopDetector};

    let config = LoopDetectionConfig {
        max_repeated_node: 3,
        max_repeated_transition: 3,
        max_repeated_tool_call: 2,
    };
    let mut detector = LoopDetector::with_config(config);

    // Record node repetitions up to limit
    detector.record_node("planner").unwrap();
    detector.record_node("planner").unwrap();
    detector.record_node("planner").unwrap();

    // Next repetition should fail
    let result = detector.record_node("planner");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("repeated 4 times (max: 3)"));
}

#[test]
fn test_resume_from_snapshot_hash_validation() {
    // Test that resume validates flow source hash
    use prometheos_lite::flow::FlowSnapshot;

    let original_source = r#"
nodes:
  - id: planner
    type: llm
  - id: executor
    type: tool
"#;

    let snapshot = FlowSnapshot::new(
        "test_flow".to_string(),
        "1.0.0".to_string(),
        original_source.to_string(),
    );

    // Resume with matching source should succeed
    assert!(snapshot.verify_hash(original_source));

    // Resume with modified source should fail
    let modified_source = r#"
nodes:
  - id: planner
    type: llm
  - id: executor
    type: tool
  - id: new_node
    type: llm
"#;
    assert!(!snapshot.verify_hash(modified_source));
}

#[test]
fn test_trust_policy_persistence() {
    // Test that trust policies can be persisted and retrieved
    use std::fs;
    
    // Create a temporary database
    let db_path = ".prometheos/test_trust.db";
    let _ = fs::remove_file(db_path);
    
    let db = prometheos_lite::db::repository::Db::new(db_path).unwrap();
    
    // Create a trust policy
    let policy = prometheos_lite::db::repository::TrustPolicyOperations::create_or_update_trust_policy(
        &db,
        "test_source",
        "Trusted",
        false,
    ).unwrap();
    
    assert_eq!(policy.source, "test_source");
    assert_eq!(policy.trust_level, "Trusted");
    assert!(!policy.require_approval);
    
    // Retrieve the policy
    let retrieved = prometheos_lite::db::repository::TrustPolicyOperations::get_trust_policy(
        &db,
        "test_source",
    ).unwrap();
    
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.source, "test_source");
    assert_eq!(retrieved.trust_level, "Trusted");
    
    // Update the policy
    let updated = prometheos_lite::db::repository::TrustPolicyOperations::create_or_update_trust_policy(
        &db,
        "test_source",
        "Untrusted",
        true,
    ).unwrap();
    
    assert_eq!(updated.trust_level, "Untrusted");
    assert!(updated.require_approval);
    
    // Clean up
    let _ = fs::remove_file(db_path);
}

#[test]
fn test_flow_snapshot_persistence() {
    // Test that flow snapshots can be persisted and retrieved
    use std::fs;
    
    let db_path = ".prometheos/test_snapshot.db";
    let _ = fs::remove_file(db_path);
    
    let db = prometheos_lite::db::repository::Db::new(db_path).unwrap();
    
    let flow_source = r#"
nodes:
  - id: test
    type: llm
"#;
    
    let source_hash = prometheos_lite::flow::FlowSnapshot::compute_hash(flow_source);
    
    // Create a snapshot
    let snapshot = prometheos_lite::db::repository::FlowSnapshotOperations::create_flow_snapshot(
        &db,
        "test_flow",
        "1.0",
        &source_hash,
        flow_source,
    ).unwrap();
    
    assert_eq!(snapshot.flow_name, "test_flow");
    assert_eq!(snapshot.source_hash, source_hash);
    
    // Retrieve by hash
    let retrieved = prometheos_lite::db::repository::FlowSnapshotOperations::get_flow_snapshot_by_hash(
        &db,
        &source_hash,
    ).unwrap();
    
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.flow_name, "test_flow");
    assert_eq!(retrieved.source_hash, source_hash);
    
    // Retrieve latest by name
    let latest = prometheos_lite::db::repository::FlowSnapshotOperations::get_latest_flow_snapshot(
        &db,
        "test_flow",
    ).unwrap();
    
    assert!(latest.is_some());
    let latest = latest.unwrap();
    assert_eq!(latest.flow_name, "test_flow");
    
    // Clean up
    let _ = fs::remove_file(db_path);
}

#[test]
fn test_interrupt_persistence() {
    // Test that interrupts can be persisted and retrieved
    use std::fs;
    
    let db_path = ".prometheos/test_interrupt.db";
    let _ = fs::remove_file(db_path);
    
    let db = prometheos_lite::db::repository::Db::new(db_path).unwrap();
    
    let schema = json!({"approved": true});
    
    // Create an interrupt
    let interrupt = prometheos_lite::db::repository::InterruptOperations::create_interrupt(
        &db,
        "run_123",
        "trace_456",
        "node_789",
        "Test interrupt",
        &schema.to_string(),
        None,
    ).unwrap();
    
    assert_eq!(interrupt.run_id, "run_123");
    assert_eq!(interrupt.status, "pending");
    
    // Retrieve the interrupt
    let retrieved = prometheos_lite::db::repository::InterruptOperations::get_interrupt(
        &db,
        &interrupt.id,
    ).unwrap();
    
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.run_id, "run_123");
    assert_eq!(retrieved.status, "pending");
    
    // Approve the interrupt
    prometheos_lite::db::repository::InterruptOperations::approve_interrupt(
        &db,
        &interrupt.id,
        &json!({"approved": true}).to_string(),
    ).unwrap();
    
    let approved = prometheos_lite::db::repository::InterruptOperations::get_interrupt(
        &db,
        &interrupt.id,
    ).unwrap().unwrap();
    
    assert_eq!(approved.status, "approved");
    
    // Clean up
    let _ = fs::remove_file(db_path);
}

#[test]
fn test_outbox_persistence() {
    // Test that outbox entries can be persisted and marked completed
    use std::fs;
    
    let db_path = ".prometheos/test_outbox.db";
    let _ = fs::remove_file(db_path);
    
    let db = prometheos_lite::db::repository::Db::new(db_path).unwrap();
    
    // Create an outbox entry
    let entry = prometheos_lite::db::repository::OutboxOperations::create_outbox_entry(
        &db,
        "run_123",
        "trace_456",
        "node_789",
        "file_writer",
        "hash_abc123",
    ).unwrap();
    
    assert_eq!(entry.run_id, "run_123");
    assert_eq!(entry.status, "pending");
    
    // Retrieve by hash
    let retrieved = prometheos_lite::db::repository::OutboxOperations::get_outbox_entry_by_hash(
        &db,
        "run_123",
        "node_789",
        "hash_abc123",
    ).unwrap();
    
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.status, "pending");
    
    // Mark as completed
    prometheos_lite::db::repository::OutboxOperations::mark_outbox_completed(
        &db,
        &entry.id,
        &json!({"success": true}).to_string(),
    ).unwrap();
    
    let completed = prometheos_lite::db::repository::OutboxOperations::get_outbox_entry_by_hash(
        &db,
        "run_123",
        "node_789",
        "hash_abc123",
    ).unwrap().unwrap();
    
    assert_eq!(completed.status, "completed");
    
    // List pending outbox
    let pending = prometheos_lite::db::repository::OutboxOperations::list_pending_outbox(
        &db,
        "run_123",
    ).unwrap();
    
    assert_eq!(pending.len(), 0);
    
    // Clean up
    let _ = fs::remove_file(db_path);
}
