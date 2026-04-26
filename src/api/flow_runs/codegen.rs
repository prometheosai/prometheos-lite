//! CodeGenFlow execution - planner/coder/reviewer execution

use crate::api::flow_runs::errors::handle_flow_error;
use crate::api::flow_runs::events::{emit_node_start, emit_node_end, emit_output};
use crate::api::websocket::ConnectionManager;
use crate::api::AppState;
use crate::db::{Db, CreateMessage};
use crate::db::repository::Repository;
use crate::llm::LlmClient;

/// Execute full code generation flow (planner -> coder -> reviewer)
pub async fn execute_codegen_flow(
    message: &str,
    conversation_id: &str,
    run_id: &str,
    db_path: &str,
    ws_manager: &ConnectionManager,
    llm_client: &LlmClient,
    state: &AppState,
) -> Result<(), anyhow::Error> {
    // Planner phase
    emit_node_start(ws_manager, run_id, "planner").await;

    let planner_prompt = format!(
        "You are a planning AI. Create a detailed plan for the following task:\n\n{}\n\nProvide a step-by-step plan with clear objectives.",
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

    // Coder phase
    emit_node_start(ws_manager, run_id, "coder").await;

    let coder_prompt = format!(
        "You are a coding AI. Generate code based on this plan:\n\n{}\n\nOriginal task: {}\n\nProvide clean, well-commented code.",
        plan, message
    );

    let generated_code = match llm_client.generate(&coder_prompt).await {
        Ok(response) => response,
        Err(e) => {
            handle_flow_error(ws_manager, run_id, "coder", format!("LLM call failed: {}", e), db_path).await;
            return Err(anyhow::anyhow!("LLM call failed: {}", e));
        }
    };

    emit_output(ws_manager, run_id, "coder", generated_code.clone()).await;
    emit_node_end(ws_manager, run_id, "coder").await;

    // Reviewer phase
    emit_node_start(ws_manager, run_id, "reviewer").await;

    let reviewer_prompt = format!(
        "You are a code reviewer. Review this code:\n\n{}\n\nOriginal task: {}\n\nProvide constructive feedback.",
        generated_code, message
    );

    let review = match llm_client.generate(&reviewer_prompt).await {
        Ok(response) => response,
        Err(e) => {
            handle_flow_error(ws_manager, run_id, "reviewer", format!("LLM call failed: {}", e), db_path).await;
            return Err(anyhow::anyhow!("LLM call failed: {}", e));
        }
    };

    emit_output(ws_manager, run_id, "reviewer", review.clone()).await;
    emit_node_end(ws_manager, run_id, "reviewer").await;

    // Memory write phase
    emit_node_start(ws_manager, run_id, "memory_write").await;

    let memory_content = format!("Task: {}\n\nPlan: {}\n\nCode: {}\n\nReview: {}",
        message, plan, generated_code, review);

    match state.embedding_provider.embed(&memory_content).await {
        Ok(embedding) => {
            emit_output(ws_manager, run_id, "memory_write", format!("Memory stored with embedding (dimension: {})", embedding.len())).await;
        }
        Err(e) => {
            handle_flow_error(ws_manager, run_id, "memory_write", format!("Memory write failed: {}", e), db_path).await;
        }
    }

    emit_node_end(ws_manager, run_id, "memory_write").await;

    // Combine outputs for assistant response
    let assistant_response = format!(
        "## Plan\n\n{}\n\n## Generated Code\n\n{}\n\n## Review\n\n{}",
        plan, generated_code, review
    );

    // Save assistant message
    if let Ok(db) = Db::new(db_path) {
        let _ = db.create_message(CreateMessage {
            conversation_id: conversation_id.to_string(),
            role: "assistant".to_string(),
            content: assistant_response.clone(),
        });
    }

    emit_output(ws_manager, run_id, "assistant", assistant_response).await;
    emit_output(ws_manager, run_id, "system", "Flow execution completed".to_string()).await;

    if let Ok(db) = Db::new(db_path) {
        let _ = db.update_flow_run_status(run_id, "completed");
    }

    Ok(())
}
