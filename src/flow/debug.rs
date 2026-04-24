//! Debug mode for step-by-step execution, state snapshots, and breakpoints

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::flow::{Flow, Node, NodeId, SharedState};

/// State snapshot for debugging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub timestamp: DateTime<Utc>,
    pub node_id: NodeId,
    pub state: SharedState,
    pub input: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub action: Option<String>,
}

/// Breakpoint configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breakpoint {
    pub node_id: NodeId,
    pub condition: Option<String>, // Optional condition expression
    pub hit_count: u32,
    pub enabled: bool,
}

/// Debug session for flow execution
pub struct DebugSession {
    flow: Flow,
    breakpoints: HashMap<NodeId, Breakpoint>,
    snapshots: Vec<StateSnapshot>,
    step_mode: StepMode,
    current_step: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepMode {
    Run,
    StepOver,
    StepInto,
    StepOut,
    Pause,
}

impl DebugSession {
    pub fn new(flow: Flow) -> Self {
        Self {
            flow,
            breakpoints: HashMap::new(),
            snapshots: Vec::new(),
            step_mode: StepMode::Run,
            current_step: 0,
        }
    }

    /// Add a breakpoint at a specific node
    pub fn add_breakpoint(&mut self, node_id: NodeId, condition: Option<String>) {
        self.breakpoints.insert(
            node_id.clone(),
            Breakpoint {
                node_id,
                condition,
                hit_count: 0,
                enabled: true,
            },
        );
    }

    /// Remove a breakpoint
    pub fn remove_breakpoint(&mut self, node_id: &NodeId) {
        self.breakpoints.remove(node_id);
    }

    /// Enable/disable a breakpoint
    pub fn toggle_breakpoint(&mut self, node_id: &NodeId) {
        if let Some(bp) = self.breakpoints.get_mut(node_id) {
            bp.enabled = !bp.enabled;
        }
    }

    /// Set step mode
    pub fn set_step_mode(&mut self, mode: StepMode) {
        self.step_mode = mode;
    }

    /// Get all snapshots
    pub fn get_snapshots(&self) -> &[StateSnapshot] {
        &self.snapshots
    }

    /// Get a specific snapshot by index
    pub fn get_snapshot(&self, index: usize) -> Option<&StateSnapshot> {
        self.snapshots.get(index)
    }

    /// Clear all snapshots
    pub fn clear_snapshots(&mut self) {
        self.snapshots.clear();
    }

    /// Check if execution should pause at a node
    fn should_pause(&mut self, node_id: &NodeId) -> bool {
        // Check breakpoints
        if let Some(bp) = self.breakpoints.get_mut(node_id) {
            if bp.enabled {
                bp.hit_count += 1;
                return true;
            }
        }

        // Check step mode
        match self.step_mode {
            StepMode::StepOver => {
                self.current_step += 1;
                self.current_step == 1
            }
            StepMode::StepInto => {
                self.current_step += 1;
                true
            }
            StepMode::Pause => true,
            _ => false,
        }
    }

    /// Execute flow with debugging
    pub async fn run_debug(&mut self, state: &mut SharedState) -> Result<DebugResult> {
        let mut current = self.flow.start_node().clone();
        let mut snapshots = Vec::new();
        let mut paused = false;
        let mut pause_reason = None;

        loop {
            // Check if we should pause before execution
            if self.should_pause(&current) {
                paused = true;
                pause_reason = Some(format!("Paused at node: {}", current));
                break;
            }

            let node = self
                .flow
                .get_node(&current)
                .ok_or_else(|| anyhow::anyhow!("Node not found: {}", current))?;

            // Prepare input
            let input = node.prep(state)?;

            // Execute
            let output = node.exec(input.clone()).await?;

            // Post-process
            let action = node.post(state, output.clone());

            // Take snapshot
            snapshots.push(StateSnapshot {
                timestamp: Utc::now(),
                node_id: current.clone(),
                state: state.clone(),
                input,
                output: Some(output),
                action: Some(action.clone()),
            });

            // Find next node
            match self.flow.get_next_node(&current, &action) {
                Some(next) => current = next.clone(),
                None => break,
            }
        }

        self.snapshots.extend(snapshots);
        Ok(DebugResult {
            snapshots: self.snapshots.clone(),
            paused,
            pause_reason,
        })
    }

