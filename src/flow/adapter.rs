//! Migration adapter: wraps existing Agent trait to implement Node trait
//!
//! This module provides a bridge between the old Agent-based system and the new
//! Flow-based system, enabling gradual migration.

use crate::agents::Agent;
use crate::flow::{Action, Input, Node, NodeConfig, Output, SharedState};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

/// AgentNode - adapter that wraps an Agent to implement the Node trait
pub struct AgentNode {
    id: String,
    agent: Arc<dyn Agent>,
    config: NodeConfig,
}

impl AgentNode {
    /// Create a new AgentNode wrapping an Agent
    pub fn new(id: String, agent: Arc<dyn Agent>) -> Self {
        Self {
            id,
            agent,
            config: NodeConfig::default(),
        }
    }

    /// Create an AgentNode with custom configuration
    pub fn with_config(id: String, agent: Arc<dyn Agent>, config: NodeConfig) -> Self {
        Self { id, agent, config }
    }

    /// Get the inner agent
    pub fn agent(&self) -> &Arc<dyn Agent> {
        &self.agent
    }
}

#[async_trait]
impl Node for AgentNode {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn prep(&self, state: &SharedState) -> Result<Input> {
        // Extract input from SharedState based on node type
        let node_name = self.agent.name();
        let input = match node_name {
            "planner" => {
                // Planner gets the original task
                state
                    .get_input("task")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            }
            "builder" => {
                // Coder gets the plan from meta (stored by planner)
                state
                    .get_meta("plan")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        // Fallback to input if plan not in meta
                        state
                            .get_input("task")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                    })
                    .to_string()
            }
            "reviewer" => {
                // Reviewer gets the generated output from meta (stored by coder)
                state
                    .get_meta("generated_output")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        // Fallback to plan if generated_output not in meta
                        state
                            .get_meta("plan")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                    })
                    .to_string()
            }
            _ => {
                // Default: try to get from input
                state
                    .get_input("task")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            }
        };

        Ok(serde_json::json!({ "input": input }))
    }

    async fn exec(&self, input: Input) -> Result<Output> {
        // Extract the input string
        let input_str = input.get("input").and_then(|v| v.as_str()).unwrap_or("");

        // Run the agent
        let result = self.agent.run(input_str).await?;

        Ok(serde_json::json!({ "result": result }))
    }

    fn post(&self, state: &mut SharedState, output: Output) -> Action {
        // Store the result in the appropriate section based on the node type
        let node_name = self.agent.name();
        let result = output.get("result").and_then(|v| v.as_str()).unwrap_or("");

        match node_name {
            "planner" => {
                state.set_meta("plan".to_string(), serde_json::json!(result));
            }
            "builder" => {
                state.set_meta("generated_output".to_string(), serde_json::json!(result));
            }
            "reviewer" => {
                state.set_meta("review".to_string(), serde_json::json!(result));
            }
            _ => {
                state.set_working(format!("{}_output", node_name), serde_json::json!(result));
            }
        }

        "continue".to_string()
    }

    fn config(&self) -> NodeConfig {
        self.config.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // Mock agent for testing
    struct MockAgent {
        name: String,
        output: String,
    }

    #[async_trait::async_trait]
    impl Agent for MockAgent {
        fn name(&self) -> &str {
            &self.name
        }

        async fn run(&self, _input: &str) -> Result<String> {
            Ok(self.output.clone())
        }
    }

    #[tokio::test]
    async fn test_agent_node_adapter() {
        let agent = Arc::new(MockAgent {
            name: "test_agent".to_string(),
            output: "test output".to_string(),
        });

        let node = AgentNode::new("test_node".to_string(), agent);

        assert_eq!(node.id(), "test_node");
        assert_eq!(node.agent().name(), "test_agent");
    }

    #[tokio::test]
    async fn test_agent_node_execution() {
        let agent = Arc::new(MockAgent {
            name: "test_agent".to_string(),
            output: "test output".to_string(),
        });

        let node = AgentNode::new("test_node".to_string(), agent);

        let mut state = SharedState::new();
        state.set_input("task".to_string(), serde_json::json!("test task"));

        let input = node.prep(&state).unwrap();
        let output = node.exec(input).await.unwrap();
        node.post(&mut state, output);

        assert_eq!(
            state.get_working("test_agent_output"),
            Some(&serde_json::json!("test output"))
        );
    }

    #[tokio::test]
    async fn test_agent_node_planner() {
        let agent = Arc::new(MockAgent {
            name: "planner".to_string(),
            output: "plan content".to_string(),
        });

        let node = AgentNode::new("planner_node".to_string(), agent);

        let mut state = SharedState::new();
        state.set_input("task".to_string(), serde_json::json!("test task"));

        let input = node.prep(&state).unwrap();
        let output = node.exec(input).await.unwrap();
        node.post(&mut state, output);

        assert_eq!(
            state.get_meta("plan"),
            Some(&serde_json::json!("plan content"))
        );
    }

    #[tokio::test]
    async fn test_agent_node_builder() {
        let agent = Arc::new(MockAgent {
            name: "builder".to_string(),
            output: "code content".to_string(),
        });

        let node = AgentNode::new("builder_node".to_string(), agent);

        let mut state = SharedState::new();
        state.set_input("task".to_string(), serde_json::json!("test task"));

        let input = node.prep(&state).unwrap();
        let output = node.exec(input).await.unwrap();
        node.post(&mut state, output);

        assert_eq!(
            state.get_meta("generated_output"),
            Some(&serde_json::json!("code content"))
        );
    }

    #[tokio::test]
    async fn test_agent_node_reviewer() {
        let agent = Arc::new(MockAgent {
            name: "reviewer".to_string(),
            output: "review content".to_string(),
        });

        let node = AgentNode::new("reviewer_node".to_string(), agent);

        let mut state = SharedState::new();
        state.set_input("task".to_string(), serde_json::json!("test task"));

        let input = node.prep(&state).unwrap();
        let output = node.exec(input).await.unwrap();
        node.post(&mut state, output);

        assert_eq!(
            state.get_meta("review"),
            Some(&serde_json::json!("review content"))
        );
    }
}
