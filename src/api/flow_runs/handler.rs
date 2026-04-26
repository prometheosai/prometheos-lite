//! Flow run handler - entrypoint for run flow

use axum::{extract::{Path, State}, Json};
use std::sync::Arc;

use crate::api::AppState;
use crate::api::flow_runs::codegen::execute_codegen_flow;
use crate::api::flow_runs::direct_llm::execute_direct_llm;
use crate::api::flow_runs::planning::execute_planning;
use crate::api::flow_runs::approval::execute_approval;
use crate::api::websocket::FlowEvent;
use crate::api::websocket::ConnectionManager;
use crate::config::AppConfig;
use crate::control::ControlFiles;
use crate::db::{Db, FlowRun, RunFlow};
use crate::db::repository::Repository;
use crate::flow::MemoryKind;
use crate::intent::{IntentClassifier, IntentRouter, Handler, Intent};
use crate::llm::LlmClient;
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
    let flow_run = db.create_flow_run(&conversation_id)
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let run_id = flow_run.id.clone();

    // Spawn async task for intent classification and routing
    tokio::spawn(async move {
        // Load control files
        let control_files = match ControlFiles::load() {
            Ok(files) => files,
            Err(e) => {
                let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                    node: "system".to_string(),
                    message: format!("Failed to load control files: {}", e),
                    timestamp: Utc::now(),
                }).await;
                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.update_flow_run_status(&run_id, "failed");
                }
                return;
            }
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

        let message_to_process = if actual_message.is_empty() {
            message.clone()
        } else {
            actual_message
        };

        // Log user message as episodic memory (async, non-blocking)
        if let Some(memory_service) = runtime.memory_service.as_ref() {
            let _ = memory_service.queue_episode(
                format!("User: {}", message_to_process),
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
            match memory_service.semantic_search(&message_to_process, 5).await {
                Ok(memories) => {
                    let context: Vec<String> = memories.iter()
                        .filter(|m| m.kind != MemoryKind::Episodic)
                        .map(|m| m.content.clone())
                        .collect();
                    if !context.is_empty() {
                        Some(format!("Relevant Memory Context:\n{}", context.join("\n")))
                    } else {
                        None
                    }
                }
                Err(_) => None
            }
        } else {
            None
        };

        // Classify intent
        let classifier = match IntentClassifier::new() {
            Ok(c) => c,
            Err(e) => {
                let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                    node: "system".to_string(),
                    message: format!("Failed to create intent classifier: {}", e),
                    timestamp: Utc::now(),
                }).await;
                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.update_flow_run_status(&run_id, "failed");
                }
                return;
            }
        };

        let classification = match classifier.classify_with_override(&message_to_process, override_intent).await {
            Ok(result) => result,
            Err(e) => {
                let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                    node: "system".to_string(),
                    message: format!("Intent classification failed: {}", e),
                    timestamp: Utc::now(),
                }).await;
                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.update_flow_run_status(&run_id, "failed");
                }
                return;
            }
        };

        let handler = IntentRouter::route(classification.intent);

        // Emit routing decision
        let _ = ws_manager.send_event(&run_id, FlowEvent::Output {
            node: "system".to_string(),
            data: format!("Intent: {} (confidence: {:.2})", classification.intent.display_name(), classification.confidence),
            timestamp: Utc::now(),
        }).await;

        // Route based on handler
        match handler {
            Handler::DirectLlm => {
                if let Err(e) = execute_direct_llm(&message_to_process, &conversation_id, &run_id, &db_path, &ws_manager, &control_files).await {
                    eprintln!("Direct LLM execution failed: {}", e);
                }
            }
            Handler::Approval => {
                if let Err(e) = execute_approval(&message_to_process, &conversation_id, &run_id, &db_path, &ws_manager, &state).await {
                    eprintln!("Approval execution failed: {}", e);
                }
            }
            Handler::Planning => {
                if let Err(e) = execute_planning(&message_to_process, &conversation_id, &run_id, &db_path, &ws_manager).await {
                    eprintln!("Planning execution failed: {}", e);
                }
            }
            Handler::CodeGenFlow => {
                let config = match AppConfig::load() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                            node: "system".to_string(),
                            message: format!("Failed to load config: {}", e),
                            timestamp: Utc::now(),
                        }).await;
                        if let Ok(db) = Db::new(&db_path) {
                            let _ = db.update_flow_run_status(&run_id, "failed");
                        }
                        return;
                    }
                };

                let llm_client = match LlmClient::from_config(&config) {
                    Ok(client) => client,
                    Err(e) => {
                        let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                            node: "system".to_string(),
                            message: format!("Failed to create LLM client: {}", e),
                            timestamp: Utc::now(),
                        }).await;
                        if let Ok(db) = Db::new(&db_path) {
                            let _ = db.update_flow_run_status(&run_id, "failed");
                        }
                        return;
                    }
                };

                if let Err(e) = execute_codegen_flow(&message_to_process, &conversation_id, &run_id, &db_path, &ws_manager, &llm_client, &state).await {
                    eprintln!("CodeGenFlow execution failed: {}", e);
                }
            }
            _ => {
                let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                    node: "system".to_string(),
                    message: format!("Handler {:?} not yet implemented", handler),
                    timestamp: Utc::now(),
                }).await;
                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.update_flow_run_status(&run_id, "failed");
                }
            }
        }
    });

    Ok(Json(flow_run))
}
