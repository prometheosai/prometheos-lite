//! Error mapping for flow execution

use crate::api::websocket::FlowEvent;
use crate::api::websocket::ConnectionManager;
use crate::db::repository::Repository;
use chrono::Utc;

/// Send error event via WebSocket and update flow run status
pub async fn handle_flow_error(
    ws_manager: &ConnectionManager,
    run_id: &str,
    node: &str,
    message: String,
    db_path: &str,
) {
    let _ = ws_manager.send_event(run_id, FlowEvent::Error {
        node: node.to_string(),
        message,
        timestamp: Utc::now(),
    }).await;
    
    if let Ok(db) = crate::db::Db::new(db_path) {
        let _ = db.update_flow_run_status(run_id, "failed");
    }
}
