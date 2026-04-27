//! Core types for the Flow execution engine.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::budget::ExecutionBudget;

/// Node identifier - unique string for each node in a flow
pub type NodeId = String;

/// Action returned by node post-processing to determine next transition
pub type Action = String;

/// Input to a node - derived from SharedState
pub type Input = serde_json::Value;

/// Output from a node - transient, not persisted to SharedState
pub type Output = serde_json::Value;

/// SharedState - explicit state management with typed fields
///
/// Convention:
/// - input: Original user input and parameters
/// - context: Retrieved from memory, long-term context
/// - working: Intermediate computation results
/// - output: Final results to return to user
/// - meta: Metadata (timestamps, execution IDs, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SharedState {
    pub input: HashMap<String, serde_json::Value>,
    pub context: HashMap<String, serde_json::Value>,
    pub working: HashMap<String, serde_json::Value>,
    pub output: HashMap<String, serde_json::Value>,
    pub meta: HashMap<String, serde_json::Value>,
}

impl SharedState {
    /// Create a new empty SharedState
    pub fn new() -> Self {
        Self::default()
    }

    /// Create SharedState with initial input
    pub fn with_input(input: HashMap<String, serde_json::Value>) -> Self {
        Self {
            input,
            ..Default::default()
        }
    }

    /// Get a value from input section
    pub fn get_input(&self, key: &str) -> Option<&serde_json::Value> {
        self.input.get(key)
    }

    /// Set a value in input section
    pub fn set_input(&mut self, key: String, value: serde_json::Value) {
        self.input.insert(key, value);
    }

    /// Get a value from context section
    pub fn get_context(&self, key: &str) -> Option<&serde_json::Value> {
        self.context.get(key)
    }

    /// Set a value in context section
    pub fn set_context(&mut self, key: String, value: serde_json::Value) {
        self.context.insert(key, value);
    }

    /// Get a value from working section
    pub fn get_working(&self, key: &str) -> Option<&serde_json::Value> {
        self.working.get(key)
    }

    /// Set a value in working section
    pub fn set_working(&mut self, key: String, value: serde_json::Value) {
        self.working.insert(key, value);
    }

    /// Get a value from output section
    pub fn get_output(&self, key: &str) -> Option<&serde_json::Value> {
        self.output.get(key)
    }

    /// Set a value in output section
    pub fn set_output(&mut self, key: String, value: serde_json::Value) {
        self.output.insert(key, value);
    }

    /// Get a value from meta section
    pub fn get_meta(&self, key: &str) -> Option<&serde_json::Value> {
        self.meta.get(key)
    }

    /// Set a value in meta section
    pub fn set_meta(&mut self, key: String, value: serde_json::Value) {
        self.meta.insert(key, value);
    }

    /// Set the execution budget in meta
    pub fn set_budget(&mut self, budget: ExecutionBudget) {
        let budget_json = serde_json::to_value(budget).unwrap_or(serde_json::json!({}));
        self.set_meta("budget".to_string(), budget_json);
    }

    /// Get the execution budget from meta
    pub fn get_budget(&self) -> Option<ExecutionBudget> {
        self.get_meta("budget")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Set the personality mode in meta
    pub fn set_personality_mode(&mut self, mode: &str) {
        self.set_meta("personality_mode".to_string(), serde_json::json!(mode));
    }

    /// Get the personality mode from meta
    pub fn get_personality_mode(&self) -> Option<String> {
        self.get_meta("personality_mode")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    }

    /// Clear the working section (useful between node executions)
    pub fn clear_working(&mut self) {
        self.working.clear();
    }

    /// Get all outputs as a JSON value
    pub fn get_all_outputs(&self) -> serde_json::Value {
        serde_json::to_value(&self.output).unwrap_or(serde_json::json!({}))
    }

    /// Merge another SharedState into this one (other takes precedence)
    pub fn merge(&mut self, other: SharedState) {
        for (k, v) in other.input {
            self.input.insert(k, v);
        }
        for (k, v) in other.context {
            self.context.insert(k, v);
        }
        for (k, v) in other.working {
            self.working.insert(k, v);
        }
        for (k, v) in other.output {
            self.output.insert(k, v);
        }
        for (k, v) in other.meta {
            self.meta.insert(k, v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_state_creation() {
        let state = SharedState::new();
        assert!(state.input.is_empty());
        assert!(state.context.is_empty());
        assert!(state.working.is_empty());
        assert!(state.output.is_empty());
        assert!(state.meta.is_empty());
    }

    #[test]
    fn test_shared_state_with_input() {
        let mut input = HashMap::new();
        input.insert("task".to_string(), serde_json::json!("test task"));

        let state = SharedState::with_input(input.clone());
        assert_eq!(
            state.get_input("task"),
            Some(&serde_json::json!("test task"))
        );
    }

    #[test]
    fn test_shared_state_getters_setters() {
        let mut state = SharedState::new();

        state.set_input("key1".to_string(), serde_json::json!("value1"));
        assert_eq!(state.get_input("key1"), Some(&serde_json::json!("value1")));

        state.set_context("key2".to_string(), serde_json::json!("value2"));
        assert_eq!(
            state.get_context("key2"),
            Some(&serde_json::json!("value2"))
        );

        state.set_working("key3".to_string(), serde_json::json!("value3"));
        assert_eq!(
            state.get_working("key3"),
            Some(&serde_json::json!("value3"))
        );

        state.set_output("key4".to_string(), serde_json::json!("value4"));
        assert_eq!(state.get_output("key4"), Some(&serde_json::json!("value4")));

        state.set_meta("key5".to_string(), serde_json::json!("value5"));
        assert_eq!(state.get_meta("key5"), Some(&serde_json::json!("value5")));
    }

    #[test]
    fn test_shared_state_clear_working() {
        let mut state = SharedState::new();
        state.set_working("key1".to_string(), serde_json::json!("value1"));
        state.set_working("key2".to_string(), serde_json::json!("value2"));

        assert_eq!(state.working.len(), 2);
        state.clear_working();
        assert!(state.working.is_empty());
    }

    #[test]
    fn test_shared_state_merge() {
        let mut state1 = SharedState::new();
        state1.set_input("key1".to_string(), serde_json::json!("old"));

        let mut state2 = SharedState::new();
        state2.set_input("key1".to_string(), serde_json::json!("new"));
        state2.set_input("key2".to_string(), serde_json::json!("value2"));

        state1.merge(state2);

        assert_eq!(state1.get_input("key1"), Some(&serde_json::json!("new")));
        assert_eq!(state1.get_input("key2"), Some(&serde_json::json!("value2")));
    }

    #[test]
    fn test_shared_state_serialization() {
        let mut state = SharedState::new();
        state.set_input("task".to_string(), serde_json::json!("test"));

        let serialized = serde_json::to_string(&state).unwrap();
        let deserialized: SharedState = serde_json::from_str(&serialized).unwrap();

        assert_eq!(
            deserialized.get_input("task"),
            Some(&serde_json::json!("test"))
        );
    }
}
