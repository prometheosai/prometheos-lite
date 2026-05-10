//! Node factory for creating concrete nodes based on node_type

use anyhow::Result;
use std::sync::Arc;

use crate::context::ContextBuilder;
use crate::flow::{MemoryService, ModelRouter, Node, NodeConfig, ToolRuntime};
use crate::flow::factory::{NodeRegistry, register_builtin_nodes, register_harness_nodes};

/// NodeFactory trait - creates concrete nodes based on node_type
pub trait NodeFactory: Send + Sync {
    /// Create a node from node_type and optional config
    fn create(&self, node_type: &str, config: Option<serde_json::Value>) -> Result<Arc<dyn Node>>;
}

/// Default implementation of NodeFactory
pub struct DefaultNodeFactory {
    model_router: Option<std::sync::Arc<ModelRouter>>,
    tool_runtime: Option<std::sync::Arc<ToolRuntime>>,
    memory_service: Option<std::sync::Arc<MemoryService>>,
    context_builder: Option<ContextBuilder>,
    repo_path: Option<std::path::PathBuf>,
    registry: NodeRegistry,
}

impl DefaultNodeFactory {
    pub fn new() -> Self {
        let mut registry = NodeRegistry::new();
        register_builtin_nodes(&mut registry);
        register_harness_nodes(&mut registry);
        Self {
            model_router: None,
            tool_runtime: None,
            memory_service: None,
            context_builder: None,
            repo_path: None,
            registry,
        }
    }

    pub fn with_repo_path(mut self, path: std::path::PathBuf) -> Self {
        self.repo_path = Some(path);
        self
    }

    /// Create a DefaultNodeFactory from a RuntimeContext
    ///
    /// The ContextBuilder is automatically wired with the memory service if available,
    /// enabling automatic memory retrieval for all LLM nodes.
    pub fn from_runtime(runtime: crate::flow::RuntimeContext) -> Self {
        let context_builder = runtime.memory_service.as_ref().map(|ms| {
            ContextBuilder::with_memory_service(
                crate::context::ContextBudgeter::default(),
                ms.clone(),
            )
        });

        Self {
            model_router: runtime.model_router,
            tool_runtime: runtime.tool_runtime,
            memory_service: runtime.memory_service,
            context_builder,
            repo_path: None,
            registry: {
                let mut r = NodeRegistry::new();
                register_builtin_nodes(&mut r);
                register_harness_nodes(&mut r);
                r
            },
        }
    }

    pub fn from_runtime_with_repo(
        runtime: crate::flow::RuntimeContext,
        repo_path: std::path::PathBuf,
    ) -> Self {
        let context_builder = runtime.memory_service.as_ref().map(|ms| {
            ContextBuilder::with_memory_service(
                crate::context::ContextBudgeter::default(),
                ms.clone(),
            )
        });

        Self {
            model_router: runtime.model_router,
            tool_runtime: runtime.tool_runtime,
            memory_service: runtime.memory_service,
            context_builder,
            repo_path: Some(repo_path),
            registry: {
                let mut r = NodeRegistry::new();
                register_builtin_nodes(&mut r);
                register_harness_nodes(&mut r);
                r
            },
        }
    }

    pub fn with_model_router(mut self, router: std::sync::Arc<ModelRouter>) -> Self {
        self.model_router = Some(router);
        self
    }

    pub fn with_tool_runtime(mut self, runtime: std::sync::Arc<ToolRuntime>) -> Self {
        self.tool_runtime = Some(runtime);
        self
    }

    pub fn with_memory_service(mut self, service: std::sync::Arc<MemoryService>) -> Self {
        self.memory_service = Some(service);
        self
    }

