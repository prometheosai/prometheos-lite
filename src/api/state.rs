//! Application state for API server
//!
//! This module defines the shared application state that is passed to all route handlers.

use std::sync::Arc;

use crate::api::ConnectionManager;
use crate::flow::EmbeddingProvider;
use crate::flow::LocalEmbeddingProvider;
use crate::flow::MemoryService;
use crate::flow::RuntimeContext;
use crate::flow::execution_service::FlowExecutionService;
use crate::intent::IntentClassifier;
use crate::work::{PlaybookResolver, WorkContextService, WorkOrchestrator};

/// Global application state shared across all API routes
///
/// This struct holds the database path, flow runtime context, WebSocket manager, embedding provider, and memory service.
/// Database connections are created per request for thread safety.
#[derive(Clone)]
pub struct AppState {
    /// Database path for UI state persistence
    pub db_path: String,
    /// Flow runtime context for executing flows
    pub runtime: Arc<RuntimeContext>,
    /// WebSocket connection manager for real-time updates
    pub ws_manager: ConnectionManager,
    /// Embedding provider for memory operations
    pub embedding_provider: Arc<dyn EmbeddingProvider>,
    /// Memory service for persistent memory storage and retrieval
    pub memory_service: Arc<MemoryService>,
}

impl AppState {
    /// Create a new AppState with the given database path, runtime context, embedding provider, and memory service
    pub fn new(
        db_path: String,
        runtime: Arc<RuntimeContext>,
        embedding_provider: Arc<dyn EmbeddingProvider>,
        memory_service: Arc<MemoryService>,
    ) -> Self {
        Self {
            db_path,
            runtime,
            ws_manager: ConnectionManager::new(),
            embedding_provider,
            memory_service,
        }
    }
}
