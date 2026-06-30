//! HTTP API server for PrometheOS Lite
//!
//! This module provides the Axum-based HTTP server with REST endpoints
//! and WebSocket support for the local chat interface.

use std::net::SocketAddr;
use std::sync::Arc;

use crate::api::AppState;
use crate::api::router::create_router;

/// Run the API server
///
/// Starts the Axum server on the specified address and port.
pub async fn run_server(addr: SocketAddr, state: Arc<AppState>) -> anyhow::Result<()> {
    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("API server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
