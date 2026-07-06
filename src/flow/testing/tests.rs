//! Tests for flow testing framework

use super::*;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn test_fixture_creation() {
    let input = serde_json::json!({
        "message": "test message"
    });

    let fixture = TestFixture::new(input.clone());
    assert_eq!(fixture.input, input);
    assert!(fixture.expected_output.is_none());
    assert!(fixture.expected_events.is_empty());
}

#[test]
fn test_fixture_with_expected_output() {
    let input = serde_json::json!({"message": "test"});
    let expected = serde_json::json!({"response": "test response"});

    let fixture = TestFixture::new(input).with_expected_output(expected.clone());

    assert_eq!(fixture.expected_output, Some(expected));
}

#[test]
fn test_fixture_with_expected_events() {
    let input = serde_json::json!({"message": "test"});
    let events = vec!["NodeStarted".to_string(), "NodeCompleted".to_string()];

    let fixture = TestFixture::new(input).with_expected_events(events.clone());

    assert_eq!(fixture.expected_events, events);
}

#[test]
fn test_expectation_creation() {
    let expectation = TestExpectation::new();
    assert!(expectation.outputs.is_empty());
    assert!(expectation.node_order.is_empty());
    assert!(expectation.min_steps.is_none());
    assert!(expectation.max_steps.is_none());
}

#[test]
fn test_expectation_with_output() {
    let expectation =
        TestExpectation::new().with_output("key".to_string(), serde_json::json!("value"));

    assert_eq!(expectation.outputs.len(), 1);
    assert_eq!(
        expectation.outputs.get("key"),
        Some(&serde_json::json!("value"))
    );
}

#[test]
fn test_expectation_with_node_order() {
    let order = vec!["node1".to_string(), "node2".to_string()];
    let expectation = TestExpectation::new().with_node_order(order.clone());

    assert_eq!(expectation.node_order, order);
}

#[test]
fn test_expectation_with_step_bounds() {
    let expectation = TestExpectation::new().with_step_bounds(Some(5), Some(10));

    assert_eq!(expectation.min_steps, Some(5));
    assert_eq!(expectation.max_steps, Some(10));
}

#[test]
fn test_flow_test_runner_creation() {
    let flow_path = PathBuf::from("test.flow.yaml");
    let _runner = FlowTestRunner::new(flow_path);

    // Runner created successfully
}

#[test]
fn test_flow_test_runner_with_scripted_response() {
    let flow_path = PathBuf::from("test.flow.yaml");
    let _runner = FlowTestRunner::new(flow_path)
        .with_scripted_response("node1".to_string(), "scripted response".to_string());

    // Runner with scripted response created successfully
}

#[test]
fn test_flow_test_runner_with_tracing() {
    let flow_path = PathBuf::from("test.flow.yaml");
    let _runner = FlowTestRunner::new(flow_path).with_tracing();

    // Runner with tracing created successfully
}

#[test]
fn test_fixture_json_serialization() {
    let temp_dir = TempDir::new().unwrap();
    let fixture_path = temp_dir.path().join("fixture.json");

    let input = serde_json::json!({"message": "test"});
    let expected = serde_json::json!({"response": "test response"});
    let events = vec!["NodeStarted".to_string()];

    let fixture = TestFixture::new(input.clone())
        .with_expected_output(expected.clone())
        .with_expected_events(events.clone());

    // Save to file
    fixture.to_json(&fixture_path).unwrap();
    assert!(fixture_path.exists());

    // Load from file
    let loaded_fixture = TestFixture::from_json(&fixture_path).unwrap();
    assert_eq!(loaded_fixture.input, input);
    assert_eq!(loaded_fixture.expected_output, Some(expected));
    assert_eq!(loaded_fixture.expected_events, events);
}
