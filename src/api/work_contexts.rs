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
    Forbidden(String),
    Conflict(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg),
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
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<WorkContextResponse>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let context = get_context_for_user_or_404(&state, &id, user_id).await?;
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
    Query(identity): Query<UserIdentityQuery>,
    Json(req): Json<UpdateStatusRequest>,
) -> Result<Json<WorkContextResponse>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let work_context_service = state
        .create_work_context_service()
        .map_err(|e| ApiError::Internal(format!("Failed to create service: {}", e)))?;

    let mut context = get_context_for_user_or_404(&state, &id, user_id).await?;

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
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<Vec<ArtifactResponse>>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let context = get_context_for_user_or_404(&state, &id, user_id).await?;

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
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<WorkContextResponse>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let _context = get_context_for_user_or_404(&state, &id, user_id).await?;
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
    Query(identity): Query<UserIdentityQuery>,
    Json(req): Json<RunContextRequest>,
) -> Result<Json<WorkContextResponse>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let _context = get_context_for_user_or_404(&state, &id, user_id).await?;
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
    Query(identity): Query<UserIdentityQuery>,
    Json(req): Json<HarnessRunRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let _context = get_context_for_user_or_404(&state, &id, user_id).await?;
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

fn harness_payload(ctx: &crate::work::types::WorkContext) -> serde_json::Value {
    ctx.metadata
        .get("harness")
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

fn required_user_id(identity: &UserIdentityQuery) -> Result<&str, ApiError> {
    let user_id = identity.user_id.trim();
    if user_id.is_empty() {
        return Err(ApiError::BadRequest("user_id is required".to_string()));
    }
    Ok(user_id)
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

async fn get_context_for_user_or_404(
    state: &Arc<AppState>,
    id: &str,
    user_id: &str,
) -> Result<crate::work::types::WorkContext, ApiError> {
    let context = get_context_or_404(state, id).await?;
    if context.user_id != user_id {
        return Err(ApiError::Forbidden(format!(
            "work context '{}' does not belong to user '{}'",
            id, user_id
        )));
    }
    Ok(context)
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

fn required_harness_view(
    harness: &serde_json::Value,
    view: &str,
) -> Result<serde_json::Value, ApiError> {
    extract_harness_view(harness, view).ok_or_else(|| {
        ApiError::Conflict(format!(
            "harness '{}' view is not available for this work context yet",
            view
        ))
    })
}

pub async fn get_harness_evidence(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let ctx = get_context_for_user_or_404(&state, &id, user_id).await?;
    let harness = harness_payload(&ctx);
    Ok(Json(required_harness_view(&harness, "evidence")?))
}

pub async fn get_harness_patches(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let ctx = get_context_for_user_or_404(&state, &id, user_id).await?;
    let harness = harness_payload(&ctx);
    Ok(Json(required_harness_view(&harness, "patches")?))
}

pub async fn get_harness_validation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let ctx = get_context_for_user_or_404(&state, &id, user_id).await?;
    let harness = harness_payload(&ctx);
    Ok(Json(required_harness_view(&harness, "validation")?))
}

pub async fn get_harness_review(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let ctx = get_context_for_user_or_404(&state, &id, user_id).await?;
    let harness = harness_payload(&ctx);
    Ok(Json(required_harness_view(&harness, "review")?))
}

pub async fn get_harness_risk(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let ctx = get_context_for_user_or_404(&state, &id, user_id).await?;
    let harness = harness_payload(&ctx);
    Ok(Json(required_harness_view(&harness, "risk")?))
}

pub async fn get_harness_completion(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let ctx = get_context_for_user_or_404(&state, &id, user_id).await?;
    let harness = harness_payload(&ctx);
    Ok(Json(required_harness_view(&harness, "completion")?))
}

pub async fn get_work_quality(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let _context = get_context_for_user_or_404(&state, &id, user_id).await?;
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
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let _context = get_context_for_user_or_404(&state, &id, user_id).await?;
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
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let _context = get_context_for_user_or_404(&state, &id, user_id).await?;
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
    Query(identity): Query<UserIdentityQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user_id = required_user_id(&identity)?;
    let _context = get_context_for_user_or_404(&state, &id, user_id).await?;
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
    use super::{
        ApiError, HarnessRunRequest, RunContextRequest, UpdateStatusRequest, UserIdentityQuery,
        continue_work_context, extract_harness_view, get_context_for_user_or_404,
        get_harness_completion, get_harness_evidence, get_harness_patches, get_harness_review,
        get_harness_risk, get_harness_validation, get_trace_by_run, get_work_context,
        get_work_context_artifacts, get_work_cost, get_work_quality, list_work_contexts,
        list_work_traces, required_harness_view, required_user_id, run_harness, run_until_complete,
        update_work_context_status,
    };
    use crate::api::state::AppState;
    use crate::flow::memory::db::MemoryDb;
    use crate::flow::memory::embedding::LocalEmbeddingProvider;
    use crate::flow::memory::service::MemoryService;
    use crate::flow::runtime::RuntimeContext;
    use crate::work::types::WorkDomain;
    use axum::Json;
    use axum::extract::{Path, Query, State};
    use std::sync::Arc;
    use tempfile::tempdir;

    struct TestState {
        state: Arc<AppState>,
        context_id: String,
        _db_dir: tempfile::TempDir,
    }

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

    #[test]
    fn test_required_harness_view_errors_when_missing() {
        let harness = serde_json::json!({
            "evidence_log": {"entries": []}
        });
        let err = required_harness_view(&harness, "completion").unwrap_err();
        assert!(matches!(err, super::ApiError::Conflict(_)));
    }

    #[test]
    fn test_required_user_id_validation() {
        let ok = UserIdentityQuery {
            user_id: "user-1".to_string(),
        };
        assert_eq!(required_user_id(&ok).unwrap(), "user-1");

        let bad = UserIdentityQuery {
            user_id: "   ".to_string(),
        };
        assert!(matches!(
            required_user_id(&bad).unwrap_err(),
            super::ApiError::BadRequest(_)
        ));
    }

    fn test_state() -> TestState {
        let db_dir = tempdir().expect("temp db dir");
        let db_path = db_dir
            .path()
            .join("work_contexts_test.db")
            .to_str()
            .expect("db path")
            .to_string();
        let runtime = Arc::new(RuntimeContext::new());
        let embedding: Arc<dyn crate::flow::EmbeddingProvider> = Arc::new(
            LocalEmbeddingProvider::new("http://127.0.0.1:9/embeddings".to_string(), 8),
        );
        let memory_service = Arc::new(MemoryService::new(
            MemoryDb::in_memory().expect("in-memory memory db"),
            Box::new(LocalEmbeddingProvider::new(
                "http://127.0.0.1:9/embeddings".to_string(),
                8,
            )),
        ));
        let state = Arc::new(
            AppState::new(db_path, runtime, embedding, memory_service).expect("app state"),
        );
        let service = state
            .create_work_context_service()
            .expect("work context service");
        let context = service
            .create_context(
                "user-1".to_string(),
                "Owned Context".to_string(),
                WorkDomain::Software,
                "goal".to_string(),
            )
            .expect("create context");
        TestState {
            state,
            context_id: context.id,
            _db_dir: db_dir,
        }
    }

    #[tokio::test]
    async fn test_context_ownership_guard() {
        let test_state = test_state();
        let err = get_context_for_user_or_404(&test_state.state, &test_state.context_id, "user-2")
            .await
            .expect_err("must reject foreign user");
        assert!(matches!(err, ApiError::Forbidden(_)));
    }

    #[tokio::test]
    async fn test_list_and_get_routes_enforce_user_identity() {
        let test_state = test_state();

        let missing_list = list_work_contexts(
            State(test_state.state.clone()),
            Query(UserIdentityQuery {
                user_id: " ".to_string(),
            }),
        )
        .await
        .expect_err("missing user must fail");
        assert!(matches!(missing_list, ApiError::BadRequest(_)));

        let owned_list = list_work_contexts(
            State(test_state.state.clone()),
            Query(UserIdentityQuery {
                user_id: "user-1".to_string(),
            }),
        )
        .await
        .expect("owner can list");
        assert!(!owned_list.0.is_empty());

        let wrong_get = get_work_context(
            State(test_state.state.clone()),
            Path(test_state.context_id.clone()),
            Query(UserIdentityQuery {
                user_id: "user-2".to_string(),
            }),
        )
        .await
        .expect_err("foreign user must fail");
        assert!(matches!(wrong_get, ApiError::Forbidden(_)));

        let missing_get = get_work_context(
            State(test_state.state),
            Path(test_state.context_id),
            Query(UserIdentityQuery {
                user_id: "".to_string(),
            }),
        )
        .await
        .expect_err("missing user must fail");
        assert!(matches!(missing_get, ApiError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_artifacts_and_metrics_routes_enforce_ownership() {
        let test_state = test_state();

        let wrong_artifacts = get_work_context_artifacts(
            State(test_state.state.clone()),
            Path(test_state.context_id.clone()),
            Query(UserIdentityQuery {
                user_id: "user-2".to_string(),
            }),
        )
        .await
        .expect_err("foreign user must fail");
        assert!(matches!(wrong_artifacts, ApiError::Forbidden(_)));

        let wrong_quality = get_work_quality(
            State(test_state.state.clone()),
            Path(test_state.context_id.clone()),
            Query(UserIdentityQuery {
                user_id: "user-2".to_string(),
            }),
        )
        .await
        .expect_err("foreign user must fail");
        assert!(matches!(wrong_quality, ApiError::Forbidden(_)));

        let wrong_cost = get_work_cost(
            State(test_state.state.clone()),
            Path(test_state.context_id.clone()),
            Query(UserIdentityQuery {
                user_id: "user-2".to_string(),
            }),
        )
        .await
        .expect_err("foreign user must fail");
        assert!(matches!(wrong_cost, ApiError::Forbidden(_)));

        let wrong_traces = list_work_traces(
            State(test_state.state),
            Path(test_state.context_id),
            Query(UserIdentityQuery {
                user_id: "user-2".to_string(),
            }),
        )
        .await
        .expect_err("foreign user must fail");
        assert!(matches!(wrong_traces, ApiError::Forbidden(_)));
    }

    #[tokio::test]
    async fn test_trace_by_run_missing_run_is_not_found() {
        let test_state = test_state();
        let err = get_trace_by_run(
            State(test_state.state),
            Path((test_state.context_id, "missing-run".to_string())),
            Query(UserIdentityQuery {
                user_id: "user-1".to_string(),
            }),
        )
        .await
        .expect_err("missing run must return not found");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_harness_views_enforce_ownership_and_report_conflict_when_absent() {
        let test_state = test_state();
        let id = test_state.context_id.clone();
        assert_harness_view_endpoint_guards(
            test_state.state.clone(),
            id.clone(),
            get_harness_evidence,
        )
        .await;
        assert_harness_view_endpoint_guards(
            test_state.state.clone(),
            id.clone(),
            get_harness_patches,
        )
        .await;
        assert_harness_view_endpoint_guards(
            test_state.state.clone(),
            id.clone(),
            get_harness_validation,
        )
        .await;
        assert_harness_view_endpoint_guards(
            test_state.state.clone(),
            id.clone(),
            get_harness_review,
        )
        .await;
        assert_harness_view_endpoint_guards(test_state.state.clone(), id.clone(), get_harness_risk)
            .await;
        assert_harness_view_endpoint_guards(test_state.state, id, get_harness_completion).await;
    }

    async fn assert_harness_view_endpoint_guards<F, Fut>(
        state: Arc<AppState>,
        id: String,
        endpoint: F,
    ) where
        F: Fn(State<Arc<AppState>>, Path<String>, Query<UserIdentityQuery>) -> Fut,
        Fut: std::future::Future<Output = Result<Json<serde_json::Value>, ApiError>>,
    {
        let forbidden = endpoint(
            State(state.clone()),
            Path(id.clone()),
            Query(UserIdentityQuery {
                user_id: "user-2".to_string(),
            }),
        )
        .await
        .expect_err("foreign user must fail");
        assert!(matches!(forbidden, ApiError::Forbidden(_)));

        let missing_view = endpoint(
            State(state),
            Path(id),
            Query(UserIdentityQuery {
                user_id: "user-1".to_string(),
            }),
        )
        .await
        .expect_err("owner with no harness data must get conflict");
        assert!(matches!(missing_view, ApiError::Conflict(_)));
    }

    #[tokio::test]
    async fn test_status_route_rejects_missing_or_wrong_user() {
        let test_state = test_state();

        let missing = update_work_context_status(
            State(test_state.state.clone()),
            Path(test_state.context_id.clone()),
            Query(UserIdentityQuery {
                user_id: "  ".to_string(),
            }),
            Json(UpdateStatusRequest {
                status: "in_progress".to_string(),
            }),
        )
        .await
        .expect_err("missing user must fail");
        assert!(matches!(missing, ApiError::BadRequest(_)));

        let wrong = update_work_context_status(
            State(test_state.state),
            Path(test_state.context_id),
            Query(UserIdentityQuery {
                user_id: "user-2".to_string(),
            }),
            Json(UpdateStatusRequest {
                status: "in_progress".to_string(),
            }),
        )
        .await
        .expect_err("foreign user must fail");
        assert!(matches!(wrong, ApiError::Forbidden(_)));
    }

    #[tokio::test]
    async fn test_continue_route_rejects_missing_or_wrong_user() {
        let test_state = test_state();

        let missing = continue_work_context(
            State(test_state.state.clone()),
            Path(test_state.context_id.clone()),
            Query(UserIdentityQuery {
                user_id: "".to_string(),
            }),
        )
        .await
        .expect_err("missing user must fail");
        assert!(matches!(missing, ApiError::BadRequest(_)));

        let wrong = continue_work_context(
            State(test_state.state),
            Path(test_state.context_id),
            Query(UserIdentityQuery {
                user_id: "user-2".to_string(),
            }),
        )
        .await
        .expect_err("foreign user must fail");
        assert!(matches!(wrong, ApiError::Forbidden(_)));
    }

    #[tokio::test]
    async fn test_run_routes_reject_missing_or_wrong_user() {
        let test_state = test_state();

        let missing_run = run_until_complete(
            State(test_state.state.clone()),
            Path(test_state.context_id.clone()),
            Query(UserIdentityQuery {
                user_id: "   ".to_string(),
            }),
            Json(RunContextRequest {
                max_iterations: None,
                max_runtime_ms: None,
                max_tool_calls: None,
                max_cost: None,
            }),
        )
        .await
        .expect_err("missing user must fail");
        assert!(matches!(missing_run, ApiError::BadRequest(_)));

        let wrong_run = run_until_complete(
            State(test_state.state.clone()),
            Path(test_state.context_id.clone()),
            Query(UserIdentityQuery {
                user_id: "user-2".to_string(),
            }),
            Json(RunContextRequest {
                max_iterations: None,
                max_runtime_ms: None,
                max_tool_calls: None,
                max_cost: None,
            }),
        )
        .await
        .expect_err("foreign user must fail");
        assert!(matches!(wrong_run, ApiError::Forbidden(_)));

        let missing_harness = run_harness(
            State(test_state.state.clone()),
            Path(test_state.context_id.clone()),
            Query(UserIdentityQuery {
                user_id: "".to_string(),
            }),
            Json(HarnessRunRequest {
                repo_root: std::path::PathBuf::from("."),
                mode: crate::harness::mode_policy::HarnessMode::Review,
                proposed_edits: vec![],
                edit_response: None,
            }),
        )
        .await
        .expect_err("missing user must fail");
        assert!(matches!(missing_harness, ApiError::BadRequest(_)));

        let wrong_harness = run_harness(
            State(test_state.state),
            Path(test_state.context_id),
            Query(UserIdentityQuery {
                user_id: "user-2".to_string(),
            }),
            Json(HarnessRunRequest {
                repo_root: std::path::PathBuf::from("."),
                mode: crate::harness::mode_policy::HarnessMode::Review,
                proposed_edits: vec![],
                edit_response: None,
            }),
        )
        .await
        .expect_err("foreign user must fail");
        assert!(matches!(wrong_harness, ApiError::Forbidden(_)));
    }

    #[tokio::test]
    async fn test_status_route_allows_owner() {
        let test_state = test_state();
        let result = update_work_context_status(
            State(test_state.state),
            Path(test_state.context_id),
            Query(UserIdentityQuery {
                user_id: "user-1".to_string(),
            }),
            Json(UpdateStatusRequest {
                status: "in_progress".to_string(),
            }),
        )
        .await
        .expect("owner should be allowed");
        assert_eq!(result.0.status, "InProgress");
    }

    #[tokio::test]
    async fn test_run_routes_pass_ownership_for_owner() {
        let test_state = test_state();
        let run_result = run_until_complete(
            State(test_state.state.clone()),
            Path(test_state.context_id.clone()),
            Query(UserIdentityQuery {
                user_id: "user-1".to_string(),
            }),
            Json(RunContextRequest {
                max_iterations: Some(1),
                max_runtime_ms: Some(50),
                max_tool_calls: Some(1),
                max_cost: Some(0.01),
            }),
        )
        .await;
        assert!(
            !matches!(
                run_result,
                Err(ApiError::BadRequest(_)) | Err(ApiError::Forbidden(_))
            ),
            "owner should pass ownership gate"
        );

        let harness_result = run_harness(
            State(test_state.state),
            Path(test_state.context_id),
            Query(UserIdentityQuery {
                user_id: "user-1".to_string(),
            }),
            Json(HarnessRunRequest {
                repo_root: std::path::PathBuf::from("."),
                mode: crate::harness::mode_policy::HarnessMode::Review,
                proposed_edits: vec![],
                edit_response: None,
            }),
        )
        .await;
        assert!(
            !matches!(
                harness_result,
                Err(ApiError::BadRequest(_)) | Err(ApiError::Forbidden(_))
            ),
            "owner should pass ownership gate"
        );
    }
}