    pub fn with_context_builder(mut self, builder: ContextBuilder) -> Self {
        self.context_builder = Some(builder);
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

    fn create_known_node(
        &self,
        node_type: &str,
        node_config: NodeConfig,
        config: Option<serde_json::Value>,
        context_builder: ContextBuilder,
    ) -> Result<Arc<dyn Node>> {
        match node_type {
            "planner" => Ok(Arc::new(super::builtin_nodes::PlannerNode::new(
                node_config,
                self.model_router.clone(),
                context_builder,
            ))),
            "coder" => Ok(Arc::new(super::builtin_nodes::CoderNode::new(
                node_config,
                self.model_router.clone(),
                context_builder,
            ))),
            "reviewer" => Ok(Arc::new(super::builtin_nodes::ReviewerNode::new(
                node_config,
                self.model_router.clone(),
                context_builder,
            ))),
            "terminal" => Ok(Arc::new(super::builtin_nodes::TerminalNode::new(
                node_config,
            ))),
            "llm" => Ok(Arc::new(super::builtin_nodes::LlmNode::new(
                node_config,
                self.model_router.clone(),
                config,
                context_builder,
            ))),
            "tool" => Ok(Arc::new(super::builtin_nodes::ToolNode::new(
                node_config,
                self.tool_runtime.clone(),
            ))),
            "file_writer" => Ok(Arc::new(super::builtin_nodes::FileWriterNode::new(
                node_config,
            ))),
            "context_loader" => Ok(Arc::new(super::builtin_nodes::ContextLoaderNode::new(
                node_config,
                self.memory_service.clone(),
            ))),
            "memory_write" => Ok(Arc::new(super::builtin_nodes::MemoryWriteNode::new(
                node_config,
                self.memory_service.clone(),
            ))),
            "conditional" => Ok(Arc::new(super::builtin_nodes::ConditionalNode::new(
                node_config,
            ))),
            "passthrough" => Ok(Arc::new(super::builtin_nodes::PassthroughNode::new(
                node_config,
            ))),
            "code_analysis" => {
                let repo_path = self
                    .repo_path
                    .clone()
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                Ok(Arc::new(super::coding_nodes::CodeAnalysisNode::new(
                    node_config,
                    repo_path,
                )))
            }
            "symbol_resolution" => {
                let repo_path = self
                    .repo_path
                    .clone()
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                Ok(Arc::new(super::coding_nodes::SymbolResolutionNode::new(
                    node_config,
                    repo_path,
                )))
            }
            "dependency_analysis" => {
                let repo_path = self
                    .repo_path
                    .clone()
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                Ok(Arc::new(super::coding_nodes::DependencyAnalysisNode::new(
                    node_config,
                    repo_path,
                )))
            }
            "harness.repo_map" => {
                let repo_path = self
                    .repo_path
                    .clone()
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                let inner = Arc::new(super::coding_nodes::CodeAnalysisNode::new(
                    node_config,
                    repo_path,
                ));
                Ok(Arc::new(super::builtin_nodes::IdWrapper::new(
                    "harness.repo_map".to_string(),
                    inner,
                )))
            }
            "harness.patch_apply" => {
                let inner = Arc::new(super::builtin_nodes::ToolNode::new(
                    node_config,
                    self.tool_runtime.clone(),
                ));
                Ok(Arc::new(super::builtin_nodes::IdWrapper::new(
                    "harness.patch_apply".to_string(),
                    inner,
                )))
            }
            "harness.validate" => {
                let inner = Arc::new(super::builtin_nodes::ToolNode::new(
                    node_config,
                    self.tool_runtime.clone(),
                ));
                Ok(Arc::new(super::builtin_nodes::IdWrapper::new(
                    "harness.validate".to_string(),
                    inner,
                )))
            }
            "harness.review" => {
                let inner = Arc::new(super::builtin_nodes::ReviewerNode::new(
                    node_config,
                    self.model_router.clone(),
                    context_builder,
                ));
                Ok(Arc::new(super::builtin_nodes::IdWrapper::new(
                    "harness.review".to_string(),
                    inner,
                )))
            }
            "harness.risk" => {
                let inner = Arc::new(super::builtin_nodes::ReviewerNode::new(
                    node_config,
                    self.model_router.clone(),
                    context_builder,
                ));
                Ok(Arc::new(super::builtin_nodes::IdWrapper::new(
                    "harness.risk".to_string(),
                    inner,
                )))
            }
            "harness.completion" => {
                let inner = Arc::new(super::builtin_nodes::TerminalNode::new(node_config));
                Ok(Arc::new(super::builtin_nodes::IdWrapper::new(
                    "harness.completion".to_string(),
                    inner,
                )))
            }
            "harness.attempt_pool" => {
                let inner = Arc::new(super::builtin_nodes::ConditionalNode::new(node_config));
                Ok(Arc::new(super::builtin_nodes::IdWrapper::new(
                    "harness.attempt_pool".to_string(),
                    inner,
                )))
            }
            "harness.context_distill" => {
                let inner = Arc::new(super::builtin_nodes::ContextLoaderNode::new(
                    node_config,
                    self.memory_service.clone(),
                ));
                Ok(Arc::new(super::builtin_nodes::IdWrapper::new(
                    "harness.context_distill".to_string(),
                    inner,
                )))
            }
            _ => anyhow::bail!("Unknown canonical node type '{}'", node_type),
        }
    }
}

impl NodeFactory for DefaultNodeFactory {
    fn create(&self, node_type: &str, config: Option<serde_json::Value>) -> Result<Arc<dyn Node>> {
        let node_config = Self::parse_config(&config)?;
        let context_builder = self
            .context_builder
            .clone()
            .unwrap_or_default();
        let resolved = self.registry.resolve(node_type).ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown node type '{}'. NodeRegistry has no registration for this type.",
                node_type
            )
        })?;
        self.create_known_node(resolved, node_config, config, context_builder)
    }
}

impl Default for DefaultNodeFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{DefaultNodeFactory, NodeFactory};

    #[test]
    fn harness_nodes_are_registry_backed_and_constructible() {
        let factory = DefaultNodeFactory::new();
        for node_type in [
            "harness.repo_map",
            "harness.patch_apply",
            "harness.validate",
            "harness.review",
            "harness.risk",
            "harness.completion",
            "harness.attempt_pool",
            "harness.context_distill",
        ] {
            let node = factory.create(node_type, None).unwrap();
            assert_eq!(node.id(), node_type);
        }
    }

    #[test]
    fn unknown_harness_node_is_hard_error() {
        let factory = DefaultNodeFactory::new();
        let result = factory.create("harness.unknown_node", None);
        assert!(result.is_err());
        let err = result.err().expect("expected error");
        assert!(err.to_string().contains("Unknown node type"));
    }
}
