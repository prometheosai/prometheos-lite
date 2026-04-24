//! API module for PrometheOS Lite HTTP server
//!
//! This module provides the HTTP API layer for the local chat interface,
//! including REST endpoints and WebSocket streaming for real-time flow execution.

pub mod server;
pub mod state;
pub mod websocket;

pub use server::{create_router, run_server};
pub use state::AppState;
pub use websocket::{ConnectionManager, FlowEvent};
