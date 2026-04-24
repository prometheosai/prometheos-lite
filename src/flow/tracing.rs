//! Logging & Tracing - structured logs and event timeline

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

use crate::flow::{NodeId, SharedState};

/// Event types for tracing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventType {
    FlowStart,
    FlowEnd,
    NodeStart,
    NodeEnd,
    NodeError,
    StateChange,
    Transition,
    Custom(String),
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
    pub event_type: EventType,
    pub node_id: Option<NodeId>,
    pub message: String,
    pub metadata: serde_json::Value,
}

/// Timeline event for execution tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: EventType,
    pub node_id: Option<NodeId>,
    pub duration_ms: Option<u64>,
    pub details: serde_json::Value,
}

/// Tracer for structured logging and event timeline
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
    pub fn log(&mut self, level: LogLevel, event_type: EventType, node_id: Option<NodeId>, message: String, metadata: serde_json::Value) {
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
    pub fn add_timeline_event(&mut self, event_type: EventType, node_id: Option<NodeId>, duration_ms: Option<u64>, details: serde_json::Value) {
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
        self.logs.iter().filter(|log| log.node_id.as_ref() == Some(node_id)).collect()
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
        serde_json::to_string_pretty(&self.logs).map_err(|e| anyhow::anyhow!("Failed to export logs: {}", e))
    }

    /// Export timeline as JSON
    pub fn export_timeline(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.timeline).map_err(|e| anyhow::anyhow!("Failed to export timeline: {}", e))
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
            tracer.log($level, $event_type, $node_id, $message.to_string(), serde_json::json!({}));
        }
    };
    ($tracer:expr, $level:expr, $event_type:expr, $node_id:expr, $message:expr, $metadata:expr) => {
        if let Ok(mut tracer) = $tracer.lock() {
            tracer.log($level, $event_type, $node_id, $message.to_string(), $metadata);
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
        
        tracer.log(
            LogLevel::Info,
            EventType::NodeStart,
            Some("node1".to_string()),
            "Starting node".to_string(),
            serde_json::json!({}),
        );

        assert_eq!(tracer.get_logs().len(), 1);
        assert_eq!(tracer.get_logs()[0].level, LogLevel::Info);
    }

    #[test]
    fn test_tracer_level_filtering() {
        let mut tracer = Tracer::with_level(LogLevel::Warning);
        
        tracer.log(
            LogLevel::Debug,
            EventType::NodeStart,
            Some("node1".to_string()),
            "Debug message".to_string(),
            serde_json::json!({}),
        );
        
        tracer.log(
            LogLevel::Warning,
            EventType::NodeStart,
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
        
        tracer.add_timeline_event(
            EventType::NodeStart,
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
        
        tracer.log(
            LogLevel::Info,
            EventType::NodeStart,
            Some("node1".to_string()),
            "Message 1".to_string(),
            serde_json::json!({}),
        );
        
        tracer.log(
            LogLevel::Info,
            EventType::NodeStart,
            Some("node2".to_string()),
            "Message 2".to_string(),
            serde_json::json!({}),
        );

        let node1_logs = tracer.get_logs_for_node(&"node1".to_string());
        assert_eq!(node1_logs.len(), 1);
    }

    #[test]
    fn test_log_entry_serialization() {
        let entry = LogEntry {
            timestamp: Utc::now(),
            level: LogLevel::Info,
            event_type: EventType::NodeStart,
            node_id: Some("node1".to_string()),
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
        
        tracer.log(
            LogLevel::Info,
            EventType::NodeStart,
            Some("node1".to_string()),
            "Test".to_string(),
            serde_json::json!({}),
        );

        let exported = tracer.export_logs().unwrap();
        assert!(exported.contains("Test"));
    }
}
