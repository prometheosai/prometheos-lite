//! Flow output and evaluation structures

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::tracing::RunId;

/// FinalOutput - the contract for flow execution results
///
/// This struct represents the complete output of a flow execution,
/// including the primary result, additional outputs, metadata,
/// evaluation metrics, and budget information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalOutput {
    /// Unique identifier for this flow execution
    pub run_id: String,
    /// Trace ID for tracking individual operations within the run
    pub trace_id: String,
    /// Name of the flow that was executed
    pub flow_name: String,
    /// Primary output from the flow (the main result)
    pub primary: serde_json::Value,
    /// Additional outputs from the flow (named results)
    pub additional: std::collections::HashMap<String, serde_json::Value>,
    /// Evaluation metrics for this execution
    pub evaluation: Evaluation,
    /// Budget usage report (if budget was enforced)
    pub budget_report: Option<serde_json::Value>,
    /// V1.5: Context budget metadata
    pub context_budget: Option<ContextBudgetMetadata>,
    /// V1.5: Memory operations metadata
    pub memory_operations: Option<MemoryExecutionMetadata>,
    /// Number of trace events generated during execution
    pub events_count: usize,
    /// Timestamp when the flow execution started
    pub timestamp: DateTime<Utc>,
    /// Duration of the flow execution in milliseconds
    pub duration_ms: u64,
    /// Whether the flow execution succeeded
    pub success: bool,
    /// Error message if the flow execution failed
    pub error: Option<String>,
    /// Execution metadata from LLM calls (node_id -> GenerateResult)
    pub execution_metadata: std::collections::HashMap<String, serde_json::Value>,
}

/// V1.5: Context budget metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBudgetMetadata {
    pub max_tokens: usize,
    pub used_tokens: usize,
    pub dropped_items: Vec<String>,
    pub memory_count: usize,
    pub artifact_count: usize,
}

/// V1.5: Memory execution metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryExecutionMetadata {
    pub compressions_performed: usize,
    pub prunes_performed: usize,
    pub total_memory_count: usize,
}

impl FinalOutput {
    /// Create a new successful FinalOutput
    pub fn success(
        run_id: RunId,
        trace_id: String,
        flow_name: String,
        primary: serde_json::Value,
        additional: HashMap<String, serde_json::Value>,
        evaluation: Evaluation,
        budget_report: Option<serde_json::Value>,
        context_budget: Option<ContextBudgetMetadata>,
        memory_operations: Option<MemoryExecutionMetadata>,
        events_count: usize,
        duration_ms: u64,
    ) -> Self {
        Self {
            run_id,
            trace_id,
            flow_name,
            primary,
            additional,
            evaluation,
            budget_report,
            context_budget,
            memory_operations,
            events_count,
            timestamp: Utc::now(),
            duration_ms,
            success: true,
            error: None,
            execution_metadata: HashMap::new(),
        }
    }

    /// Create a new failed FinalOutput
    pub fn failure(
        run_id: RunId,
        trace_id: String,
        flow_name: String,
        error: String,
        duration_ms: u64,
    ) -> Self {
        Self {
            run_id: run_id.clone(),
            trace_id: trace_id.clone(),
            flow_name: flow_name.clone(),
            primary: serde_json::json!({
                "status": "failed",
                "error": error.clone()
            }),
            additional: HashMap::new(),
            evaluation: Evaluation::empty(run_id, flow_name),
            budget_report: None,
            context_budget: None,
            memory_operations: None,
            events_count: 0,
            timestamp: Utc::now(),
            duration_ms,
            success: false,
            error: Some(error),
            execution_metadata: HashMap::new(),
        }
    }

    /// Get a value from additional outputs
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.additional.get(key)
    }

    /// Check if output contains a specific key
    pub fn contains(&self, key: &str) -> bool {
        self.additional.contains_key(key)
    }
}

/// Evaluation metrics for flow execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evaluation {
    /// Run ID for this execution
    pub run_id: RunId,
    /// Flow name
    pub flow_name: String,
    /// Number of nodes executed
    pub nodes_executed: u32,
    /// Number of nodes that failed
    pub nodes_failed: u32,
    /// Number of transitions taken
    pub transitions_taken: u32,
    /// Total execution duration in milliseconds
    pub duration_ms: u64,
    /// Average node execution time in milliseconds
    pub avg_node_duration_ms: f64,
    /// Memory usage in bytes (if available)
    pub memory_usage_bytes: Option<u64>,
    /// Custom metrics
    pub custom_metrics: HashMap<String, serde_json::Value>,
    /// Timestamp when evaluation was generated
    pub timestamp: DateTime<Utc>,
}

