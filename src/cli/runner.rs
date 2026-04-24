//! CLI Runner for flow execution and flow file loading

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use prometheos_lite::flow::{Flow, FlowBuilder, Node, SharedState};

/// Flow file format for serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowFile {
    pub name: String,
    pub description: Option<String>,
    pub start_node: String,
    pub nodes: Vec<NodeDefinition>,
    pub transitions: Vec<TransitionDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDefinition {
    pub id: String,
    pub node_type: String,
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionDefinition {
    pub from: String,
    pub action: String,
    pub to: String,
}

/// CLI Runner for executing flows
pub struct FlowRunner {
    flow: Flow,
}

impl FlowRunner {
    /// Create a new FlowRunner from a Flow
    pub fn new(flow: Flow) -> Self {
        Self { flow }
    }

    /// Load a flow from a JSON file
    pub fn from_json_file(path: PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read flow file: {}", path.display()))?;

        let flow_file: FlowFile =
            serde_json::from_str(&content).context("Failed to parse flow file")?;

        let flow = Self::build_flow_from_file(flow_file)?;
        Ok(Self::new(flow))
    }

    /// Build a Flow from a FlowFile
    fn build_flow_from_file(file: FlowFile) -> Result<Flow> {
        let mut builder = FlowBuilder::new().start(file.start_node.clone());

        // Note: This is a simplified version. In a real implementation,
        // you would need a node factory to create nodes based on node_type
        // For now, we'll use placeholder nodes
        for node_def in &file.nodes {
            let node = Arc::new(PlaceholderNode::new(node_def.id.clone()));
            builder = builder.add_node(node_def.id.clone(), node);
        }

        // Add transitions
        for transition in &file.transitions {
            builder = builder.add_transition(
                transition.from.clone(),
                transition.action.clone(),
                transition.to.clone(),
            );
        }

        builder.build().context("Failed to build flow from file")
    }

    /// Execute the flow with the given state
    pub async fn run(&mut self, state: &mut SharedState) -> Result<()> {
        self.flow.run(state).await
    }

    /// Execute the flow with initial input
    pub async fn run_with_input(&mut self, input: serde_json::Value) -> Result<SharedState> {
        let mut state = SharedState::new();
        state.set_input("user_input".to_string(), input);
        self.run(&mut state).await?;
        Ok(state)
    }
}

/// Placeholder node for flow file loading (to be replaced with actual node factory)
struct PlaceholderNode {
    id: String,
}

impl PlaceholderNode {
    fn new(id: String) -> Self {
        Self { id }
    }
}

#[async_trait::async_trait]
impl Node for PlaceholderNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn prep(&self, _state: &SharedState) -> Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }

    async fn exec(&self, _input: serde_json::Value) -> Result<serde_json::Value> {
        Ok(serde_json::json!({ "placeholder": true }))
    }

    fn post(&self, _state: &mut SharedState, _output: serde_json::Value) -> String {
        "continue".to_string()
    }

    fn config(&self) -> prometheos_lite::flow::NodeConfig {
        prometheos_lite::flow::NodeConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_file_serialization() {
        let flow_file = FlowFile {
            name: "test_flow".to_string(),
            description: Some("A test flow".to_string()),
            start_node: "node1".to_string(),
            nodes: vec![NodeDefinition {
                id: "node1".to_string(),
                node_type: "placeholder".to_string(),
                config: None,
            }],
            transitions: vec![],
        };

        let json = serde_json::to_string(&flow_file).unwrap();
        let parsed: FlowFile = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "test_flow");
        assert_eq!(parsed.start_node, "node1");
    }

    #[tokio::test]
    async fn test_flow_runner() {
        let mut builder = FlowBuilder::new();
        let node = Arc::new(PlaceholderNode::new("test".to_string()));
        builder.start("test".to_string());
        builder.add_node("test".to_string(), node);

        let flow = builder.build().unwrap();
        let mut runner = FlowRunner::new(flow);

        let mut state = SharedState::new();
        state.set_input("test".to_string(), serde_json::json!("value"));

        let result = runner.run(&mut state).await;
        assert!(result.is_ok());
    }
}
