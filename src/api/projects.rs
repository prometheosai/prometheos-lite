//! Project endpoints

use axum::{Json, extract::State};
use std::sync::Arc;

use crate::api::AppState;
use crate::db::repository::Repository;
use crate::db::{CreateProject, Db, Project};

/// Get all projects
pub async fn get_projects(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Project>>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.get_projects() {
        Ok(projects) => Ok(Json(projects)),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Create a new project
pub async fn create_project(
    State(state): State<Arc<AppState>>,
    Json(input): Json<CreateProject>,
) -> Result<Json<Project>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.create_project(input) {
        Ok(project) => Ok(Json(project)),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}
