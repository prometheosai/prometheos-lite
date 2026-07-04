//! Runtime Context - Service registry for flow execution

use std::sync::Arc;

use crate::flow::{MemoryService, ModelRouter, ToolRuntime, TraceStorage};

/// RuntimeContext - holds all shared services for flow execution
///
/// This struct provides a centralized registry of services that nodes
/// may need during execution, such as LLM providers, tool runtimes,
/// memory services, and trace storage.
#[derive(Clone)]
pub struct RuntimeContext {
    /// Model router for LLM provider selection and routing
    pub model_router: Option<Arc<ModelRouter>>,
    /// Tool runtime for sandboxed command execution
    pub tool_runtime: Option<Arc<ToolRuntime>>,
    /// Memory service for semantic memory operations
    pub memory_service: Option<Arc<MemoryService>>,
    /// Trace storage for persistent observability
    pub trace_storage: Option<Arc<TraceStorage>>,
}

impl RuntimeContext {
    /// Create a new empty RuntimeContext
    pub fn new() -> Self {
        Self {
            model_router: None,
            tool_runtime: None,
            memory_service: None,
            trace_storage: None,
        }
    }

    /// Create a RuntimeContext with a ModelRouter
    pub fn with_model_router(mut self, router: Arc<ModelRouter>) -> Self {
        self.model_router = Some(router);
        self
    }

    /// Create a RuntimeContext with a ToolRuntime
    pub fn with_tool_runtime(mut self, runtime: Arc<ToolRuntime>) -> Self {
        self.tool_runtime = Some(runtime);
        self
    }

    /// Create a RuntimeContext with a MemoryService
    pub fn with_memory_service(mut self, service: Arc<MemoryService>) -> Self {
        self.memory_service = Some(service);
        self
    }

    /// Create a RuntimeContext with TraceStorage
    pub fn with_trace_storage(mut self, storage: Arc<TraceStorage>) -> Self {
        self.trace_storage = Some(storage);
        self
    }

    /// Create a fully populated RuntimeContext
    pub fn full(
        model_router: Arc<ModelRouter>,
        tool_runtime: Arc<ToolRuntime>,
        memory_service: Arc<MemoryService>,
        trace_storage: Arc<TraceStorage>,
    ) -> Self {
        Self {
            model_router: Some(model_router),
            tool_runtime: Some(tool_runtime),
            memory_service: Some(memory_service),
            trace_storage: Some(trace_storage),
        }
    }
}

impl Default for RuntimeContext {
    fn default() -> Self {
        Self::new()
    }
}
