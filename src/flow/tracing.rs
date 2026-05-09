//! Logging & Tracing - structured logs and event timeline

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::flow::{NodeId, SharedState};

pub mod storage;

pub use storage::TraceStorage;

/// Run ID for tracking a complete flow execution
pub type RunId = String;

/// Trace ID for tracking individual operations within a run
pub type TraceId = String;

/// Event types for tracing - Two-layer model
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TraceEvent {
    // Run-level events (system-level)
    RunStarted {
        run_id: RunId,
        flow_name: String,
    },
    RunCompleted {
        run_id: RunId,
        duration_ms: u64,
    },
    RunFailed {
        run_id: RunId,
        error: String,
    },

    // Flow-level events (node-level)
    NodeStarted {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        input_summary: Option<String>,
    },
    NodeCompleted {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        duration_ms: u64,
        output_summary: Option<String>,
        status: String,
    },
    NodeFailed {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        error: String,
        input_summary: Option<String>,
    },
    TransitionTaken {
        run_id: RunId,
        from: NodeId,
        action: String,
        to: NodeId,
    },

    // Flow lifecycle events
    FlowLoaded {
        run_id: RunId,
        flow_name: String,
        path: String,
    },
    FlowValidationFailed {
        run_id: RunId,
        errors: Vec<String>,
    },

    // Budget events
    BudgetChecked {
        run_id: RunId,
        resource: String,
        current: u64,
        limit: u64,
    },
    BudgetExceeded {
        run_id: RunId,
        resource: String,
        current: u64,
        limit: u64,
    },

    // Tool events
    ToolRequested {
        run_id: RunId,
        trace_id: TraceId,
        tool_name: String,
        args_hash: String,
    },
    ToolCompleted {
        run_id: RunId,
        trace_id: TraceId,
        tool_name: String,
        args_hash: String,
        result_hash: String,
        duration_ms: u64,
        success: bool,
    },

    // V1.4 Tool call logs
    ToolCallLog {
        run_id: RunId,
        trace_id: TraceId,
        tool_name: String,
        input: serde_json::Value,
        output: serde_json::Value,
        duration_ms: u64,
        success: bool,
        error: Option<String>,
    },

    // V1.4 Command logs
    CommandLog {
        run_id: RunId,
        trace_id: TraceId,
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
        stdout: String,
        stderr: String,
        exit_code: i32,
        duration_ms: u64,
        success: bool,
    },

    // V1.4 Execution trace per WorkContext
    WorkContextTrace {
        work_context_id: String,
        run_id: RunId,
        trace_id: TraceId,
        phase: String,
        action: String,
        timestamp: DateTime<Utc>,
        metadata: serde_json::Value,
    },

    // Memory events
    MemoryRead {
        run_id: RunId,
        trace_id: TraceId,
        query: String,
        results_count: u32,
    },
    MemoryWrite {
        run_id: RunId,
        trace_id: TraceId,
        kind: String,
    },

    // Output events
    EvaluationCompleted {
        run_id: RunId,
        trace_id: TraceId,
        score: Option<f64>,
    },
    OutputGenerated {
        run_id: RunId,
        trace_id: TraceId,
        output_key: String,
    },

    // LLM events
    LlmRequestStarted {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        provider: String,
        model: String,
    },
    LlmRequestCompleted {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        provider: String,
        model: String,
        prompt_tokens: u32,
        completion_tokens: u32,
        latency_ms: u64,
    },
    LlmRequestFailed {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        provider: String,
        model: String,
        error: String,
    },

    // Guardrail events - V1.1
    PermissionChecked {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        permission: String,
        allowed: bool,
    },
    PermissionDenied {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        permission: String,
        reason: String,
    },
    ApprovalRequested {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        interrupt_id: String,
    },
    ApprovalGranted {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        interrupt_id: String,
    },
    ApprovalDenied {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        interrupt_id: String,
    },
    InterruptCreated {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        interrupt_id: String,
        reason: String,
    },
    InterruptResumed {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        interrupt_id: String,
    },
    FlowSnapshotStored {
        run_id: RunId,
        flow_name: String,
        source_hash: String,
    },
    SchemaHashChecked {
        run_id: RunId,
        flow_name: String,
        hash_match: bool,
    },
    IdempotencyChecked {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        idempotency_key: String,
        is_duplicate: bool,
    },
    OutboxPending {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        tool_name: String,
        outbox_id: String,
    },
    OutboxCompleted {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        tool_name: String,
        outbox_id: String,
    },
    TrustPolicyApplied {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        source: String,
        trust_level: String,
    },
    LoopDetected {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        loop_type: String,
    },
}

