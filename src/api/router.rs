//! Route registration for the API server

use axum::body::Body;
use axum::http::Request;
use axum::{
    Router,
    extract::State,
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

use crate::api::AppState;
use crate::api::control_panel::create_control_panel_router;
use crate::api::conversations::{create_conversation, get_conversations};
use crate::api::flow_runs::run_flow;
use crate::api::health::{health_check, runtime_stack};
use crate::api::messages::{create_message, get_messages};
use crate::api::playbooks::{create_playbook, get_playbook, list_playbooks, update_playbook};
use crate::api::projects::{create_project, get_projects};
use crate::api::websocket::websocket_handler;
use crate::api::work_contexts::{
    continue_work_context, create_work_context, get_harness_completion, get_harness_evidence,
    get_harness_patches, get_harness_review, get_harness_risk, get_harness_validation,
    get_trace_by_run, get_work_context, get_work_context_artifacts, get_work_cost,
    get_work_quality, list_work_contexts, list_work_traces, run_harness, run_until_complete,
    submit_intent, update_work_context_status,
};

async fn count_requests_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    state.increment_request_count();
    next.run(req).await
}

/// Create the API router with all routes
pub fn create_router(state: Arc<AppState>) -> Router {
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers(tower_http::cors::Any);

    Router::new()
        .route("/health", get(health_check))
        .route("/runtime/stack", get(runtime_stack))
        .route("/projects", get(get_projects).post(create_project))
        .route("/projects/:id/conversations", get(get_conversations))
        .route("/conversations", post(create_conversation))
        .route("/conversations/:id/messages", get(get_messages))
        .route("/messages", post(create_message))
        .route("/playbooks", get(list_playbooks).post(create_playbook))
        .route("/playbooks/:id", get(get_playbook))
        .route("/playbooks/:id/update", post(update_playbook))
        .route("/conversations/:id/run", post(run_flow))
        .route("/ws/runs/:id", get(websocket_handler))
        .route(
            "/work-contexts",
            get(list_work_contexts).post(create_work_context),
        )
        .route("/work-contexts/submit-intent", post(submit_intent))
        .route("/work-contexts/:id", get(get_work_context))
        .route(
            "/work-contexts/:id/status",
            post(update_work_context_status),
        )
        .route(
            "/work-contexts/:id/artifacts",
            get(get_work_context_artifacts),
        )
        .route("/work-contexts/:id/continue", post(continue_work_context))
        .route(
            "/work-contexts/:id/run-until-complete",
            post(run_until_complete),
        )
        .route("/work-contexts/:id/harness/run", post(run_harness))
        .route(
            "/work-contexts/:id/harness/evidence",
            get(get_harness_evidence),
        )
        .route(
            "/work-contexts/:id/harness/patches",
            get(get_harness_patches),
        )
        .route(
            "/work-contexts/:id/harness/validation",
            get(get_harness_validation),
        )
        .route("/work-contexts/:id/harness/review", get(get_harness_review))
        .route("/work-contexts/:id/harness/risk", get(get_harness_risk))
        .route(
            "/work-contexts/:id/harness/completion",
            get(get_harness_completion),
        )
        .route("/work-contexts/:id/work-quality", get(get_work_quality))
        .route("/work-contexts/:id/work-cost", get(get_work_cost))
        .route("/work-contexts/:id/traces", get(list_work_traces))
        .route("/work-contexts/:id/traces/:run_id", get(get_trace_by_run))
        .nest("/control-panel", create_control_panel_router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            count_requests_middleware,
        ))
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
}
