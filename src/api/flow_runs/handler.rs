//! Flow run handler - entrypoint for run flow
//!
//! Uses FlowExecutionService for all execution logic,
//! keeping the handler thin: just HTTP framing + WS events.

use axum::{
    Json,
    extract::{Path, State},
};
use std::sync::Arc;

use crate::api::AppState;
use crate::api::websocket::FlowEvent;
use crate::db::{Db, FlowRun, Repository, RunFlow};
use crate::flow::MemoryKind;
use crate::flow::execution_service::{ExecutionOptions, FlowExecutionService};
use crate::intent::Intent;
use chrono::Utc;

/// Run a flow for a conversation
pub async fn run_flow(
    State(state): State<Arc<AppState>>,
    Path(conversation_id): Path<String>,
    Json(input): Json<RunFlow>,
) -> Result<Json<FlowRun>, axum::http::StatusCode> {
    let db_path = state.db_path.clone();
    let message = input.message.clone();
    let ws_manager = state.ws_manager.clone();
    let runtime = state.runtime.clone();

    // Save user message
    let db = Db::new(&db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = db.create_message(crate::db::CreateMessage {
        conversation_id: conversation_id.clone(),
        role: "user".to_string(),
        content: message.clone(),
    });

    // Create FlowRun
    let flow_run = db
        .create_flow_run(&conversation_id)
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let run_id = flow_run.id.clone();

    // Spawn async task for flow execution via FlowExecutionService
    tokio::spawn(async move {
        // Create the shared execution service (runtime-aware factory)
        let exec_service = match FlowExecutionService::new(runtime.clone()) {
            Ok(svc) => svc,
            Err(e) => {
                let _ = ws_manager
                    .send_event(
                        &run_id,
                        FlowEvent::Error {
                            node: "system".to_string(),
                            message: format!("Failed to create execution service: {}", e),
                            timestamp: Utc::now(),
                        },
                    )
                    .await;
                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.update_flow_run_status(&run_id, "failed");
                }
                return;
            }
        };

        // Log user message as episodic memory (async, non-blocking)
        if let Some(memory_service) = runtime.memory_service.as_ref() {
            let _ = memory_service.queue_episode(
                format!("User: {}", message),
                None,
                None,
                Some(conversation_id.clone()),
                serde_json::json!({
                    "role": "user",
                    "flow_run_id": run_id,
                }),
            );
        }

        // Load relevant context from memory before LLM calls
        let relevant_context = if let Some(memory_service) = runtime.memory_service.as_ref() {
            match memory_service.semantic_search(&message, 5).await {
                Ok(memories) => {
                    let context: Vec<String> = memories
                        .iter()
                        .filter(|m| m.kind != MemoryKind::Episodic)
                        .map(|m| m.content.clone())
                        .collect();
                    if !context.is_empty() {
                        Some(format!("Relevant Memory Context:\n{}", context.join("\n")))
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        } else {
            None
        };

        // Check for override commands
        let override_intent = Intent::from_override(&message);
        let actual_message = if override_intent.is_some() {
            message
                .split_whitespace()
                .skip_while(|word| word.starts_with('/'))
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string()
        } else {
            message.clone()
        };

        // Prepend relevant memory context to message if available
        let message_to_process = if actual_message.is_empty() {
            message.clone()
        } else {
            actual_message
        };
        
        let message_with_context = if let Some(context) = relevant_context {
            format!("{}\n\nUser message: {}", context, message_to_process)
        } else {
            message_to_process
        };

        // Build execution options
        let mut options = ExecutionOptions::default();
        if let Some(intent) = override_intent {
            options = options.with_override_intent(intent);
        }

        // Emit that we're starting classification
        let _ = ws_manager
            .send_event(
                &run_id,
                FlowEvent::Output {
                    node: "system".to_string(),
                    data: "Classifying intent...".to_string(),
                    timestamp: Utc::now(),
                },
            )
            .await;

        // Execute via the shared service with context-enhanced message
        let final_output = exec_service
            .execute_message(&message_with_context, options)
            .await;

        match final_output {
            Ok(output) => {
                if output.success {
                    // Emit the output
                    let _ = ws_manager
                        .send_event(
                            &run_id,
                            FlowEvent::Output {
                                node: "system".to_string(),
                                data: serde_json::to_string_pretty(&output)
                                    .unwrap_or_else(|_| "Failed to serialize output".to_string()),
                                timestamp: Utc::now(),
                            },
                        )
                        .await;

                    if let Ok(db) = Db::new(&db_path) {
                        let _ = db.update_flow_run_status(&run_id, "completed");
                    }
                } else {
                    let _ = ws_manager
                        .send_event(
                            &run_id,
                            FlowEvent::Error {
                                node: "system".to_string(),
                                message: format!(
                                    "Flow execution failed: {}",
                                    output.error.unwrap_or_default()
                                ),
                                timestamp: Utc::now(),
                            },
                        )
                        .await;

                    if let Ok(db) = Db::new(&db_path) {
                        let _ = db.update_flow_run_status(&run_id, "failed");
                    }
                }
            }
            Err(e) => {
                let _ = ws_manager
                    .send_event(
                        &run_id,
                        FlowEvent::Error {
                            node: "system".to_string(),
                            message: format!("Flow execution failed: {}", e),
                            timestamp: Utc::now(),
                        },
                    )
                    .await;
                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.update_flow_run_status(&run_id, "failed");
                }
            }
        }
    });

    Ok(Json(flow_run))
}
