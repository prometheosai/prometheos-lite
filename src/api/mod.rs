//! API module for PrometheOS Lite HTTP server
//!
//! This module provides the HTTP API layer for the local chat interface,
//! including REST endpoints and WebSocket streaming for real-time flow execution.

pub mod conversations;
pub mod flow_runs;
pub mod health;
pub mod messages;
pub mod projects;
pub mod router;
pub mod server;
pub mod state;
pub mod websocket;

pub use router::create_router;
pub use server::run_server;
pub use state::AppState;
pub use websocket::{ConnectionManager, FlowEvent};
