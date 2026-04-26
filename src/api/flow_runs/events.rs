//! WebSocket event helpers for flow execution

use crate::api::websocket::FlowEvent;
use crate::api::websocket::ConnectionManager;
use chrono::Utc;

/// Emit a node start event
pub async fn emit_node_start(
    ws_manager: &ConnectionManager,
    run_id: &str,
    node: &str,
) {
    let _ = ws_manager.send_event(run_id, FlowEvent::NodeStart {
        node: node.to_string(),
        timestamp: Utc::now(),
    }).await;
}

/// Emit a node end event
pub async fn emit_node_end(
    ws_manager: &ConnectionManager,
    run_id: &str,
    node: &str,
) {
    let _ = ws_manager.send_event(run_id, FlowEvent::NodeEnd {
        node: node.to_string(),
        timestamp: Utc::now(),
    }).await;
}

/// Emit an output event
pub async fn emit_output(
    ws_manager: &ConnectionManager,
    run_id: &str,
    node: &str,
    data: String,
) {
    let _ = ws_manager.send_event(run_id, FlowEvent::Output {
        node: node.to_string(),
        data,
        timestamp: Utc::now(),
    }).await;
}
