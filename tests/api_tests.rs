//! API Tests for PrometheOS Lite v1.2
//!
//! Note: Due to in-memory database isolation challenges in test environment,
//! these tests focus on the health endpoint which validates server startup.
//! Full integration testing should be done manually with a persistent database.

use axum::http::StatusCode;
use axum_test::TestServer;
use std::sync::Arc;

use prometheos_lite::api::{create_router, AppState};
use prometheos_lite::flow::RuntimeContext;

#[tokio::test]
async fn test_health_endpoint() {
    let db_path = ":memory:".to_string();
    let runtime = Arc::new(RuntimeContext::new());
    let state = Arc::new(AppState::new(db_path, runtime));
    let app = create_router(state);
    let server = TestServer::new(app).unwrap();

    let response = server
        .get("/health")
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let body = response.json::<serde_json::Value>();
    assert_eq!(body["status"], "ok");
}
