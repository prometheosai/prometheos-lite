//! Core types for the Flow execution engine.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::budget::{BudgetGuard, BudgetUsage, ExecutionBudget};
use std::sync::{Arc, Mutex};

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

    /// Set the run ID in meta (one per flow execution)
    pub fn set_run_id(&mut self, run_id: &str) {
        self.set_meta("run_id".to_string(), serde_json::json!(run_id));
    }

    /// Get the run ID from meta
    pub fn get_run_id(&self) -> Option<String> {
        self.get_meta("run_id")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    }

    /// Set the trace ID in meta (one per flow execution)
    pub fn set_trace_id(&mut self, trace_id: &str) {
        self.set_meta("trace_id".to_string(), serde_json::json!(trace_id));
    }

    /// Get the trace ID from meta
    pub fn get_trace_id(&self) -> Option<String> {
        self.get_meta("trace_id")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    }

    /// Store a BudgetGuard reference in meta (as a JSON report)
    /// Note: BudgetGuard is not serializable, so we store its report.
    /// The actual guard lives in Flow.budget_guard and is checked at the flow loop level.
    /// Nodes should use check_budget_* methods on SharedState to record against
    /// the guard stored in the flow execution context.
    pub fn set_budget_report(&mut self, report: serde_json::Value) {
        self.set_meta("budget_report".to_string(), report);
    }

    /// Get the budget report from meta
    pub fn get_budget_report(&self) -> Option<&serde_json::Value> {
        self.get_meta("budget_report")
    }

    /// Store BudgetGuard reference in meta for nodes to access
    pub fn set_budget_guard(&mut self, guard: Arc<Mutex<BudgetGuard>>) {
        // Store as a JSON-serializable placeholder - actual guard access is via separate method
        // The guard itself is stored in a separate map keyed by a known ID
        let report = if let Ok(g) = guard.lock() {
            g.get_report()
        } else {
            serde_json::json!({})
        };
        self.set_budget_report(report);
    }

    /// Get the BudgetGuard from meta (if set)
    /// This requires the guard to have been stored via set_budget_guard
    pub fn get_budget_guard(&self) -> Option<Arc<Mutex<BudgetGuard>>> {
        // This would need the guard to be stored separately
        // For now, return None - the actual implementation would store the guard
        // in a thread-local or pass it through execution context
        None
    }

    /// Check if an LLM call is allowed under current budget
    /// Returns error if budget would be exceeded
    pub fn check_llm_budget(&self) -> anyhow::Result<()> {
        if let Some(report) = self.get_budget_report() {
            let current: u64 = report["usage"]["llm_calls"].as_u64().unwrap_or(0);
            let limit: u64 = report["budget"]["max_llm_calls"]
                .as_u64()
                .unwrap_or(u64::MAX);
            if current >= limit {
                anyhow::bail!("LLM call budget exceeded: {} >= {}", current, limit);
            }
        }
        Ok(())
    }

    /// Check if a tool call is allowed under current budget
    pub fn check_tool_budget(&self) -> anyhow::Result<()> {
        if let Some(report) = self.get_budget_report() {
            let current: u64 = report["usage"]["tool_calls"].as_u64().unwrap_or(0);
            let limit: u64 = report["budget"]["max_tool_calls"]
                .as_u64()
                .unwrap_or(u64::MAX);
            if current >= limit {
                anyhow::bail!("Tool call budget exceeded: {} >= {}", current, limit);
            }
        }
        Ok(())
    }

    /// Check if memory read is allowed under current budget
    pub fn check_memory_read_budget(&self) -> anyhow::Result<()> {
        if let Some(report) = self.get_budget_report() {
            let current: u64 = report["usage"]["memory_reads"].as_u64().unwrap_or(0);
            let limit: u64 = report["budget"]["max_memory_reads"]
                .as_u64()
                .unwrap_or(u64::MAX);
            if current >= limit {
                anyhow::bail!("Memory read budget exceeded: {} >= {}", current, limit);
            }
        }
        Ok(())
    }

    /// Check if memory write is allowed under current budget
    pub fn check_memory_write_budget(&self) -> anyhow::Result<()> {
        if let Some(report) = self.get_budget_report() {
            let current: u64 = report["usage"]["memory_writes"].as_u64().unwrap_or(0);
            let limit: u64 = report["budget"]["max_memory_writes"]
                .as_u64()
                .unwrap_or(u64::MAX);
            if current >= limit {
                anyhow::bail!("Memory write budget exceeded: {} >= {}", current, limit);
            }
        }
        Ok(())
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
