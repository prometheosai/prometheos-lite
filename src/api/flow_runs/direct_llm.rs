//! Direct LLM response path for conversation/questions

use crate::api::flow_runs::errors::handle_flow_error;
use crate::api::flow_runs::events::{emit_output, emit_node_end};
use crate::api::websocket::ConnectionManager;
use crate::config::AppConfig;
use crate::db::Db;
use crate::db::repository::Repository;
use crate::llm::LlmClient;
use chrono::Utc;

/// Execute direct LLM response for conversation/questions
pub async fn execute_direct_llm(
    message: &str,
    conversation_id: &str,
    run_id: &str,
    db_path: &str,
    ws_manager: &ConnectionManager,
    control_files: &crate::control::ControlFiles,
) -> Result<(), anyhow::Error> {
    let config = match AppConfig::load() {
        Ok(c) => c,
        Err(e) => {
            handle_flow_error(ws_manager, run_id, "system", format!("Failed to load config: {}", e), db_path).await;
            return Err(anyhow::anyhow!("Failed to load config: {}", e));
        }
    };

    let llm_client = match LlmClient::from_config(&config) {
        Ok(client) => client,
        Err(e) => {
            handle_flow_error(ws_manager, run_id, "system", format!("Failed to create LLM client: {}", e), db_path).await;
            return Err(anyhow::anyhow!("Failed to create LLM client: {}", e));
        }
    };

    let conversation_prompt = control_files.build_conversation_prompt(message);

    emit_output(ws_manager, run_id, "system", "Thinking...".to_string()).await;

    let response = match llm_client.generate(&conversation_prompt).await {
        Ok(resp) => resp,
        Err(e) => {
            handle_flow_error(ws_manager, run_id, "system", format!("LLM call failed: {}", e), db_path).await;
            return Err(anyhow::anyhow!("LLM call failed: {}", e));
        }
    };

    // Save assistant message
    if let Ok(db) = Db::new(db_path) {
        let _ = db.create_message(crate::db::CreateMessage {
            conversation_id: conversation_id.to_string(),
            role: "assistant".to_string(),
            content: response.clone(),
        });
    }

    emit_output(ws_manager, run_id, "assistant", response.clone()).await;
    emit_output(ws_manager, run_id, "system", "Direct response completed".to_string()).await;
    emit_node_end(ws_manager, run_id, "assistant").await;

    if let Ok(db) = Db::new(db_path) {
        let _ = db.update_flow_run_status(run_id, "completed");
    }

    Ok(())
}
