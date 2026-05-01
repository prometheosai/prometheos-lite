//! WorkContext API endpoints

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::api::state::AppState;
use crate::work::{
    WorkContextService,
    types::{WorkContext, WorkDomain, WorkStatus},
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
    pub max_iterations: Option<usize>,
    #[serde(default)]
    pub max_runtime_ms: Option<u64>,
    #[serde(default)]
    pub max_tool_calls: Option<usize>,
    #[serde(default)]
    pub max_cost: Option<f64>,
}

/// Request to update WorkContext status
#[derive(Debug, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: String,
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

impl From<crate::work::types::WorkContext> for WorkContextResponse {
    fn from(context: crate::work::types::WorkContext) -> Self {
        Self {
            id: context.id,
            title: context.title,
            domain: format!("{:?}", context.domain),
            goal: context.goal,
            status: format!("{:?}", context.status),
            phase: format!("{:?}", context.current_phase),
            created_at: context.created_at.to_rfc3339(),
            updated_at: context.updated_at.to_rfc3339(),
        }
    }
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

/// Custom API error type that implements IntoResponse
#[derive(Debug, Clone)]
pub enum ApiError {
    Internal(String),
    NotFound(String),
    BadRequest(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        ApiError::Internal(err.to_string())
    }
}

/// List WorkContexts
pub async fn list_work_contexts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<WorkContextResponse>>, ApiError> {
    let work_context_service = state
        .create_work_context_service()
        .map_err(|e| ApiError::Internal(format!("Failed to create service: {}", e)))?;

    let contexts = work_context_service
        .list_contexts("api-user")
        .map_err(|e| ApiError::Internal(format!("Failed to list contexts: {}", e)))?;

    let response = contexts
        .into_iter()
        .map(WorkContextResponse::from)
        .collect();

    Ok(Json(response))
}

/// Get a specific WorkContext
pub async fn get_work_context(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkContextResponse>, ApiError> {
    let work_context_service = state
        .create_work_context_service()
        .map_err(|e| ApiError::Internal(format!("Failed to create service: {}", e)))?;

    let context = work_context_service
        .get_context(&id)
        .map_err(|e| ApiError::Internal(format!("Failed to get context: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("WorkContext not found: {}", id)))?;

    Ok(Json(WorkContextResponse::from(context)))
}

/// Create a new WorkContext
pub async fn create_work_context(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateWorkContextRequest>,
) -> Result<Json<WorkContextResponse>, ApiError> {
    let work_context_service = state
        .create_work_context_service()
        .map_err(|e| ApiError::Internal(format!("Failed to create service: {}", e)))?;

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
        .map_err(|e| ApiError::Internal(format!("Failed to create context: {}", e)))?;

    Ok(Json(WorkContextResponse::from(context)))
}

/// Update WorkContext status
pub async fn update_work_context_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateStatusRequest>,
) -> Result<Json<WorkContextResponse>, ApiError> {
    let work_context_service = state
        .create_work_context_service()
        .map_err(|e| ApiError::Internal(format!("Failed to create service: {}", e)))?;

    let mut context = work_context_service
        .get_context(&id)
        .map_err(|e| ApiError::Internal(format!("Failed to get context: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("WorkContext not found: {}", id)))?;

    let new_status = match req.status.to_lowercase().as_str() {
        "draft" => WorkStatus::Draft,
        "in_progress" => WorkStatus::InProgress,
        "awaiting_approval" => WorkStatus::AwaitingApproval,
        "completed" => WorkStatus::Completed,
        "blocked" => WorkStatus::Blocked,
        _ => {
            return Err(ApiError::BadRequest(format!(
                "Invalid status: {}",
                req.status
            )));
        }
    };

    work_context_service
        .update_status(&mut context, new_status)
        .map_err(|e| ApiError::Internal(format!("Failed to update status: {}", e)))?;

    Ok(Json(WorkContextResponse::from(context)))
}

/// Get artifacts for a WorkContext
pub async fn get_work_context_artifacts(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ArtifactResponse>>, ApiError> {
    let work_context_service = state
        .create_work_context_service()
        .map_err(|e| ApiError::Internal(format!("Failed to create service: {}", e)))?;

    let context = work_context_service
        .get_context(&id)
        .map_err(|e| ApiError::Internal(format!("Failed to get context: {}", e)))?
        .ok_or_else(|| ApiError::NotFound(format!("WorkContext not found: {}", id)))?;

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
pub async fn continue_work_context(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkContextResponse>, ApiError> {
    let orchestrator = state
        .create_work_orchestrator()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let context = orchestrator
        .continue_context(id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(WorkContextResponse::from(context)))
}

/// Submit a user intent to create or attach to a WorkContext
pub async fn submit_intent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SubmitIntentRequest>,
) -> Result<Json<WorkContextResponse>, ApiError> {
    let orchestrator = state
        .create_work_orchestrator()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let context = orchestrator
        .submit_user_intent(req.user_id, req.message, req.conversation_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(WorkContextResponse::from(context)))
}

/// Run a WorkContext until blocked or complete
pub async fn run_until_complete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<RunContextRequest>,
) -> Result<Json<WorkContextResponse>, ApiError> {
    let orchestrator = state
        .create_work_orchestrator()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let limits = crate::work::orchestrator::ExecutionLimits {
        max_iterations: req.max_iterations.unwrap_or(10) as u32,
        max_runtime_ms: req.max_runtime_ms.unwrap_or(300_000),
        max_tool_calls: req.max_tool_calls.unwrap_or(50) as u32,
        max_cost: req.max_cost.unwrap_or(1.0),
        ..Default::default()
    };
    let context = orchestrator
        .run_until_blocked_or_complete(id, limits)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(WorkContextResponse::from(context)))
}
