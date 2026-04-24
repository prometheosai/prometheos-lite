//! Node trait and configuration for flow execution.

use crate::flow::{Action, Input, NodeId, Output, SharedState};
use anyhow::Result;
use async_trait::async_trait;

/// Configuration for a node's execution behavior
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Number of retry attempts on failure
    pub retries: u8,
    /// Delay between retries in milliseconds
    pub retry_delay_ms: u64,
    /// Optional timeout for node execution in milliseconds
    pub timeout_ms: Option<u64>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            retries: 3,
            retry_delay_ms: 100,
            timeout_ms: Some(300_000), // 5 minutes default
        }
    }
}

impl NodeConfig {
    /// Create a new NodeConfig with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the number of retries
    pub fn with_retries(mut self, retries: u8) -> Self {
        self.retries = retries;
        self
    }

    /// Set the retry delay in milliseconds
    pub fn with_retry_delay_ms(mut self, delay_ms: u64) -> Self {
        self.retry_delay_ms = delay_ms;
        self
    }

    /// Set the timeout in milliseconds
    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Disable timeout
    pub fn without_timeout(mut self) -> Self {
        self.timeout_ms = None;
        self
    }
}

/// Node trait - the fundamental execution unit in a Flow
///
/// Lifecycle:
/// 1. prep() - Prepare input from SharedState
/// 2. exec() - Execute with input, produce output
/// 3. post() - Process output, update SharedState, return Action
#[async_trait]
pub trait Node: Send + Sync {
    /// Unique identifier for this node
    fn id(&self) -> NodeId;

    /// Prepare input from SharedState
    fn prep(&self, state: &SharedState) -> Result<Input>;

    /// Execute with prepared input, produce output
    async fn exec(&self, input: Input) -> Result<Output>;

    /// Post-process: update SharedState with output, return Action for next transition
    fn post(&self, state: &mut SharedState, output: Output) -> Action;

    /// Get node configuration
    fn config(&self) -> NodeConfig;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_config_default() {
        let config = NodeConfig::default();
        assert_eq!(config.retries, 3);
        assert_eq!(config.retry_delay_ms, 100);
        assert_eq!(config.timeout_ms, Some(300_000));
    }

    #[test]
    fn test_node_config_builder() {
        let config = NodeConfig::new()
            .with_retries(5)
            .with_retry_delay_ms(200)
            .with_timeout_ms(600_000);

        assert_eq!(config.retries, 5);
        assert_eq!(config.retry_delay_ms, 200);
        assert_eq!(config.timeout_ms, Some(600_000));
    }

    #[test]
    fn test_node_config_without_timeout() {
        let config = NodeConfig::new().without_timeout();
        assert!(config.timeout_ms.is_none());
    }

    // Example test node for trait validation
    struct TestNode {
        id: String,
        config: NodeConfig,
    }

    #[async_trait]
    impl Node for TestNode {
        fn id(&self) -> NodeId {
            self.id.clone()
        }

        fn prep(&self, _state: &SharedState) -> Result<Input> {
            Ok(serde_json::json!({}))
        }

        async fn exec(&self, _input: Input) -> Result<Output> {
            Ok(serde_json::json!({}))
        }

        fn post(&self, _state: &mut SharedState, _output: Output) -> Action {
            "continue".to_string()
        }

        fn config(&self) -> NodeConfig {
            self.config.clone()
        }
    }

    #[tokio::test]
    async fn test_node_trait() {
        let node = TestNode {
            id: "test_node".to_string(),
            config: NodeConfig::default(),
        };

        assert_eq!(node.id(), "test_node");
        
        let state = SharedState::new();
        let input = node.prep(&state).unwrap();
        assert!(input.is_object());
        
        let output = node.exec(input).await.unwrap();
        assert!(output.is_object());
        
        let mut state = SharedState::new();
        let action = node.post(&mut state, output);
        assert_eq!(action, "continue");
    }
}
