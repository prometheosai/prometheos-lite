//! Conversation endpoints

use axum::{
    Json,
    extract::{Path, State},
};
use std::sync::Arc;

use crate::api::AppState;
use crate::db::repository::Repository;
use crate::db::{Conversation, CreateConversation, Db};

/// Get all conversations for a project
pub async fn get_conversations(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Json<Vec<Conversation>>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.get_conversations(&project_id) {
        Ok(conversations) => Ok(Json(conversations)),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Create a new conversation
pub async fn create_conversation(
    State(state): State<Arc<AppState>>,
    Json(input): Json<CreateConversation>,
) -> Result<Json<Conversation>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.create_conversation(input) {
        Ok(conversation) => Ok(Json(conversation)),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}
