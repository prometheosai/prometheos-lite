//! WebSocket support for real-time flow execution updates
//!
//! This module provides WebSocket endpoints for streaming flow execution events
//! to connected clients.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use crate::api::AppState;

/// WebSocket event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum FlowEvent {
    #[serde(rename = "node_start")]
    NodeStart { node: String, timestamp: DateTime<Utc> },
    #[serde(rename = "node_end")]
    NodeEnd { node: String, timestamp: DateTime<Utc> },
    #[serde(rename = "output")]
    Output { node: String, data: String, timestamp: DateTime<Utc> },
    #[serde(rename = "error")]
    Error { node: String, message: String, timestamp: DateTime<Utc> },
}

/// WebSocket connection manager
#[derive(Clone)]
pub struct ConnectionManager {
    /// Map of run_id to broadcast channels
    channels: Arc<RwLock<HashMap<String, broadcast::Sender<FlowEvent>>>>,
}

impl ConnectionManager {
    /// Create a new connection manager
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get or create a broadcast channel for a run
    pub async fn get_channel(&self, run_id: &str) -> broadcast::Sender<FlowEvent> {
        let mut channels = self.channels.write().await;
        
        if let Some(sender) = channels.get(run_id) {
            sender.clone()
        } else {
            let (sender, _) = broadcast::channel(100);
            channels.insert(run_id.to_string(), sender.clone());
            sender
        }
    }

    /// Send an event to all subscribers for a run
    pub async fn send_event(&self, run_id: &str, event: FlowEvent) {
        if let Some(sender) = self.channels.read().await.get(run_id) {
            let _ = sender.send(event);
        }
    }

    /// Remove a channel when all subscribers are gone
    pub async fn cleanup(&self, run_id: &str) {
        let mut channels = self.channels.write().await;
        channels.remove(run_id);
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// WebSocket handler for flow run updates
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, run_id))
}

/// Handle a WebSocket connection
async fn handle_socket(
    mut socket: WebSocket,
    state: Arc<AppState>,
    run_id: String,
) {
    // Get the broadcast channel for this run
    let channel = state.ws_manager.get_channel(&run_id).await;
    let mut receiver = channel.subscribe();

    // Send initial connection message
    let init_event = FlowEvent::NodeStart {
        node: "system".to_string(),
        timestamp: Utc::now(),
    };
    let _ = socket.send(Message::Text(serde_json::to_string(&init_event).unwrap())).await;

    // Forward events from the channel to the WebSocket
    loop {
        tokio::select! {
            // Receive events from the channel
            event = receiver.recv() => {
                match event {
                    Ok(event) => {
                        if let Ok(json) = serde_json::to_string(&event) {
                            if socket.send(Message::Text(json)).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            // Handle incoming messages from client (ping/pong)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Ping(msg))) => {
                        if socket.send(Message::Pong(msg)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                }
            }
        }
    }

    // Cleanup when connection closes
    state.ws_manager.cleanup(&run_id).await;
}