/// Log level
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

/// Hierarchical trace structure for tracking execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchicalTrace {
    /// Top-level trace ID
    pub trace_id: TraceId,
    /// WorkContext ID (if applicable)
    pub work_context_id: Option<String>,
    /// Flow run ID
    pub flow_run_id: RunId,
    /// Node executions within this trace
    pub node_runs: Vec<NodeRun>,
    /// Tool calls within this trace
    pub tool_calls: Vec<ToolCall>,
    /// LLM calls within this trace
    pub llm_calls: Vec<LlmCall>,
    /// Timestamp when trace started
    pub started_at: DateTime<Utc>,
    /// Timestamp when trace completed
    pub completed_at: Option<DateTime<Utc>>,
}

/// Node execution record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRun {
    /// Node ID
    pub node_id: NodeId,
    /// Trace ID for this node execution
    pub trace_id: TraceId,
    /// Input summary
    pub input_summary: Option<String>,
    /// Output summary
    pub output_summary: Option<String>,
    /// Execution status
    pub status: String,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Error if failed
    pub error: Option<String>,
    /// Timestamp when node started
    pub started_at: DateTime<Utc>,
    /// Timestamp when node completed
    pub completed_at: DateTime<Utc>,
}

/// Tool call record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Tool name
    pub tool_name: String,
    /// Trace ID for this tool call
    pub trace_id: TraceId,
    /// Arguments hash
    pub args_hash: String,
    /// Result hash
    pub result_hash: String,
    /// Whether the call succeeded
    pub success: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Timestamp when tool was called
    pub called_at: DateTime<Utc>,
}

/// LLM call record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCall {
    /// Node ID that made the LLM call
    pub node_id: NodeId,
    /// Trace ID for this LLM call
    pub trace_id: TraceId,
    /// Provider (e.g., "openai", "anthropic")
    pub provider: String,
    /// Model name
    pub model: String,
    /// Number of prompt tokens
    pub prompt_tokens: u32,
    /// Number of completion tokens
    pub completion_tokens: u32,
    /// Latency in milliseconds
    pub latency_ms: u64,
    /// Error if failed
    pub error: Option<String>,
    /// Timestamp when LLM request started
    pub started_at: DateTime<Utc>,
    /// Timestamp when LLM request completed
    pub completed_at: Option<DateTime<Utc>>,
}

/// Structured log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub event_type: TraceEvent,
    pub node_id: Option<NodeId>,
    pub message: String,
    pub metadata: serde_json::Value,
}

/// Timeline event for execution tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: TraceEvent,
    pub node_id: Option<NodeId>,
    pub duration_ms: Option<u64>,
    pub details: serde_json::Value,
}

/// Execution metrics from tracing
#[derive(Debug, Clone, Copy, Default)]
pub struct TraceMetrics {
    pub llm_calls: u32,
    pub tool_calls: u32,
    pub node_runs: u32,
    /// Whether execution budget was exceeded during the run
    pub budget_exceeded: bool,
}

/// Tracer for structured logging and event timeline
#[derive(Debug)]
pub struct Tracer {
    logs: Vec<LogEntry>,
    timeline: Vec<TimelineEvent>,
    hierarchical_trace: Option<HierarchicalTrace>,
    enabled: bool,
    min_level: LogLevel,
}

impl Tracer {
    pub fn new() -> Self {
        Self {
            logs: Vec::new(),
            timeline: Vec::new(),
            hierarchical_trace: None,
            enabled: true,
            min_level: LogLevel::Info,
        }
    }

    pub fn with_level(min_level: LogLevel) -> Self {
        Self {
            logs: Vec::new(),
            timeline: Vec::new(),
            hierarchical_trace: None,
            enabled: true,
            min_level,
        }
    }

