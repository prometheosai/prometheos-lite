//! WorkContext API endpoints

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use crate::api::state::AppState;
use crate::harness::{
    HarnessWorkContextService, edit_protocol::EditOperation, mode_policy::HarnessMode,
    parse_edit_response,
};
use crate::work::types::{WorkDomain, WorkStatus};

/// Request to create a new WorkContext
#[derive(Debug, Deserialize)]
pub struct CreateWorkContextRequest {
    pub title: String,
    pub domain: String,
    pub user_id: String,
    pub goal: String,
}

/// Request to submit a user intent
#[derive(Debug, Deserialize)]
pub struct SubmitIntentRequest {
    pub user_id: String,
    pub message: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UserIdentityQuery {
    pub user_id: String,
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

#[derive(Debug, Deserialize)]
pub struct HarnessRunRequest {
    pub repo_root: PathBuf,
    #[serde(default = "default_harness_mode")]
    pub mode: HarnessMode,
    #[serde(default)]
    pub proposed_edits: Vec<EditOperation>,
    #[serde(default)]
    pub edit_response: Option<String>,
}
fn default_harness_mode() -> HarnessMode {
    HarnessMode::Review
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
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<Vec<WorkContextResponse>>, ApiError> {
    if identity.user_id.trim().is_empty() {
        return Err(ApiError::BadRequest("user_id is required".to_string()));
    }
    let work_context_service = state
        .create_work_context_service()
        .map_err(|e| ApiError::Internal(format!("Failed to create service: {}", e)))?;

    let contexts = work_context_service
        .list_contexts(&identity.user_id)
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
    if req.user_id.trim().is_empty() {
        return Err(ApiError::BadRequest("user_id is required".to_string()));
    }
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

    let context = work_context_service
        .create_context(req.user_id, req.title, domain, req.goal)
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
    if req.user_id.trim().is_empty() {
        return Err(ApiError::BadRequest("user_id is required".to_string()));
    }
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

pub async fn run_harness(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<HarnessRunRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let work_context_service = state
        .create_work_context_service()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let service = HarnessWorkContextService::new(work_context_service);
    let mut edits = req.proposed_edits;
    if let Some(raw) = req.edit_response.as_deref() {
        edits.extend(parse_edit_response(raw).map_err(|e| ApiError::BadRequest(e.to_string()))?);
    }
    let result = service
        .run_for_context(&id, req.repo_root, req.mode, edits)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(
        serde_json::to_value(result).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
}

pub async fn get_harness_metadata(
    State(_state): State<Arc<AppState>>,
    Path((_id, _view)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Err(ApiError::BadRequest(
        "Deprecated endpoint removed. Use explicit /harness/{evidence|patches|validation|review|risk|completion} endpoints.".to_string(),
    ))
}

fn harness_payload(ctx: &crate::work::types::WorkContext) -> serde_json::Value {
    ctx.metadata
        .get("harness")
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

async fn get_context_or_404(
    state: &Arc<AppState>,
    id: &str,
) -> Result<crate::work::types::WorkContext, ApiError> {
    let svc = state
        .create_work_context_service()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    svc.get_context(id)
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("WorkContext not found: {}", id)))
}

fn extract_harness_view(harness: &serde_json::Value, view: &str) -> Option<serde_json::Value> {
    match view {
        "evidence" => harness.get("evidence_log").cloned(),
        "patches" => harness.get("patch_result").cloned(),
        "validation" => harness.get("validation_result").cloned(),
        "review" => harness.get("review_issues").cloned(),
        "risk" => harness.get("risk_assessment").cloned(),
        "completion" => harness.get("completion_decision").cloned(),
        "trajectory" => harness.get("trajectory").cloned(),
        "artifacts" => harness.get("artifacts").cloned(),
        "confidence" => harness.get("confidence").cloned(),
        "replay" => Some(harness.clone()),
        _ => None,
    }
}

pub async fn get_harness_evidence(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ctx = get_context_or_404(&state, &id).await?;
    let harness = harness_payload(&ctx);
    Ok(Json(
        extract_harness_view(&harness, "evidence").unwrap_or(serde_json::Value::Null),
    ))
}

pub async fn get_harness_patches(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ctx = get_context_or_404(&state, &id).await?;
    let harness = harness_payload(&ctx);
    Ok(Json(
        extract_harness_view(&harness, "patches").unwrap_or(serde_json::Value::Null),
    ))
}

pub async fn get_harness_validation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ctx = get_context_or_404(&state, &id).await?;
    let harness = harness_payload(&ctx);
    Ok(Json(
        extract_harness_view(&harness, "validation").unwrap_or(serde_json::Value::Null),
    ))
}

pub async fn get_harness_review(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ctx = get_context_or_404(&state, &id).await?;
    let harness = harness_payload(&ctx);
    Ok(Json(
        extract_harness_view(&harness, "review").unwrap_or(serde_json::Value::Null),
    ))
}

pub async fn get_harness_risk(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ctx = get_context_or_404(&state, &id).await?;
    let harness = harness_payload(&ctx);
    Ok(Json(
        extract_harness_view(&harness, "risk").unwrap_or(serde_json::Value::Null),
    ))
}

pub async fn get_harness_completion(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ctx = get_context_or_404(&state, &id).await?;
    let harness = harness_payload(&ctx);
    Ok(Json(
        extract_harness_view(&harness, "completion").unwrap_or(serde_json::Value::Null),
    ))
}

pub async fn get_work_quality(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let svc = state
        .create_work_context_service()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let runs = svc
        .list_harness_run_metrics(&id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let latest = runs.first().cloned();
    Ok(Json(serde_json::json!({
        "work_context_id": id,
        "quality_metrics": latest.as_ref().map(|r| r.quality_metrics.clone()).unwrap_or_default(),
        "latest_run_id": latest.as_ref().map(|r| r.run_id.clone()),
        "runs": runs,
    })))
}

pub async fn get_work_cost(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let svc = state
        .create_work_context_service()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let runs = svc
        .list_harness_run_metrics(&id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let latest = runs.first().cloned();
    Ok(Json(serde_json::json!({
        "work_context_id": id,
        "token_usage": latest.as_ref().map(|r| r.token_usage.clone()).unwrap_or_default(),
        "latest_run_id": latest.as_ref().map(|r| r.run_id.clone()),
        "runs": runs,
    })))
}

pub async fn list_work_traces(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let svc = state
        .create_work_context_service()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let runs = svc
        .list_harness_run_metrics(&id)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(serde_json::json!({
        "work_context_id": id,
        "latest_run_id": runs.first().map(|r| r.run_id.clone()),
        "runs": runs
    })))
}

pub async fn get_trace_by_run(
    State(state): State<Arc<AppState>>,
    Path((id, run_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let svc = state
        .create_work_context_service()
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let run = svc
        .get_harness_run_metrics(&id, &run_id)
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Run '{}' not found for work context '{}'",
                run_id, id
            ))
        })?;
    Ok(Json(serde_json::json!({
        "work_context_id": id,
        "run_id": run_id,
        "trace_summary": run.trace_summary,
        "token_usage": run.token_usage,
        "quality_metrics": run.quality_metrics,
        "trajectory": run.trajectory,
        "created_at": run.created_at,
    })))
}

#[cfg(test)]
mod tests {
    use super::extract_harness_view;

    #[test]
    fn test_extract_harness_view_matrix() {
        let harness = serde_json::json!({
            "evidence_log": {"entries": []},
            "patch_result": {"applied": true},
            "validation_result": {"passed": true},
            "review_issues": [],
            "risk_assessment": {"level": "low"},
            "completion_decision": "Complete",
            "trajectory": {"id": "run-1"},
            "artifacts": [],
            "confidence": {"score": 0.9}
        });

        for view in [
            "evidence",
            "patches",
            "validation",
            "review",
            "risk",
            "completion",
            "trajectory",
            "artifacts",
            "confidence",
            "replay",
        ] {
            assert!(
                extract_harness_view(&harness, view).is_some(),
                "expected view '{}' to resolve",
                view
            );
        }
        assert!(extract_harness_view(&harness, "unknown").is_none());
    }
}