impl Evaluation {
    /// Create a new Evaluation
    pub fn new(
        run_id: RunId,
        flow_name: String,
        nodes_executed: u32,
        nodes_failed: u32,
        transitions_taken: u32,
        duration_ms: u64,
    ) -> Self {
        let avg_node_duration_ms = if nodes_executed > 0 {
            duration_ms as f64 / nodes_executed as f64
        } else {
            0.0
        };

        Self {
            run_id,
            flow_name,
            nodes_executed,
            nodes_failed,
            transitions_taken,
            duration_ms,
            avg_node_duration_ms,
            memory_usage_bytes: None,
            custom_metrics: HashMap::new(),
            timestamp: Utc::now(),
        }
    }

    /// Create an empty Evaluation (for failed executions)
    pub fn empty(run_id: RunId, flow_name: String) -> Self {
        Self {
            run_id,
            flow_name,
            nodes_executed: 0,
            nodes_failed: 0,
            transitions_taken: 0,
            duration_ms: 0,
            avg_node_duration_ms: 0.0,
            memory_usage_bytes: None,
            custom_metrics: HashMap::new(),
            timestamp: Utc::now(),
        }
    }

    /// Add a custom metric
    pub fn with_custom_metric(mut self, key: String, value: serde_json::Value) -> Self {
        self.custom_metrics.insert(key, value);
        self
    }

    /// Set memory usage
    pub fn with_memory_usage(mut self, bytes: u64) -> Self {
        self.memory_usage_bytes = Some(bytes);
        self
    }

    /// Calculate success rate (0.0 to 1.0)
    pub fn success_rate(&self) -> f64 {
        if self.nodes_executed == 0 {
            0.0
        } else {
            (self.nodes_executed - self.nodes_failed) as f64 / self.nodes_executed as f64
        }
    }

    /// Check if evaluation indicates successful execution
    pub fn is_successful(&self) -> bool {
        self.nodes_failed == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_final_output_success() {
        let run_id = "test-run-123".to_string();
        let trace_id = "test-trace-abc".to_string();
        let mut additional = HashMap::new();
        additional.insert("review".to_string(), serde_json::json!("Good code"));

        let evaluation = Evaluation::new(run_id.clone(), "codegen".to_string(), 3, 0, 2, 1000);

        let output = FinalOutput::success(
            run_id.clone(),
            trace_id.clone(),
            "codegen".to_string(),
            serde_json::json!("generated code"),
            additional,
            evaluation,
            None,
            None,
            None,
            5,
            1000,
        );

        assert!(output.success);
        assert_eq!(output.run_id, run_id);
        assert_eq!(output.trace_id, trace_id);
        assert_eq!(output.flow_name, "codegen");
        assert!(output.contains("review"));
    }

    #[test]
    fn test_final_output_failure() {
        let run_id = "test-run-456".to_string();
        let trace_id = "test-trace-def".to_string();
        let output = FinalOutput::failure(
            run_id.clone(),
            trace_id.clone(),
            "codegen".to_string(),
            "LLM timeout".to_string(),
            500,
        );

        assert!(!output.success);
        assert_eq!(output.run_id, run_id);
        assert_eq!(output.trace_id, trace_id);
        assert_eq!(output.error, Some("LLM timeout".to_string()));
    }

    #[test]
    fn test_evaluation() {
        let run_id = "test-run-789".to_string();
        let eval = Evaluation::new(run_id.clone(), "codegen".to_string(), 5, 0, 4, 2000);

        assert_eq!(eval.run_id, run_id);
        assert_eq!(eval.nodes_executed, 5);
        assert_eq!(eval.success_rate(), 1.0);
        assert!(eval.is_successful());
    }

    #[test]
    fn test_evaluation_with_failures() {
        let eval = Evaluation::new("test-run".to_string(), "codegen".to_string(), 5, 2, 4, 2000);

        assert_eq!(eval.success_rate(), 0.6);
        assert!(!eval.is_successful());
    }

    #[test]
    fn test_evaluation_custom_metrics() {
        let eval = Evaluation::new("test-run".to_string(), "codegen".to_string(), 5, 0, 4, 2000)
            .with_custom_metric("tokens_used".to_string(), serde_json::json!(1000))
            .with_memory_usage(1024 * 1024);

        assert_eq!(
            eval.custom_metrics.get("tokens_used"),
            Some(&serde_json::json!(1000))
        );
        assert_eq!(eval.memory_usage_bytes, Some(1024 * 1024));
    }
}