    pub fn with_hierarchical_trace(
        trace_id: TraceId,
        flow_run_id: RunId,
        work_context_id: Option<String>,
    ) -> Self {
        Self {
            logs: Vec::new(),
            timeline: Vec::new(),
            hierarchical_trace: Some(HierarchicalTrace {
                trace_id,
                work_context_id,
                flow_run_id,
                node_runs: Vec::new(),
                tool_calls: Vec::new(),
                llm_calls: Vec::new(),
                started_at: Utc::now(),
                completed_at: None,
            }),
            enabled: true,
            min_level: LogLevel::Info,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_min_level(&mut self, level: LogLevel) {
        self.min_level = level;
    }

    /// Log an event
    pub fn log(
        &mut self,
        level: LogLevel,
        event_type: TraceEvent,
        node_id: Option<NodeId>,
        message: String,
        metadata: serde_json::Value,
    ) {
        if !self.enabled || level < self.min_level {
            return;
        }

        self.logs.push(LogEntry {
            timestamp: Utc::now(),
            level,
            event_type,
            node_id,
            message,
            metadata,
        });
    }

    /// Add a timeline event
    pub fn add_timeline_event(
        &mut self,
        event_type: TraceEvent,
        node_id: Option<NodeId>,
        duration_ms: Option<u64>,
        details: serde_json::Value,
    ) {
        if !self.enabled {
            return;
        }

        self.timeline.push(TimelineEvent {
            timestamp: Utc::now(),
            event_type,
            node_id,
            duration_ms,
            details,
        });
    }

    /// Log a run-level event
    pub fn log_run_event(&mut self, event: TraceEvent, message: String) {
        let level = match &event {
            TraceEvent::RunFailed { .. } => LogLevel::Error,
            TraceEvent::FlowValidationFailed { .. } => LogLevel::Error,
            TraceEvent::BudgetExceeded { .. } => LogLevel::Warning,
            _ => LogLevel::Info,
        };
        self.log(level, event, None, message, serde_json::json!({}));
    }

    /// Log a flow-level event
    pub fn log_flow_event(&mut self, event: TraceEvent, node_id: Option<NodeId>, message: String) {
        let level = match &event {
            TraceEvent::NodeFailed { .. } => LogLevel::Error,
            TraceEvent::BudgetExceeded { .. } => LogLevel::Warning,
            _ => LogLevel::Info,
        };
        self.log(level, event, node_id, message, serde_json::json!({}));
    }

    /// Generate a new run ID
    pub fn generate_run_id() -> RunId {
        Uuid::new_v4().to_string()
    }

    /// Generate a new trace ID
    pub fn generate_trace_id() -> TraceId {
        Uuid::new_v4().to_string()
    }

    /// Get all logs
    pub fn get_logs(&self) -> &[LogEntry] {
        &self.logs
    }

    /// Get logs filtered by level
    pub fn get_logs_by_level(&self, level: LogLevel) -> Vec<&LogEntry> {
        self.logs.iter().filter(|log| log.level >= level).collect()
    }

    /// Get logs for a specific node
    pub fn get_logs_for_node(&self, node_id: &NodeId) -> Vec<&LogEntry> {
        self.logs
            .iter()
            .filter(|log| log.node_id.as_ref() == Some(node_id))
            .collect()
    }

    /// Get timeline
    pub fn get_timeline(&self) -> &[TimelineEvent] {
        &self.timeline
    }

    /// Clear all logs and timeline
    pub fn clear(&mut self) {
        self.logs.clear();
        self.timeline.clear();
        self.hierarchical_trace = None;
    }

    /// Get hierarchical trace
    pub fn get_hierarchical_trace(&self) -> Option<&HierarchicalTrace> {
        self.hierarchical_trace.as_ref()
    }

    /// Complete the hierarchical trace
    pub fn complete_hierarchical_trace(&mut self) {
        if let Some(ref mut trace) = self.hierarchical_trace {
            trace.completed_at = Some(Utc::now());
        }
    }

    /// Add a node run to hierarchical trace
    pub fn add_node_run(&mut self, node_run: NodeRun) {
        if let Some(ref mut trace) = self.hierarchical_trace {
            trace.node_runs.push(node_run);
        }
    }

    /// Add a tool call to hierarchical trace
    pub fn add_tool_call(&mut self, tool_call: ToolCall) {
        if let Some(ref mut trace) = self.hierarchical_trace {
            trace.tool_calls.push(tool_call);
        }
    }

    /// Add an LLM call to hierarchical trace
    pub fn add_llm_call(&mut self, llm_call: LlmCall) {
        if let Some(ref mut trace) = self.hierarchical_trace {
            trace.llm_calls.push(llm_call);
        }
    }

    /// Get execution metrics from hierarchical trace
    pub fn get_metrics(&self) -> TraceMetrics {
        if let Some(ref trace) = self.hierarchical_trace {
            TraceMetrics {
                llm_calls: trace.llm_calls.len() as u32,
                tool_calls: trace.tool_calls.len() as u32,
                node_runs: trace.node_runs.len() as u32,
                budget_exceeded: false, // Budget tracking via BudgetGuard in SharedState
            }
        } else {
            TraceMetrics {
                llm_calls: 0,
                tool_calls: 0,
                node_runs: 0,
                budget_exceeded: false,
            }
        }
    }

    /// Export logs as JSON
    pub fn export_logs(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.logs)
            .map_err(|e| anyhow::anyhow!("Failed to export logs: {}", e))
    }

    /// Export timeline as JSON
    pub fn export_timeline(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.timeline)
            .map_err(|e| anyhow::anyhow!("Failed to export timeline: {}", e))
    }

