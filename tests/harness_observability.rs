//! Issue 26: Observability Layer Tests
//!
//! Comprehensive tests for the Observability Layer including:
//! - HarnessMetrics struct (execution metrics, tokens, cost, custom metrics)
//! - ObservabilityCollector for metrics collection
//! - OperationSpan struct (name, timing, status, attributes, events)
//! - SpanStatus enum (Ok, Error, InProgress)
//! - SpanEvent struct (timestamp, name, attributes)
//! - Metrics recording and reporting
//! - Span lifecycle management

use std::collections::HashMap;

use prometheos_lite::harness::observability::{
    HarnessMetrics, ObservabilityCollector, OperationSpan, SpanEvent, SpanStatus,
};

// ============================================================================
// HarnessMetrics Tests
// ============================================================================

#[test]
fn test_harness_metrics_creation() {
    let metrics = HarnessMetrics {
        execution_id: "exec-123".to_string(),
        start_time: chrono::Utc::now(),
        end_time: None,
        duration_ms: 0,
        steps_completed: 5,
        steps_failed: 1,
        patches_generated: 3,
        patches_applied: 2,
        validations_run: 4,
        validations_passed: 3,
        reviews_performed: 2,
        issues_found: 5,
        commands_executed: 10,
        tokens_consumed: 50000,
        cost_usd: 0.75,
        custom_metrics: HashMap::new(),
    };

    assert_eq!(metrics.execution_id, "exec-123");
    assert_eq!(metrics.steps_completed, 5);
    assert_eq!(metrics.tokens_consumed, 50000);
    assert_eq!(metrics.cost_usd, 0.75);
    assert!(metrics.end_time.is_none());
}

#[test]
fn test_harness_metrics_with_custom() {
    let mut custom = HashMap::new();
    custom.insert("cache_hits".to_string(), 100.0);
    custom.insert("cache_misses".to_string(), 10.0);

    let metrics = HarnessMetrics {
        execution_id: "exec-456".to_string(),
        start_time: chrono::Utc::now(),
        end_time: Some(chrono::Utc::now()),
        duration_ms: 30000,
        steps_completed: 10,
        steps_failed: 0,
        patches_generated: 5,
        patches_applied: 5,
        validations_run: 8,
        validations_passed: 8,
        reviews_performed: 4,
        issues_found: 0,
        commands_executed: 20,
        tokens_consumed: 100000,
        cost_usd: 1.50,
        custom_metrics: custom,
    };

    assert_eq!(metrics.custom_metrics.len(), 2);
    assert!(metrics.end_time.is_some());
}

// ============================================================================
// ObservabilityCollector Tests
// ============================================================================

#[test]
fn test_observability_collector_new() {
    let _collector = ObservabilityCollector::new("test-exec".to_string());
    // Collector created successfully
}

// ============================================================================
// OperationSpan Tests
// ============================================================================

#[test]
fn test_operation_span_creation() {
    let span = OperationSpan {
        name: "validation".to_string(),
        start_time: chrono::Utc::now(),
        end_time: None,
        duration_ms: 0,
        status: SpanStatus::InProgress,
        attributes: HashMap::new(),
        events: vec![],
    };

    assert_eq!(span.name, "validation");
    assert!(matches!(span.status, SpanStatus::InProgress));
    assert!(span.end_time.is_none());
}

#[test]
fn test_operation_span_completed() {
    let start = chrono::Utc::now();
    let end = start + chrono::Duration::seconds(5);

    let span = OperationSpan {
        name: "patch_application".to_string(),
        start_time: start,
        end_time: Some(end),
        duration_ms: 5000,
        status: SpanStatus::Ok,
        attributes: {
            let mut map = HashMap::new();
            map.insert("file_count".to_string(), "3".to_string());
            map
        },
        events: vec![],
    };

    assert!(matches!(span.status, SpanStatus::Ok));
    assert_eq!(span.duration_ms, 5000);
    assert_eq!(span.attributes.get("file_count"), Some(&"3".to_string()));
}

#[test]
fn test_operation_span_error() {
    let span = OperationSpan {
        name: "validation".to_string(),
        start_time: chrono::Utc::now(),
        end_time: Some(chrono::Utc::now()),
        duration_ms: 1000,
        status: SpanStatus::Error("Test failed".to_string()),
        attributes: HashMap::new(),
        events: vec![],
    };

    assert!(matches!(span.status, SpanStatus::Error(_)));
    if let SpanStatus::Error(msg) = span.status {
        assert_eq!(msg, "Test failed");
    }
}

// ============================================================================
// SpanStatus Tests
// ============================================================================

#[test]
fn test_span_status_variants() {
    assert!(matches!(SpanStatus::Ok, SpanStatus::Ok));
    assert!(matches!(SpanStatus::InProgress, SpanStatus::InProgress));
    assert!(matches!(
        SpanStatus::Error("test".to_string()),
        SpanStatus::Error(_)
    ));
}

// ============================================================================
// SpanEvent Tests
// ============================================================================

#[test]
fn test_span_event_creation() {
    let event = SpanEvent {
        timestamp: chrono::Utc::now(),
        name: "validation_start".to_string(),
        attributes: {
            let mut map = HashMap::new();
            map.insert("command".to_string(), "cargo test".to_string());
            map
        },
    };

    assert_eq!(event.name, "validation_start");
    assert_eq!(
        event.attributes.get("command"),
        Some(&"cargo test".to_string())
    );
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_metrics_workflow() {
    let mut metrics = HarnessMetrics {
        execution_id: "workflow-test".to_string(),
        start_time: chrono::Utc::now(),
        end_time: None,
        duration_ms: 0,
        steps_completed: 0,
        steps_failed: 0,
        patches_generated: 0,
        patches_applied: 0,
        validations_run: 0,
        validations_passed: 0,
        reviews_performed: 0,
        issues_found: 0,
        commands_executed: 0,
        tokens_consumed: 0,
        cost_usd: 0.0,
        custom_metrics: HashMap::new(),
    };

    // Simulate workflow progress
    metrics.steps_completed = 3;
    metrics.patches_generated = 2;
    metrics.validations_run = 3;
    metrics.tokens_consumed = 25000;
    metrics.cost_usd = 0.50;
    metrics.end_time = Some(chrono::Utc::now());
    metrics.duration_ms = 15000;

    assert_eq!(metrics.steps_completed, 3);
    assert!(metrics.end_time.is_some());
    assert_eq!(metrics.duration_ms, 15000);
}

#[test]
fn test_span_with_events() {
    let span = OperationSpan {
        name: "execution".to_string(),
        start_time: chrono::Utc::now(),
        end_time: Some(chrono::Utc::now()),
        duration_ms: 5000,
        status: SpanStatus::Ok,
        attributes: HashMap::new(),
        events: vec![
            SpanEvent {
                timestamp: chrono::Utc::now(),
                name: "step_1_complete".to_string(),
                attributes: HashMap::new(),
            },
            SpanEvent {
                timestamp: chrono::Utc::now(),
                name: "step_2_complete".to_string(),
                attributes: HashMap::new(),
            },
        ],
    };

    assert_eq!(span.events.len(), 2);
    assert!(matches!(span.status, SpanStatus::Ok));
}
