//! Route registration for the API server

use axum::{
    Router,
    routing::{get, post},
};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

use crate::api::AppState;
use crate::api::conversations::{create_conversation, get_conversations};
use crate::api::flow_runs::run_flow;
use crate::api::health::health_check;
use crate::api::messages::{create_message, get_messages};
use crate::api::projects::{create_project, get_projects};
use crate::api::websocket::websocket_handler;
use crate::api::work_contexts::{
    continue_work_context, create_work_context, get_work_context, get_work_context_artifacts,
    list_work_contexts, run_until_complete, submit_intent, update_work_context_status,
};

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
        .route("/work-contexts", get(list_work_contexts).post(create_work_context))
        .route("/work-contexts/submit-intent", post(submit_intent))
        .route("/work-contexts/:id", get(get_work_context))
        .route("/work-contexts/:id/status", post(update_work_context_status))
        .route("/work-contexts/:id/artifacts", get(get_work_context_artifacts))
        .route("/work-contexts/:id/continue", post(continue_work_context))
        .route("/work-contexts/:id/run-until-complete", post(run_until_complete))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