    /// Resume execution after pause
    pub async fn resume(&mut self, state: &mut SharedState) -> Result<DebugResult> {
        self.step_mode = StepMode::Run;
        self.run_debug(state).await
    }
}

/// Result of a debug execution
#[derive(Debug, Clone)]
pub struct DebugResult {
    pub snapshots: Vec<StateSnapshot>,
    pub paused: bool,
    pub pause_reason: Option<String>,
}

/// Debug wrapper for Flow to add debug capabilities
pub struct DebugFlow {
    flow: Flow,
    debug_session: Arc<Mutex<DebugSession>>,
}

impl DebugFlow {
    pub fn new(flow: Flow) -> Self {
        let debug_session = Arc::new(Mutex::new(DebugSession::new(flow.clone())));
        Self {
            flow,
            debug_session,
        }
    }

    pub fn debug_session(&self) -> Arc<Mutex<DebugSession>> {
        self.debug_session.clone()
    }

    pub async fn run_debug(&self, state: &mut SharedState) -> Result<DebugResult> {
        let mut session = self
            .debug_session
            .lock()
            .map_err(|e| anyhow::anyhow!("Mutex lock failed: {}", e))?;
        session.run_debug(state).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{FlowBuilder, NodeConfig};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct TestNode {
        id: String,
    }

    impl TestNode {
        fn new(id: String) -> Self {
            Self { id }
        }
    }

    #[async_trait]
    impl Node for TestNode {
        fn id(&self) -> String {
            self.id.clone()
        }

        fn prep(&self, _state: &SharedState) -> Result<serde_json::Value> {
            Ok(serde_json::json!({}))
        }

        async fn exec(&self, _input: serde_json::Value) -> Result<serde_json::Value> {
            Ok(serde_json::json!({ "executed": true }))
        }

        fn post(&self, _state: &mut SharedState, _output: serde_json::Value) -> String {
            "continue".to_string()
        }

        fn config(&self) -> NodeConfig {
            NodeConfig::default()
        }
    }

    #[tokio::test]
    async fn test_debug_session_breakpoint() {
        let node1 = Arc::new(TestNode::new("node1".to_string()));
        let node2 = Arc::new(TestNode::new("node2".to_string()));

        let flow = FlowBuilder::new()
            .start("node1".to_string())
            .add_node("node1".to_string(), node1)
            .add_node("node2".to_string(), node2)
            .chain("node1".to_string(), "node2".to_string())
            .build()
            .unwrap();

        let mut debug = DebugSession::new(flow);
        debug.add_breakpoint("node2".to_string(), None);

        let mut state = SharedState::new();
        let result = debug.run_debug(&mut state).await.unwrap();

        assert!(result.paused);
        assert!(result.pause_reason.is_some());
        assert!(!debug.snapshots.is_empty());
    }

    #[tokio::test]
    async fn test_debug_session_step_mode() {
        let node1 = Arc::new(TestNode::new("node1".to_string()));
        let node2 = Arc::new(TestNode::new("node2".to_string()));

        let flow = FlowBuilder::new()
            .start("node1".to_string())
            .add_node("node1".to_string(), node1)
            .add_node("node2".to_string(), node2)
            .chain("node1".to_string(), "node2".to_string())
            .build()
            .unwrap();

        let mut debug = DebugSession::new(flow);
        debug.set_step_mode(StepMode::StepInto);

        let mut state = SharedState::new();
        let result = debug.run_debug(&mut state).await.unwrap();

        assert!(result.paused);
    }

    #[test]
    fn test_state_snapshot_serialization() {
        let snapshot = StateSnapshot {
            timestamp: Utc::now(),
            node_id: "test".to_string(),
            state: SharedState::new(),
            input: serde_json::json!({}),
            output: Some(serde_json::json!({})),
            action: Some("continue".to_string()),
        };

        let json = serde_json::to_string(&snapshot).unwrap();
        let parsed: StateSnapshot = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.node_id, "test");
    }
}
