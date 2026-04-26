//! Approval handler - continues with full CodeGenFlow after user approves the plan

use crate::api::flow_runs::codegen::execute_codegen_flow;
use crate::api::flow_runs::errors::handle_flow_error;
use crate::api::flow_runs::events::emit_output;
use crate::api::websocket::ConnectionManager;
use crate::config::AppConfig;
use crate::db::repository::Repository;
use crate::llm::LlmClient;

/// Execute approval handler - continues with implementation
pub async fn execute_approval(
    message: &str,
    conversation_id: &str,
    run_id: &str,
    db_path: &str,
    ws_manager: &ConnectionManager,
    state: &crate::api::AppState,
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

    emit_output(ws_manager, run_id, "system", "User approved the plan. Starting implementation...".to_string()).await;

    // Continue with CodeGenFlow logic (coder -> reviewer -> memory_write)
    execute_codegen_flow(message, conversation_id, run_id, db_path, ws_manager, &llm_client, state).await?;

    Ok(())
}
