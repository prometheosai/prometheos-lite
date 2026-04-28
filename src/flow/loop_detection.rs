//! Loop detection for preventing runaway flows

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// Configuration for loop detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopDetectionConfig {
    /// Maximum times a single node can be repeated
    pub max_repeated_node: usize,
    /// Maximum times a transition cycle can be repeated
    pub max_repeated_transition: usize,
    /// Maximum times a tool call with same args can be repeated
    pub max_repeated_tool_call: usize,
}

impl Default for LoopDetectionConfig {
    fn default() -> Self {
        Self {
            max_repeated_node: 10,
            max_repeated_transition: 5,
            max_repeated_tool_call: 3,
        }
    }
}

/// Loop detector for tracking execution patterns
#[derive(Debug, Clone)]
pub struct LoopDetector {
    config: LoopDetectionConfig,
    node_counts: HashMap<String, usize>,
    transition_counts: HashMap<String, usize>,
    tool_call_counts: HashMap<String, usize>,
    recent_transitions: VecDeque<String>,
}

impl LoopDetector {
    /// Create a new loop detector with default config
    pub fn new() -> Self {
        Self::with_config(LoopDetectionConfig::default())
    }

    /// Create a new loop detector with custom config
    pub fn with_config(config: LoopDetectionConfig) -> Self {
        Self {
            config,
            node_counts: HashMap::new(),
            transition_counts: HashMap::new(),
            tool_call_counts: HashMap::new(),
            recent_transitions: VecDeque::with_capacity(10),
        }
    }

    /// Record a node execution
    pub fn record_node(&mut self, node_id: &str) -> Result<(), String> {
        let count = self.node_counts.entry(node_id.to_string()).or_insert(0);
        *count += 1;

        if *count > self.config.max_repeated_node {
            return Err(format!(
                "Node '{}' repeated {} times (max: {})",
                node_id, count, self.config.max_repeated_node
            ));
        }

        Ok(())
    }

    /// Record a transition
    pub fn record_transition(&mut self, from: &str, to: &str) -> Result<(), String> {
        let transition_key = format!("{}->{}", from, to);
        let count = self.transition_counts.entry(transition_key.clone()).or_insert(0);
        *count += 1;

        if *count > self.config.max_repeated_transition {
            return Err(format!(
                "Transition '{}' repeated {} times (max: {})",
                transition_key, count, self.config.max_repeated_transition
            ));
        }

        // Track recent transitions for cycle detection
        self.recent_transitions.push_back(transition_key);
        if self.recent_transitions.len() > 10 {
            self.recent_transitions.pop_front();
        }

        Ok(())
    }

    /// Record a tool call
    pub fn record_tool_call(&mut self, tool_name: &str, args_hash: &str) -> Result<(), String> {
        let call_key = format!("{}:{}", tool_name, args_hash);
        let count = self.tool_call_counts.entry(call_key.clone()).or_insert(0);
        *count += 1;

        if *count > self.config.max_repeated_tool_call {
            return Err(format!(
                "Tool call '{}' repeated {} times (max: {})",
                call_key, count, self.config.max_repeated_tool_call
            ));
        }

        Ok(())
    }

    /// Check if a transition cycle is detected
    pub fn detect_cycle(&self) -> bool {
        if self.recent_transitions.len() < 3 {
            return false;
        }

        // Simple cycle detection: check if the same sequence repeats
        let transitions: Vec<&String> = self.recent_transitions.iter().collect();
        let last = transitions.last().unwrap();
        let count = transitions.iter().filter(|&&t| t == *last).count();

        count >= 3
    }

    /// Reset the detector
    pub fn reset(&mut self) {
        self.node_counts.clear();
        self.transition_counts.clear();
        self.tool_call_counts.clear();
        self.recent_transitions.clear();
    }

    /// Get the current configuration
    pub fn config(&self) -> &LoopDetectionConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loop_detector_node_repetition() {
        let config = LoopDetectionConfig {
            max_repeated_node: 3,
            max_repeated_transition: 5,
            max_repeated_tool_call: 3,
        };
        let mut detector = LoopDetector::with_config(config);

        detector.record_node("node1").unwrap();
        detector.record_node("node1").unwrap();
        detector.record_node("node1").unwrap();

        let result = detector.record_node("node1");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("repeated 4 times"));
    }

    #[test]
    fn test_loop_detector_transition_repetition() {
        let config = LoopDetectionConfig {
            max_repeated_node: 10,
            max_repeated_transition: 2,
            max_repeated_tool_call: 3,
        };
        let mut detector = LoopDetector::with_config(config);

        detector.record_transition("node1", "node2").unwrap();
        detector.record_transition("node1", "node2").unwrap();

        let result = detector.record_transition("node1", "node2");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("repeated 3 times"));
    }

    #[test]
    fn test_loop_detector_tool_call_repetition() {
        let config = LoopDetectionConfig {
            max_repeated_node: 10,
            max_repeated_transition: 5,
            max_repeated_tool_call: 2,
        };
        let mut detector = LoopDetector::with_config(config);

        detector.record_tool_call("echo", "hash123").unwrap();
        detector.record_tool_call("echo", "hash123").unwrap();

        let result = detector.record_tool_call("echo", "hash123");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("repeated 3 times"));
    }

    #[test]
    fn test_loop_detector_reset() {
        let mut detector = LoopDetector::new();
        detector.record_node("node1").unwrap();
        detector.record_transition("node1", "node2").unwrap();

        detector.reset();

        assert!(detector.node_counts.is_empty());
        assert!(detector.transition_counts.is_empty());
    }

    #[test]
    fn test_loop_detector_cycle_detection() {
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
}
