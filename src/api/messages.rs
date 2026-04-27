//! Message endpoints

use axum::{
    Json,
    extract::{Path, State},
};
use std::sync::Arc;

use crate::api::AppState;
use crate::db::repository::Repository;
use crate::db::{CreateMessage, Db, Message};

/// Get all messages for a conversation
pub async fn get_messages(
    State(state): State<Arc<AppState>>,
    Path(conversation_id): Path<String>,
) -> Result<Json<Vec<Message>>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.get_messages(&conversation_id) {
        Ok(messages) => Ok(Json(messages)),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Create a new message
pub async fn create_message(
    State(state): State<Arc<AppState>>,
    Json(input): Json<CreateMessage>,
) -> Result<Json<Message>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.create_message(input) {
        Ok(message) => Ok(Json(message)),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}
