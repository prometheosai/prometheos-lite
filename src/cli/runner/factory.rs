//! Node factory for creating concrete nodes based on node_type

use anyhow::Result;
use std::sync::Arc;

use prometheos_lite::flow::{Node, NodeConfig};

/// NodeFactory trait - creates concrete nodes based on node_type
pub trait NodeFactory: Send + Sync {
    /// Create a node from node_type and optional config
    fn create(&self, node_type: &str, config: Option<serde_json::Value>) -> Result<Arc<dyn Node>>;
}

/// Default implementation of NodeFactory
pub struct DefaultNodeFactory {
    model_router: Option<std::sync::Arc<prometheos_lite::flow::ModelRouter>>,
    tool_runtime: Option<std::sync::Arc<prometheos_lite::flow::ToolRuntime>>,
    memory_service: Option<std::sync::Arc<prometheos_lite::flow::MemoryService>>,
}

impl DefaultNodeFactory {
    pub fn new() -> Self {
        Self {
            model_router: None,
            tool_runtime: None,
            memory_service: None,
        }
    }

    /// Create a DefaultNodeFactory from a RuntimeContext
    pub fn from_runtime(runtime: prometheos_lite::flow::RuntimeContext) -> Self {
        Self {
            model_router: runtime.model_router,
            tool_runtime: runtime.tool_runtime,
            memory_service: runtime.memory_service,
        }
    }

    pub fn with_model_router(mut self, router: std::sync::Arc<prometheos_lite::flow::ModelRouter>) -> Self {
        self.model_router = Some(router);
        self
    }

    pub fn with_tool_runtime(mut self, runtime: std::sync::Arc<prometheos_lite::flow::ToolRuntime>) -> Self {
        self.tool_runtime = Some(runtime);
        self
    }

    pub fn with_memory_service(mut self, service: std::sync::Arc<prometheos_lite::flow::MemoryService>) -> Self {
        self.memory_service = Some(service);
        self
    }

    fn parse_config(config: &Option<serde_json::Value>) -> Result<NodeConfig> {
        if let Some(cfg) = config {
            let retries = cfg["retries"].as_u64().unwrap_or(3) as u8;
            let retry_delay_ms = cfg["retry_delay_ms"].as_u64().unwrap_or(100);
            let timeout_ms = cfg["timeout_ms"].as_u64();

            Ok(NodeConfig {
                retries,
                retry_delay_ms,
                timeout_ms,
            })
        } else {
            Ok(NodeConfig::default())
        }
    }
}

impl NodeFactory for DefaultNodeFactory {
    fn create(&self, node_type: &str, config: Option<serde_json::Value>) -> Result<Arc<dyn Node>> {
        let node_config = Self::parse_config(&config)?;

        match node_type {
            "planner" => Ok(Arc::new(super::nodes::PlannerNode::new(node_config, self.model_router.clone()))),
            "coder" => Ok(Arc::new(super::nodes::CoderNode::new(node_config, self.model_router.clone()))),
            "reviewer" => Ok(Arc::new(super::nodes::ReviewerNode::new(node_config, self.model_router.clone()))),
            "llm" => Ok(Arc::new(super::nodes::LlmNode::new(node_config, self.model_router.clone(), config))),
            "tool" => Ok(Arc::new(super::nodes::ToolNode::new(node_config, self.tool_runtime.clone()))),
            "file_writer" => Ok(Arc::new(super::nodes::FileWriterNode::new(node_config))),
            "context_loader" => Ok(Arc::new(super::nodes::ContextLoaderNode::new(node_config, self.memory_service.clone()))),
            "memory_write" => Ok(Arc::new(super::nodes::MemoryWriteNode::new(node_config, self.memory_service.clone()))),
            "conditional" => Ok(Arc::new(super::nodes::ConditionalNode::new(node_config))),
            _ => {
                // Default to passthrough for unknown types
                Ok(Arc::new(super::nodes::PassthroughNode::new(node_config)))
            }
        }
    }
}

impl Default for DefaultNodeFactory {
    fn default() -> Self {
        Self::new()
    }
}
