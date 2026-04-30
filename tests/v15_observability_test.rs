//! V1.5 Observability Tests
//!
//! Tests for trace storage, trace hierarchy, and failure trace visibility.

use chrono::Utc;
use prometheos::flow::tracing::{HierarchicalTrace, NodeRun, ToolCall, LlmCall, TraceStorage};
use uuid::Uuid;

fn create_test_trace() -> HierarchicalTrace {
    HierarchicalTrace {
        trace_id: Uuid::new_v4(),
        work_context_id: Some("test_context".to_string()),
        flow_run_id: Uuid::new_v4(),
        node_runs: vec![
            NodeRun {
                node_id: "node1".to_string(),
                trace_id: Uuid::new_v4(),
                input_summary: Some("test input".to_string()),
                output_summary: Some("test output".to_string()),
                status: "completed".to_string(),
                duration_ms: 100,
                error: None,
                started_at: Utc::now(),
                completed_at: Utc::now(),
            }
        ],
        tool_calls: vec![
            ToolCall {
                tool_name: "test_tool".to_string(),
                trace_id: Uuid::new_v4(),
                args_hash: "args_hash".to_string(),
                result_hash: "result_hash".to_string(),
                success: true,
                duration_ms: 50,
                called_at: Utc::now(),
            }
        ],
        llm_calls: vec![
            LlmCall {
                node_id: "node1".to_string(),
                trace_id: Uuid::new_v4(),
                provider: "openai".to_string(),
                model: "gpt-4".to_string(),
                prompt_tokens: 100,
                completion_tokens: 50,
                latency_ms: 200,
                error: None,
                started_at: Utc::now(),
                completed_at: Some(Utc::now()),
            }
        ],
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
    }
}

#[test]
fn test_trace_storage_init() {
    let storage = TraceStorage::in_memory().unwrap();
    // Schema should be created without errors
}

#[test]
fn test_save_and_get_trace() {
    let storage = TraceStorage::in_memory().unwrap();

    let trace = create_test_trace();
    storage.save_trace(&trace).unwrap();
    
    let retrieved = storage.get_trace(&trace.trace_id.to_string()).unwrap();
    
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().trace_id, trace.trace_id);
}

#[test]
fn test_trace_hierarchy() {
    let storage = TraceStorage::in_memory().unwrap();

    let trace = create_test_trace();
    storage.save_trace(&trace).unwrap();
    
    let retrieved = storage.get_trace(&trace.trace_id.to_string()).unwrap().unwrap();
    
    // Verify hierarchy: trace has node runs, node runs have metadata
    assert!(!retrieved.node_runs.is_empty());
    assert!(!retrieved.tool_calls.is_empty());
    assert!(!retrieved.llm_calls.is_empty());
    
    // Verify node run has required fields
    let node_run = &retrieved.node_runs[0];
    assert!(!node_run.node_id.is_empty());
    assert!(!node_run.status.is_empty());
    assert!(node_run.duration_ms > 0);
}

#[test]
fn test_delete_trace() {
    let storage = TraceStorage::in_memory().unwrap();

    let trace = create_test_trace();
    storage.save_trace(&trace).unwrap();
    storage.delete_trace(&trace.trace_id.to_string()).unwrap();
    
    let retrieved = storage.get_trace(&trace.trace_id.to_string()).unwrap();
    assert!(retrieved.is_none());
}

#[test]
fn test_get_traces_by_flow_run() {
    let storage = TraceStorage::in_memory().unwrap();

    let flow_run_id = Uuid::new_v4();
    let mut trace1 = create_test_trace();
    trace1.flow_run_id = flow_run_id;
    
    let mut trace2 = create_test_trace();
    trace2.flow_run_id = flow_run_id;
    
    storage.save_trace(&trace1).unwrap();
    storage.save_trace(&trace2).unwrap();
    
    let traces = storage.get_traces_by_flow_run(&flow_run_id.to_string()).unwrap();
    assert_eq!(traces.len(), 2);
}

