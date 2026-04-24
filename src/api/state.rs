//! Application state for API server
//!
//! This module defines the shared application state that is passed to all route handlers.

use std::sync::Arc;

use crate::api::ConnectionManager;
use crate::flow::RuntimeContext;

/// Global application state shared across all API routes
///
/// This struct holds the database path, flow runtime context, and WebSocket manager.
/// Database connections are created per request for thread safety.
#[derive(Clone)]
pub struct AppState {
    /// Database path for UI state persistence
    pub db_path: String,
    /// Flow runtime context for executing flows
    pub runtime: Arc<RuntimeContext>,
    /// WebSocket connection manager for real-time updates
    pub ws_manager: ConnectionManager,
}

impl AppState {
    /// Create a new AppState with the given database path and runtime context
    pub fn new(db_path: String, runtime: Arc<RuntimeContext>) -> Self {
        Self {
            db_path,
            runtime,
            ws_manager: ConnectionManager::new(),
        }
    }
}
