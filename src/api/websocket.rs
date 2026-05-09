//! WebSocket support for real-time flow execution updates
//!
//! This module provides WebSocket endpoints for streaming flow execution events
//! to connected clients.

use axum::{
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};

use crate::api::AppState;

/// WebSocket event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum FlowEvent {
    #[serde(rename = "node_start")]
    NodeStart {
        node: String,
        timestamp: DateTime<Utc>,
    },
    #[serde(rename = "node_end")]
    NodeEnd {
        node: String,
        timestamp: DateTime<Utc>,
    },
    #[serde(rename = "output")]
    Output {
        node: String,
        data: String,
        timestamp: DateTime<Utc>,
    },
    #[serde(rename = "error")]
    Error {
        node: String,
        message: String,
        timestamp: DateTime<Utc>,
    },
    #[serde(rename = "job_queued")]
    JobQueued {
        job_id: String,
        job_type: String,
        priority: String,
        timestamp: DateTime<Utc>,
    },
    #[serde(rename = "job_started")]
    JobStarted {
        job_id: String,
        timestamp: DateTime<Utc>,
    },
    #[serde(rename = "job_completed")]
    JobCompleted {
        job_id: String,
        timestamp: DateTime<Utc>,
    },
    #[serde(rename = "job_failed")]
    JobFailed {
        job_id: String,
        error: String,
        timestamp: DateTime<Utc>,
    },
    #[serde(rename = "skill_extracted")]
    SkillExtracted {
        skill_id: String,
        skill_name: String,
        timestamp: DateTime<Utc>,
    },
    #[serde(rename = "evolution_created")]
    EvolutionCreated {
        evolution_id: String,
        playbook_id: String,
        version: u32,
        timestamp: DateTime<Utc>,
    },
    #[serde(rename = "evolution_promoted")]
    EvolutionPromoted {
        evolution_id: String,
        version: u32,
        timestamp: DateTime<Utc>,
    },
    #[serde(rename = "metrics_update")]
    MetricsUpdate {
        uptime_seconds: u64,
        memory_usage_mb: u64,
        active_connections: usize,
        timestamp: DateTime<Utc>,
    },
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

    /// Count active WebSocket subscribers across all run channels.
    pub async fn active_connections(&self) -> usize {
        let channels = self.channels.read().await;
        channels
            .values()
            .map(|sender| sender.receiver_count())
            .sum()
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
async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>, run_id: String) {
    // Get the broadcast channel for this run
    let channel = state.ws_manager.get_channel(&run_id).await;
    let mut receiver = channel.subscribe();

    // Send initial connection message
    let init_event = FlowEvent::NodeStart {
        node: "system".to_string(),
        timestamp: Utc::now(),
    };
    let _ = socket
        .send(Message::Text(serde_json::to_string(&init_event).unwrap()))
        .await;

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
