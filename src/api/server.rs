//! HTTP API server for PrometheOS Lite
//!
//! This module provides the Axum-based HTTP server with REST endpoints
//! and WebSocket support for the local chat interface.

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

use crate::api::AppState;
use crate::api::websocket::websocket_handler;
use crate::db::{Db, Repository};
use crate::db::models::{CreateConversation, CreateMessage, CreateProject, RunFlow};

/// Health check response
#[derive(serde::Serialize)]
struct HealthResponse {
    status: String,
}

/// Health check endpoint
///
/// Returns a simple JSON response indicating the server is running.
async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

// === Project Endpoints ===

/// Get all projects
async fn get_projects(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<crate::db::Project>>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.get_projects() {
        Ok(projects) => Ok(Json(projects)),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Create a new project
async fn create_project(
    State(state): State<Arc<AppState>>,
    Json(input): Json<CreateProject>,
) -> Result<Json<crate::db::Project>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.create_project(input) {
        Ok(project) => Ok(Json(project)),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// === Conversation Endpoints ===

/// Get all conversations for a project
async fn get_conversations(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Json<Vec<crate::db::Conversation>>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.get_conversations(&project_id) {
        Ok(conversations) => Ok(Json(conversations)),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Create a new conversation
async fn create_conversation(
    State(state): State<Arc<AppState>>,
    Json(input): Json<CreateConversation>,
) -> Result<Json<crate::db::Conversation>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.create_conversation(input) {
        Ok(conversation) => Ok(Json(conversation)),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// === Message Endpoints ===

/// Get all messages for a conversation
async fn get_messages(
    State(state): State<Arc<AppState>>,
    Path(conversation_id): Path<String>,
) -> Result<Json<Vec<crate::db::Message>>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.get_messages(&conversation_id) {
        Ok(messages) => Ok(Json(messages)),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Create a new message
async fn create_message(
    State(state): State<Arc<AppState>>,
    Json(input): Json<CreateMessage>,
) -> Result<Json<crate::db::Message>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.create_message(input) {
        Ok(message) => Ok(Json(message)),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// === Flow Execution Endpoints ===

/// Run a flow for a conversation
async fn run_flow(
    State(state): State<Arc<AppState>>,
    Path(conversation_id): Path<String>,
    Json(input): Json<RunFlow>,
) -> Result<Json<crate::db::FlowRun>, axum::http::StatusCode> {
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
        use crate::api::websocket::FlowEvent;
        use crate::llm::LlmClient;
        use crate::config::AppConfig;
        use crate::intent::{IntentClassifier, IntentRouter, Handler, Intent};
        use crate::control::ControlFiles;
        use chrono::Utc;

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
            // Strip the override command from the message
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

        // Use actual message for processing, or original if no override
        let message_to_process = if actual_message.is_empty() {
            message.clone()
        } else {
            actual_message
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
                // Direct LLM response for conversation/questions
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

                // Concise conversation prompt with control files
                let conversation_prompt = control_files.build_conversation_prompt(&message_to_process);

                let response = match llm_client.generate(&conversation_prompt).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                            node: "system".to_string(),
                            message: format!("LLM call failed: {}", e),
                            timestamp: Utc::now(),
                        }).await;
                        if let Ok(db) = Db::new(&db_path) {
                            let _ = db.update_flow_run_status(&run_id, "failed");
                        }
                        return;
                    }
                };

                // Save assistant message
                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.create_message(crate::db::CreateMessage {
                        conversation_id: conversation_id.clone(),
                        role: "assistant".to_string(),
                        content: response,
                    });
                }

                // Emit completion event
                let _ = ws_manager.send_event(&run_id, FlowEvent::Output {
                    node: "system".to_string(),
                    data: "Direct response completed".to_string(),
                    timestamp: Utc::now(),
                }).await;

                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.update_flow_run_status(&run_id, "completed");
                }
            }
            Handler::CodeGenFlow => {
                // Full code generation flow
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

                // Emit node_start event for planner
                let _ = ws_manager.send_event(&run_id, FlowEvent::NodeStart {
                    node: "planner".to_string(),
                    timestamp: Utc::now(),
                }).await;

                // Real LLM call for planning phase
                let planner_prompt = format!(
                    "You are a planning AI. Create a detailed plan for the following task:\n\n{}\n\nProvide a step-by-step plan with clear objectives.",
                    message_to_process
                );

                let plan = match llm_client.generate(&planner_prompt).await {
                    Ok(response) => response,
                    Err(e) => {
                        let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                            node: "planner".to_string(),
                            message: format!("LLM call failed: {}", e),
                            timestamp: Utc::now(),
                        }).await;
                        if let Ok(db) = Db::new(&db_path) {
                            let _ = db.update_flow_run_status(&run_id, "failed");
                        }
                        return;
                    }
                };

                let _ = ws_manager.send_event(&run_id, FlowEvent::Output {
                    node: "planner".to_string(),
                    data: plan.clone(),
                    timestamp: Utc::now(),
                }).await;

                let _ = ws_manager.send_event(&run_id, FlowEvent::NodeEnd {
                    node: "planner".to_string(),
                    timestamp: Utc::now(),
                }).await;

                // Emit node_start event for coder
                let _ = ws_manager.send_event(&run_id, FlowEvent::NodeStart {
                    node: "coder".to_string(),
                    timestamp: Utc::now(),
                }).await;

                // Real LLM call for coding phase
                let coder_prompt = format!(
                    "You are a coding AI. Generate code based on this plan:\n\n{}\n\nOriginal task: {}\n\nProvide clean, well-commented code.",
                    plan, message_to_process
                );

                let generated_code = match llm_client.generate(&coder_prompt).await {
                    Ok(response) => response,
                    Err(e) => {
                        let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                            node: "coder".to_string(),
                            message: format!("LLM call failed: {}", e),
                            timestamp: Utc::now(),
                        }).await;
                        if let Ok(db) = Db::new(&db_path) {
                            let _ = db.update_flow_run_status(&run_id, "failed");
                        }
                        return;
                    }
                };

                let _ = ws_manager.send_event(&run_id, FlowEvent::Output {
                    node: "coder".to_string(),
                    data: generated_code.clone(),
                    timestamp: Utc::now(),
                }).await;

                let _ = ws_manager.send_event(&run_id, FlowEvent::NodeEnd {
                    node: "coder".to_string(),
                    timestamp: Utc::now(),
                }).await;

                // Emit node_start event for reviewer
                let _ = ws_manager.send_event(&run_id, FlowEvent::NodeStart {
                    node: "reviewer".to_string(),
                    timestamp: Utc::now(),
                }).await;

                // Real LLM call for review phase
                let reviewer_prompt = format!(
                    "You are a code reviewer. Review this code:\n\n{}\n\nOriginal task: {}\n\nProvide constructive feedback.",
                    generated_code, message_to_process
                );

                let review = match llm_client.generate(&reviewer_prompt).await {
                    Ok(response) => response,
                    Err(e) => {
                        let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                            node: "reviewer".to_string(),
                            message: format!("LLM call failed: {}", e),
                            timestamp: Utc::now(),
                        }).await;
                        if let Ok(db) = Db::new(&db_path) {
                            let _ = db.update_flow_run_status(&run_id, "failed");
                        }
                        return;
                    }
                };

                let _ = ws_manager.send_event(&run_id, FlowEvent::Output {
                    node: "reviewer".to_string(),
                    data: review.clone(),
                    timestamp: Utc::now(),
                }).await;

                let _ = ws_manager.send_event(&run_id, FlowEvent::NodeEnd {
                    node: "reviewer".to_string(),
                    timestamp: Utc::now(),
                }).await;

                // Simulate memory write with skipped warning
                let _ = ws_manager.send_event(&run_id, FlowEvent::NodeStart {
                    node: "memory_write".to_string(),
                    timestamp: Utc::now(),
                }).await;

                let _ = ws_manager.send_event(&run_id, FlowEvent::Error {
                    node: "memory_write".to_string(),
                    message: "Memory write skipped: Embedding server unavailable".to_string(),
                    timestamp: Utc::now(),
                }).await;

                let _ = ws_manager.send_event(&run_id, FlowEvent::NodeEnd {
                    node: "memory_write".to_string(),
                    timestamp: Utc::now(),
                }).await;

                // Combine outputs for assistant response
                let assistant_response = format!(
                    "## Plan\n\n{}\n\n## Generated Code\n\n{}\n\n## Review\n\n{}",
                    plan, generated_code, review
                );

                // Save assistant message
                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.create_message(crate::db::CreateMessage {
                        conversation_id: conversation_id.clone(),
                        role: "assistant".to_string(),
                        content: assistant_response,
                    });
                }

                // Emit completion event
                let _ = ws_manager.send_event(&run_id, FlowEvent::Output {
                    node: "system".to_string(),
                    data: "Flow execution completed".to_string(),
                    timestamp: Utc::now(),
                }).await;

                // Update status to completed
                if let Ok(db) = Db::new(&db_path) {
                    let _ = db.update_flow_run_status(&run_id, "completed");
                }
            }
            _ => {
                // Placeholder handlers for other intents
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

/// Create the API router with all routes
pub fn create_router(state: Arc<AppState>) -> Router {
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers(tower_http::cors::Any);

    Router::new()
        .route("/health", get(health_check))
        .route("/projects", get(get_projects).post(create_project))
        .route("/projects/:id/conversations", get(get_conversations))
        .route("/conversations", post(create_conversation))
        .route("/conversations/:id/messages", get(get_messages))
        .route("/messages", post(create_message))
        .route("/conversations/:id/run", post(run_flow))
        .route("/ws/runs/:id", get(websocket_handler))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}

/// Run the API server
///
/// Starts the Axum server on the specified address and port.
pub async fn run_server(addr: SocketAddr, state: Arc<AppState>) -> anyhow::Result<()> {
    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("API server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
