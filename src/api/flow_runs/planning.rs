//! Planning handler - generates plan/PRD only, waits for user approval

use crate::api::flow_runs::errors::handle_flow_error;
use crate::api::flow_runs::events::{emit_node_start, emit_node_end, emit_output};
use crate::api::websocket::ConnectionManager;
use crate::config::AppConfig;
use crate::db::{Db, CreateMessage};
use crate::db::repository::Repository;
use crate::llm::LlmClient;

/// Execute planning handler - generates plan/PRD only
pub async fn execute_planning(
    message: &str,
    conversation_id: &str,
    run_id: &str,
    db_path: &str,
    ws_manager: &ConnectionManager,
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

    emit_node_start(ws_manager, run_id, "planner").await;

    let planner_prompt = format!(
        "You are a product manager and technical writer. Create a detailed PRD (Product Requirements Document) in GitHub Issues format for the following request:\n\n{}\n\nInclude:\n- Title\n- Summary\n- Goals\n- User Stories\n- Technical Requirements\n- Acceptance Criteria\n- Success Metrics\nFormat the response as a GitHub Issue with proper markdown.",
        message
    );

    let plan = match llm_client.generate(&planner_prompt).await {
        Ok(response) => response,
        Err(e) => {
            handle_flow_error(ws_manager, run_id, "planner", format!("LLM call failed: {}", e), db_path).await;
            return Err(anyhow::anyhow!("LLM call failed: {}", e));
        }
    };

    emit_output(ws_manager, run_id, "planner", plan.clone()).await;
    emit_node_end(ws_manager, run_id, "planner").await;

    // Save assistant message with the plan
    if let Ok(db) = Db::new(db_path) {
        let _ = db.create_message(CreateMessage {
            conversation_id: conversation_id.to_string(),
            role: "assistant".to_string(),
            content: plan.clone(),
        });
    }

    emit_output(ws_manager, run_id, "system", "Plan generated. Review the PRD above. To proceed with implementation, say 'Implement this plan' or 'Continue'. To modify the plan, provide your feedback.".to_string()).await;

    if let Ok(db) = Db::new(db_path) {
        let _ = db.update_flow_run_status(run_id, "waiting_for_approval");
    }

    Ok(())
}
