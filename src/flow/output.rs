//! Flow output and evaluation structures

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

use super::tracing::RunId;

/// Final output from a flow execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalOutput {
    /// Run ID for this execution
    pub run_id: RunId,
    /// Flow name
    pub flow_name: String,
    /// Primary output value (from FlowFile.outputs.primary)
    pub primary: serde_json::Value,
    /// Additional outputs (from FlowFile.outputs.include)
    pub additional: HashMap<String, serde_json::Value>,
    /// Timestamp when output was generated
    pub timestamp: DateTime<Utc>,
    /// Execution duration in milliseconds
    pub duration_ms: u64,
    /// Whether the execution succeeded
    pub success: bool,
    /// Error message if execution failed
    pub error: Option<String>,
}

impl FinalOutput {
    /// Create a new successful FinalOutput
    pub fn success(
        run_id: RunId,
        flow_name: String,
        primary: serde_json::Value,
        additional: HashMap<String, serde_json::Value>,
        duration_ms: u64,
    ) -> Self {
        Self {
            run_id,
            flow_name,
            primary,
            additional,
            timestamp: Utc::now(),
            duration_ms,
            success: true,
            error: None,
        }
    }

    /// Create a new failed FinalOutput
    pub fn failure(
        run_id: RunId,
        flow_name: String,
        error: String,
        duration_ms: u64,
    ) -> Self {
        Self {
            run_id,
            flow_name,
            primary: serde_json::Value::Null,
            additional: HashMap::new(),
            timestamp: Utc::now(),
            duration_ms,
            success: false,
            error: Some(error),
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
        let mut additional = HashMap::new();
        additional.insert("review".to_string(), serde_json::json!("Good code"));

        let output = FinalOutput::success(
            run_id.clone(),
            "codegen".to_string(),
            serde_json::json!("generated code"),
            additional,
            1000,
        );

        assert!(output.success);
        assert_eq!(output.run_id, run_id);
        assert_eq!(output.flow_name, "codegen");
        assert!(output.contains("review"));
    }

    #[test]
    fn test_final_output_failure() {
        let run_id = "test-run-456".to_string();
        let output = FinalOutput::failure(
            run_id.clone(),
            "codegen".to_string(),
            "LLM timeout".to_string(),
            500,
        );

        assert!(!output.success);
        assert_eq!(output.run_id, run_id);
        assert_eq!(output.error, Some("LLM timeout".to_string()));
    }

    #[test]
    fn test_evaluation() {
        let run_id = "test-run-789".to_string();
        let eval = Evaluation::new(
            run_id.clone(),
            "codegen".to_string(),
            5,
            0,
            4,
            2000,
        );

        assert_eq!(eval.run_id, run_id);
        assert_eq!(eval.nodes_executed, 5);
        assert_eq!(eval.success_rate(), 1.0);
        assert!(eval.is_successful());
    }

    #[test]
    fn test_evaluation_with_failures() {
        let eval = Evaluation::new(
            "test-run".to_string(),
            "codegen".to_string(),
            5,
            2,
            4,
            2000,
        );

        assert_eq!(eval.success_rate(), 0.6);
        assert!(!eval.is_successful());
    }

    #[test]
    fn test_evaluation_custom_metrics() {
        let eval = Evaluation::new(
            "test-run".to_string(),
            "codegen".to_string(),
            5,
            0,
            4,
            2000,
        )
        .with_custom_metric("tokens_used".to_string(), serde_json::json!(1000))
        .with_memory_usage(1024 * 1024);

        assert_eq!(eval.custom_metrics.get("tokens_used"), Some(&serde_json::json!(1000)));
        assert_eq!(eval.memory_usage_bytes, Some(1024 * 1024));
    }
}