    /// Export hierarchical trace as JSON
    pub fn export_hierarchical_trace(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.hierarchical_trace)
            .map_err(|e| anyhow::anyhow!("Failed to export hierarchical trace: {}", e))
    }

    /// Create a summary from a JSON value (truncated if too long)
    pub fn summarize_value(value: &serde_json::Value, max_length: usize) -> Option<String> {
        let json_str = value.to_string();
        if json_str.is_empty() {
            return None;
        }
        if json_str.len() <= max_length {
            Some(json_str)
        } else {
            Some(format!("{}...", &json_str[..max_length]))
        }
    }

    /// Compute hash of a JSON value for idempotency tracking
    pub fn compute_hash(value: &serde_json::Value) -> String {
        format!("{:x}", md5::compute(value.to_string().as_bytes()))
    }
}

impl Default for Tracer {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared tracer for use across the application
pub type SharedTracer = Arc<Mutex<Tracer>>;

/// Create a new shared tracer
pub fn create_tracer() -> SharedTracer {
    Arc::new(Mutex::new(Tracer::new()))
}

/// Create a shared tracer with a specific log level
pub fn create_tracer_with_level(level: LogLevel) -> SharedTracer {
    Arc::new(Mutex::new(Tracer::with_level(level)))
}

/// Macro for logging
#[macro_export]
macro_rules! trace_log {
    ($tracer:expr, $level:expr, $event_type:expr, $node_id:expr, $message:expr) => {
        if let Ok(mut tracer) = $tracer.lock() {
            tracer.log(
                $level,
                $event_type,
                $node_id,
                $message.to_string(),
                serde_json::json!({}),
            );
        }
    };
    ($tracer:expr, $level:expr, $event_type:expr, $node_id:expr, $message:expr, $metadata:expr) => {
        if let Ok(mut tracer) = $tracer.lock() {
            tracer.log(
                $level,
                $event_type,
                $node_id,
                $message.to_string(),
                $metadata,
            );
        }
    };
}

/// Macro for timeline events
#[macro_export]
macro_rules! trace_event {
    ($tracer:expr, $event_type:expr, $node_id:expr, $duration_ms:expr, $details:expr) => {
        if let Ok(mut tracer) = $tracer.lock() {
            tracer.add_timeline_event($event_type, $node_id, $duration_ms, $details);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracer_log() {
        let mut tracer = Tracer::new();
        let run_id = Tracer::generate_run_id();

        tracer.log(
            LogLevel::Info,
            TraceEvent::RunStarted {
                run_id: run_id.clone(),
                flow_name: "test".to_string(),
            },
            None,
            "Starting run".to_string(),
            serde_json::json!({}),
        );

        assert_eq!(tracer.get_logs().len(), 1);
        assert_eq!(tracer.get_logs()[0].level, LogLevel::Info);
    }

    #[test]
    fn test_tracer_level_filtering() {
        let mut tracer = Tracer::with_level(LogLevel::Warning);
        let run_id = Tracer::generate_run_id();

        tracer.log(
            LogLevel::Debug,
            TraceEvent::NodeStarted {
                run_id: run_id.clone(),
                trace_id: Tracer::generate_trace_id(),
                node_id: "node1".to_string(),
                input_summary: None,
            },
            Some("node1".to_string()),
            "Debug message".to_string(),
            serde_json::json!({}),
        );

        tracer.log(
            LogLevel::Warning,
            TraceEvent::NodeFailed {
                run_id,
                trace_id: Tracer::generate_trace_id(),
                node_id: "node1".to_string(),
                error: "Test error".to_string(),
                input_summary: None,
            },
            Some("node1".to_string()),
            "Warning message".to_string(),
            serde_json::json!({}),
        );

        assert_eq!(tracer.get_logs().len(), 1);
        assert_eq!(tracer.get_logs()[0].level, LogLevel::Warning);
    }

    #[test]
    fn test_tracer_timeline() {
        let mut tracer = Tracer::new();
        let run_id = Tracer::generate_run_id();

        tracer.add_timeline_event(
            TraceEvent::NodeStarted {
                run_id: run_id.clone(),
                trace_id: Tracer::generate_trace_id(),
                node_id: "node1".to_string(),
                input_summary: None,
            },
            Some("node1".to_string()),
            Some(100),
            serde_json::json!({}),
        );

        assert_eq!(tracer.get_timeline().len(), 1);
        assert_eq!(tracer.get_timeline()[0].duration_ms, Some(100));
    }

    #[test]
    fn test_tracer_filter_by_node() {
        let mut tracer = Tracer::new();
        let run_id = Tracer::generate_run_id();

        tracer.log(
            LogLevel::Info,
            TraceEvent::NodeStarted {
                run_id: run_id.clone(),
                trace_id: Tracer::generate_trace_id(),
                node_id: "node1".to_string(),
                input_summary: None,
            },
            Some("node1".to_string()),
            "Message 1".to_string(),
            serde_json::json!({}),
        );

        tracer.log(
            LogLevel::Info,
            TraceEvent::NodeStarted {
                run_id,
                trace_id: Tracer::generate_trace_id(),
                node_id: "node2".to_string(),
                input_summary: None,
            },
            Some("node2".to_string()),
            "Message 2".to_string(),
            serde_json::json!({}),
        );

        let node1_logs = tracer.get_logs_for_node(&"node1".to_string());
        assert_eq!(node1_logs.len(), 1);
    }

    #[test]
    fn test_log_entry_serialization() {
        let run_id = Tracer::generate_run_id();
        let entry = LogEntry {
            timestamp: Utc::now(),
            level: LogLevel::Info,
            event_type: TraceEvent::RunStarted {
                run_id: run_id.clone(),
                flow_name: "test".to_string(),
            },
            node_id: None,
            message: "Test".to_string(),
            metadata: serde_json::json!({}),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: LogEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.message, "Test");
    }

    #[test]
    fn test_tracer_export() {
        let mut tracer = Tracer::new();
        let run_id = Tracer::generate_run_id();

        tracer.log(
            LogLevel::Info,
            TraceEvent::RunStarted {
                run_id,
                flow_name: "test".to_string(),
            },
            None,
            "Test".to_string(),
            serde_json::json!({}),
        );

        let exported = tracer.export_logs().unwrap();
        assert!(exported.contains("Test"));
    }

    #[test]
    fn test_two_layer_tracing() {
        let mut tracer = Tracer::new();
        let run_id = Tracer::generate_run_id();

        // Run-level event
        tracer.log_run_event(
            TraceEvent::RunStarted {
                run_id: run_id.clone(),
                flow_name: "codegen".to_string(),
            },
            "Flow execution started".to_string(),
        );

        // Flow-level events
        let trace_id = Tracer::generate_trace_id();
        tracer.log_flow_event(
            TraceEvent::NodeStarted {
                run_id: run_id.clone(),
                trace_id: trace_id.clone(),
                node_id: "planner".to_string(),
                input_summary: None,
            },
            Some("planner".to_string()),
            "Node execution started".to_string(),
        );

        tracer.log_flow_event(
            TraceEvent::NodeCompleted {
                run_id,
                trace_id,
                node_id: "planner".to_string(),
                duration_ms: 150,
                output_summary: None,
                status: "success".to_string(),
            },
            Some("planner".to_string()),
            "Node execution completed".to_string(),
        );

        assert_eq!(tracer.get_logs().len(), 3);
    }

    #[test]
    fn test_generate_ids() {
        let run_id = Tracer::generate_run_id();
        let trace_id = Tracer::generate_trace_id();

        assert!(!run_id.is_empty());
        assert!(!trace_id.is_empty());
        assert_ne!(run_id, trace_id);
    }

    #[test]
    fn test_hierarchical_trace() {
        let trace_id = Tracer::generate_trace_id();
        let run_id = Tracer::generate_run_id();
        let work_context_id = Some("ctx-123".to_string());

        let mut tracer =
            Tracer::with_hierarchical_trace(trace_id.clone(), run_id.clone(), work_context_id);

        let hierarchical = tracer.get_hierarchical_trace();
        assert!(hierarchical.is_some());
        let trace = hierarchical.unwrap();
        assert_eq!(trace.trace_id, trace_id);
        assert_eq!(trace.flow_run_id, run_id);
        assert_eq!(trace.work_context_id, Some("ctx-123".to_string()));
        assert!(trace.node_runs.is_empty());
        assert!(trace.tool_calls.is_empty());
        assert!(trace.llm_calls.is_empty());
    }

    #[test]
    fn test_add_node_run() {
        let mut tracer = Tracer::with_hierarchical_trace(
            Tracer::generate_trace_id(),
            Tracer::generate_run_id(),
            None,
        );

        let node_run = NodeRun {
            node_id: "planner".to_string(),
            trace_id: Tracer::generate_trace_id(),
            input_summary: Some("plan request".to_string()),
            output_summary: Some("plan generated".to_string()),
            status: "success".to_string(),
            duration_ms: 100,
            error: None,
            started_at: Utc::now(),
            completed_at: Utc::now(),
        };

        tracer.add_node_run(node_run);

        let hierarchical = tracer.get_hierarchical_trace().unwrap();
        assert_eq!(hierarchical.node_runs.len(), 1);
        assert_eq!(hierarchical.node_runs[0].node_id, "planner");
    }

    #[test]
    fn test_add_tool_call() {
        let mut tracer = Tracer::with_hierarchical_trace(
            Tracer::generate_trace_id(),
            Tracer::generate_run_id(),
            None,
        );

        let tool_call = ToolCall {
            tool_name: "file_writer".to_string(),
            trace_id: Tracer::generate_trace_id(),
            args_hash: "hash123".to_string(),
            result_hash: "hash456".to_string(),
            success: true,
            duration_ms: 50,
            called_at: Utc::now(),
        };

        tracer.add_tool_call(tool_call);

        let hierarchical = tracer.get_hierarchical_trace().unwrap();
        assert_eq!(hierarchical.tool_calls.len(), 1);
        assert_eq!(hierarchical.tool_calls[0].tool_name, "file_writer");
    }

    #[test]
    fn test_add_llm_call() {
        let mut tracer = Tracer::with_hierarchical_trace(
            Tracer::generate_trace_id(),
            Tracer::generate_run_id(),
            None,
        );

        let llm_call = LlmCall {
            node_id: "planner".to_string(),
            trace_id: Tracer::generate_trace_id(),
            provider: "openai".to_string(),
            model: "gpt-4".to_string(),
            prompt_tokens: 100,
            completion_tokens: 200,
            latency_ms: 1500,
            error: None,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
        };

        tracer.add_llm_call(llm_call);

        let hierarchical = tracer.get_hierarchical_trace().unwrap();
        assert_eq!(hierarchical.llm_calls.len(), 1);
        assert_eq!(hierarchical.llm_calls[0].provider, "openai");
    }

    #[test]
    fn test_complete_hierarchical_trace() {
        let mut tracer = Tracer::with_hierarchical_trace(
            Tracer::generate_trace_id(),
            Tracer::generate_run_id(),
            None,
        );

        assert!(
            tracer
                .get_hierarchical_trace()
                .unwrap()
                .completed_at
                .is_none()
        );

        tracer.complete_hierarchical_trace();

        assert!(
            tracer
                .get_hierarchical_trace()
                .unwrap()
                .completed_at
                .is_some()
        );
    }

    #[test]
    fn test_summarize_value() {
        let short_value = serde_json::json!({"key": "value"});
        let short_summary = Tracer::summarize_value(&short_value, 100);
        assert!(short_summary.is_some());
        assert!(short_summary.unwrap().len() <= 100);

        let long_value = serde_json::json!({"key": "a".repeat(200)});
        let long_summary = Tracer::summarize_value(&long_value, 50);
        assert!(long_summary.is_some());
        assert!(long_summary.as_ref().unwrap().ends_with("..."));
        assert!(long_summary.as_ref().unwrap().len() <= 53); // 50 + "..."
    }

    #[test]
    fn test_compute_hash() {
        let value1 = serde_json::json!({"key": "value"});
        let value2 = serde_json::json!({"key": "value"});
        let value3 = serde_json::json!({"key": "different"});

        let hash1 = Tracer::compute_hash(&value1);
        let hash2 = Tracer::compute_hash(&value2);
        let hash3 = Tracer::compute_hash(&value3);

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }
}
