//! Logging & Tracing - structured logs and event timeline

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::flow::{NodeId, SharedState};

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
    },
    NodeCompleted {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        duration_ms: u64,
    },
    NodeFailed {
        run_id: RunId,
        trace_id: TraceId,
        node_id: NodeId,
        error: String,
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
    },
    ToolCompleted {
        run_id: RunId,
        trace_id: TraceId,
        tool_name: String,
        duration_ms: u64,
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

/// Tracer for structured logging and event timeline
#[derive(Debug)]
pub struct Tracer {
    logs: Vec<LogEntry>,
    timeline: Vec<TimelineEvent>,
    enabled: bool,
    min_level: LogLevel,
}

impl Tracer {
    pub fn new() -> Self {
        Self {
            logs: Vec::new(),
            timeline: Vec::new(),
            enabled: true,
            min_level: LogLevel::Info,
        }
    }

    pub fn with_level(min_level: LogLevel) -> Self {
        Self {
            logs: Vec::new(),
            timeline: Vec::new(),
            enabled: true,
            min_level,
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
}
