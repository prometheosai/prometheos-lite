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
    artifact::Artifact,
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

/// Request to submit a user intent
#[derive(Debug, Deserialize)]
pub struct SubmitIntentRequest {
    pub user_id: String,
    pub message: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
}

/// Request to run a WorkContext until blocked or complete
#[derive(Debug, Deserialize)]
pub struct RunContextRequest {
    #[serde(default)]
    pub max_iterations: Option<u32>,
    #[serde(default)]
    pub max_runtime_ms: Option<u64>,
    #[serde(default)]
    pub max_tool_calls: Option<u32>,
    #[serde(default)]
    pub max_cost: Option<f64>,
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

/// Response with Artifact details
#[derive(Debug, Serialize)]
pub struct ArtifactResponse {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub created_by: String,
    pub storage_type: String,
    pub file_path: Option<String>,
    pub created_at: String,
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

/// Get artifacts for a WorkContext
pub async fn get_work_context_artifacts(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ArtifactResponse>>, StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let work_context_service = WorkContextService::new(Arc::new(db));

    let context = work_context_service
        .get_context(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let response: Vec<ArtifactResponse> = context
        .artifacts
        .into_iter()
        .map(|artifact| {
            let storage_type = match &artifact.storage {
                crate::work::artifact::ArtifactStorage::Inline => "inline".to_string(),
                crate::work::artifact::ArtifactStorage::FilePath(path) => format!("file:{}", path),
            };
            let file_path = match &artifact.storage {
                crate::work::artifact::ArtifactStorage::FilePath(path) => Some(path.clone()),
                _ => None,
            };
            ArtifactResponse {
                id: artifact.id,
                kind: format!("{:?}", artifact.kind),
                name: artifact.name,
                created_by: artifact.created_by,
                storage_type,
                file_path,
                created_at: artifact.created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(response))
}

/// Continue a WorkContext
///
/// This endpoint continues a blocked WorkContext using the WorkOrchestrator.
pub async fn continue_work_context(
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

/// Submit a user intent to create or attach to a WorkContext
pub async fn submit_intent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SubmitIntentRequest>,
) -> Result<Json<WorkContextResponse>, StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let work_context_service = WorkContextService::new(Arc::new(db));

    let domain = match req.message.to_lowercase().as_str() {
        msg if msg.contains("code") || msg.contains("implement") => WorkDomain::Software,
        _ => WorkDomain::General,
    };

    let context = work_context_service
        .create_context(req.user_id, req.message.clone(), domain, req.message)
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

/// Run a WorkContext until blocked or complete
pub async fn run_until_complete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<RunContextRequest>,
) -> Result<Json<WorkContextResponse>, StatusCode> {
    let db = Db::new(&state.db_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let work_context_service = WorkContextService::new(Arc::new(db));

    let mut context = work_context_service
        .get_context(&id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Update status to in_progress
    work_context_service
        .update_status(&mut context, WorkStatus::InProgress)
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
