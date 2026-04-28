//! WorkContext API endpoints

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::api::state::AppState;
use crate::db::Db;
use crate::work::{
    types::{WorkDomain, WorkStatus},
    WorkContextService,
};

/// Request to create a new WorkContext
#[derive(Debug, Deserialize)]
pub struct CreateWorkContextRequest {
    pub title: String,
    pub domain: String,
    pub goal: String,
    #[serde(default)]
    pub user_id: String,
}

/// Response with WorkContext details
#[derive(Debug, Serialize)]
pub struct WorkContextResponse {
    pub id: String,
    pub title: String,
    pub domain: String,
    pub goal: String,
    pub status: String,
    pub phase: String,
    pub created_at: String,
    pub updated_at: String,
}

/// List WorkContexts
pub async fn list_work_contexts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<WorkContextResponse>>, StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let work_context_service = WorkContextService::new(Arc::new(db));

    let contexts = work_context_service
        .list_contexts("api-user")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response: Vec<WorkContextResponse> = contexts
        .into_iter()
        .map(|ctx| WorkContextResponse {
            id: ctx.id,
            title: ctx.title,
            domain: format!("{:?}", ctx.domain),
            goal: ctx.goal,
            status: format!("{:?}", ctx.status),
            phase: format!("{:?}", ctx.current_phase),
            created_at: ctx.created_at.to_rfc3339(),
            updated_at: ctx.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(response))
}

/// Get a specific WorkContext
pub async fn get_work_context(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkContextResponse>, StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let work_context_service = WorkContextService::new(Arc::new(db));

    let context = work_context_service
        .get_context(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let response = WorkContextResponse {
        id: context.id,
        title: context.title,
        domain: format!("{:?}", context.domain),
        goal: context.goal,
        status: format!("{:?}", context.status),
        phase: format!("{:?}", context.current_phase),
        created_at: context.created_at.to_rfc3339(),
        updated_at: context.updated_at.to_rfc3339(),
    };

    Ok(Json(response))
}

/// Create a new WorkContext
pub async fn create_work_context(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateWorkContextRequest>,
) -> Result<Json<WorkContextResponse>, StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let work_context_service = WorkContextService::new(Arc::new(db));

    let domain = match req.domain.to_lowercase().as_str() {
        "software" => WorkDomain::Software,
        "business" => WorkDomain::Business,
        "marketing" => WorkDomain::Marketing,
        "personal" => WorkDomain::Personal,
        "creative" => WorkDomain::Creative,
        "research" => WorkDomain::Research,
        "operations" => WorkDomain::Operations,
        _ => WorkDomain::General,
    };

    let user_id = if req.user_id.is_empty() {
        "api-user".to_string()
    } else {
        req.user_id
    };

    let context = work_context_service
        .create_context(user_id, req.title, domain, req.goal)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response = WorkContextResponse {
        id: context.id,
        title: context.title,
        domain: format!("{:?}", context.domain),
        goal: context.goal,
        status: format!("{:?}", context.status),
        phase: format!("{:?}", context.current_phase),
        created_at: context.created_at.to_rfc3339(),
        updated_at: context.updated_at.to_rfc3339(),
    };

    Ok(Json(response))
}

/// Request to update WorkContext status
#[derive(Debug, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: String,
}

/// Update WorkContext status
pub async fn update_work_context_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateStatusRequest>,
) -> Result<Json<WorkContextResponse>, StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let work_context_service = WorkContextService::new(Arc::new(db));

    let mut context = work_context_service
        .get_context(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let new_status = match req.status.to_lowercase().as_str() {
        "draft" => WorkStatus::Draft,
        "in_progress" => WorkStatus::InProgress,
        "awaiting_approval" => WorkStatus::AwaitingApproval,
        "completed" => WorkStatus::Completed,
        "blocked" => WorkStatus::Blocked,
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    work_context_service
        .update_status(&mut context, new_status)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response = WorkContextResponse {
        id: context.id,
        title: context.title,
        domain: format!("{:?}", context.domain),
        goal: context.goal,
        status: format!("{:?}", context.status),
        phase: format!("{:?}", context.current_phase),
        created_at: context.created_at.to_rfc3339(),
        updated_at: context.updated_at.to_rfc3339(),
    };

    Ok(Json(response))
}
