//! Health check endpoint

use axum::Json;

/// Health check response
#[derive(serde::Serialize)]
pub struct HealthResponse {
    status: String,
}

/// Health check endpoint
///
/// Returns a simple JSON response indicating the server is running.
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}