#[test]
fn test_trace_generation_correctness() {
    let storage = TraceStorage::in_memory().unwrap();

    let trace = create_test_trace();
    storage.save_trace(&trace).unwrap();
    
    let retrieved = storage.get_trace(&trace.trace_id.to_string()).unwrap().unwrap();
    
    // Every execution should have trace_id
    assert!(!retrieved.trace_id.to_string().is_empty());
    
    // Every node should be logged
    assert!(!retrieved.node_runs.is_empty());
    
    // Every tool call should be logged
    assert!(!retrieved.tool_calls.is_empty());
    
    // LLM metrics should be present
    assert!(!retrieved.llm_calls.is_empty());
    let llm_call = &retrieved.llm_calls[0];
    assert!(llm_call.prompt_tokens > 0);
    assert!(llm_call.completion_tokens > 0);
    assert!(llm_call.latency_ms > 0);
    assert!(!llm_call.provider.is_empty());
    assert!(!llm_call.model.is_empty());
}

#[test]
fn test_failure_trace_visibility() {
    let storage = TraceStorage::in_memory().unwrap();

    let mut trace = create_test_trace();
    
    // Add a failed node
    trace.node_runs.push(NodeRun {
        node_id: "failed_node".to_string(),
        trace_id: Uuid::new_v4(),
        input_summary: Some("failing input".to_string()),
        output_summary: None,
        status: "failed".to_string(),
        duration_ms: 50,
        error: Some("Test error".to_string()),
        started_at: Utc::now(),
        completed_at: Utc::now(),
    });
    
    storage.save_trace(&trace).unwrap();
    
    let retrieved = storage.get_trace(&trace.trace_id.to_string()).unwrap().unwrap();
    
    // Failure should be traceable
    let failed_node = retrieved.node_runs.iter().find(|n| n.status == "failed");
    assert!(failed_node.is_some());
    assert!(failed_node.unwrap().error.is_some());
}

#[test]
fn test_llm_metrics_in_trace() {
    let storage = TraceStorage::in_memory().unwrap();

    let trace = create_test_trace();
    storage.save_trace(&trace).unwrap();
    
    let retrieved = storage.get_trace(&trace.trace_id.to_string()).unwrap().unwrap();
    
    let llm_call = &retrieved.llm_calls[0];
    
    // Verify LLM metrics
    assert!(llm_call.prompt_tokens > 0);
    assert!(llm_call.completion_tokens > 0);
    assert!(llm_call.latency_ms > 0);
    assert!(!llm_call.provider.is_empty());
    assert!(!llm_call.model.is_empty());
    assert!(llm_call.started_at <= llm_call.completed_at.unwrap_or(llm_call.started_at));
}

#[test]
fn test_delete_traces_older_than() {
    let storage = TraceStorage::in_memory().unwrap();

    let mut old_trace = create_test_trace();
    old_trace.started_at = Utc::now() - chrono::Duration::days(30);
    
    let mut recent_trace = create_test_trace();
    recent_trace.started_at = Utc::now();
    
    storage.save_trace(&old_trace).unwrap();
    storage.save_trace(&recent_trace).unwrap();
    
    let cutoff = Utc::now() - chrono::Duration::days(7);
    let deleted = storage.delete_traces_older_than(cutoff).unwrap();
    
    assert!(deleted > 0);
}

#[test]
fn test_tool_call_metrics() {
    let storage = TraceStorage::in_memory().unwrap();

    let trace = create_test_trace();
    storage.save_trace(&trace).unwrap();
    
    let retrieved = storage.get_trace(&trace.trace_id.to_string()).unwrap().unwrap();
    
    let tool_call = &retrieved.tool_calls[0];
    
    // Verify tool call metrics
    assert!(!tool_call.tool_name.is_empty());
    assert!(!tool_call.args_hash.is_empty());
    assert!(!tool_call.result_hash.is_empty());
    assert!(tool_call.duration_ms > 0);
    assert!(tool_call.called_at <= Utc::now());
}
