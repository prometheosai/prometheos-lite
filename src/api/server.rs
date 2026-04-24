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

/// Run flow for a conversation
async fn run_flow(
    State(state): State<Arc<AppState>>,
    Path(conversation_id): Path<String>,
    Json(input): Json<RunFlow>,
) -> Result<Json<crate::db::FlowRun>, axum::http::StatusCode> {
    let db_path = state.db_path.clone();
    let message = input.message.clone();
    let ws_manager = state.ws_manager.clone();

    // Save user message
    let db = Db::new(&db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = db.create_message(crate::db::CreateMessage {
        conversation_id: conversation_id.clone(),
        role: "user".to_string(),
        content: message,
    });

    // Create FlowRun
    let flow_run = db.create_flow_run(&conversation_id)
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let run_id = flow_run.id.clone();

    // Spawn async task for flow execution with WebSocket events
    tokio::spawn(async move {
        use crate::api::websocket::FlowEvent;
        use chrono::Utc;

        // Emit node_start event
        let _ = ws_manager.send_event(&run_id, FlowEvent::NodeStart {
            node: "planner".to_string(),
            timestamp: Utc::now(),
        }).await;

        // Simulate flow execution
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        // Emit output event
        let _ = ws_manager.send_event(&run_id, FlowEvent::Output {
            node: "planner".to_string(),
            data: "Planning completed".to_string(),
            timestamp: Utc::now(),
        }).await;

        // Emit node_end event
        let _ = ws_manager.send_event(&run_id, FlowEvent::NodeEnd {
            node: "planner".to_string(),
            timestamp: Utc::now(),
        }).await;

        // Simulate more nodes
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        let _ = ws_manager.send_event(&run_id, FlowEvent::NodeStart {
            node: "coder".to_string(),
            timestamp: Utc::now(),
        }).await;

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        let _ = ws_manager.send_event(&run_id, FlowEvent::Output {
            node: "coder".to_string(),
            data: "Code generated".to_string(),
            timestamp: Utc::now(),
        }).await;

        let _ = ws_manager.send_event(&run_id, FlowEvent::NodeEnd {
            node: "coder".to_string(),
            timestamp: Utc::now(),
        }).await;

        // Update status to completed
        if let Ok(db) = Db::new(&db_path) {
            let _ = db.update_flow_run_status(&run_id, "completed");
        }
    });

    Ok(Json(flow_run))
}

/// Create the API router with all routes
pub fn create_router(state: Arc<AppState>) -> Router {
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
