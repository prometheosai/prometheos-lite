use axum::{Json, extract::Path, extract::State, http::StatusCode};
use std::sync::Arc;

use crate::api::AppState;
use crate::db::repository::Repository;
use crate::db::{CreateProject, Db, Project};

pub async fn get_projects(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Project>>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.get_projects() {
        Ok(projects) => Ok(Json(projects)),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub async fn get_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Project>, axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.get_project(&id) {
        Ok(Some(project)) => Ok(Json(project)),
        Ok(None) => Err(axum::http::StatusCode::NOT_FOUND),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub async fn create_project(
    State(state): State<Arc<AppState>>,
    Json(input): Json<CreateProject>,
) -> Result<(StatusCode, Json<Project>), axum::http::StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    match db.create_project(input) {
        Ok(project) => Ok((StatusCode::CREATED, Json(project))),
        Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
    }
}
