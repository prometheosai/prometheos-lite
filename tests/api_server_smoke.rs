//! Minimal API server smoke test for the experimental `prometheos serve` surface.
//!
//! Verifies safe endpoints without model invocation, API keys, or external services.
//! The API server remains experimental — this is a smoke gate, not an alpha promise.

use axum::Router;
use axum::routing::get;
use tower::ServiceExt;

/// Helper: build a minimal AppState for testing the assembled API router.
/// Returns (AppState, TempDir) so the caller keeps the tempdir alive.
fn test_app_state() -> (
    std::sync::Arc<prometheos_lite::api::AppState>,
    tempfile::TempDir,
) {
    let db_dir = tempfile::tempdir().expect("temp db dir");
    let db_path = db_dir
        .path()
        .join("api_smoke_test.db")
        .to_str()
        .expect("db path")
        .to_string();

    let runtime = std::sync::Arc::new(prometheos_lite::flow::runtime::RuntimeContext::new());
    let embedding: std::sync::Arc<dyn prometheos_lite::flow::EmbeddingProvider> =
        std::sync::Arc::new(prometheos_lite::flow::memory::LocalEmbeddingProvider::new(
            "http://127.0.0.1:9/embeddings".to_string(),
            8,
            None,
        ));
    let memory_service = std::sync::Arc::new(prometheos_lite::flow::memory::MemoryService::new(
        prometheos_lite::flow::memory::MemoryDb::in_memory().expect("in-memory memory db"),
        Box::new(prometheos_lite::flow::memory::LocalEmbeddingProvider::new(
            "http://127.0.0.1:9/embeddings".to_string(),
            8,
            None,
        )),
    ));

    let state = std::sync::Arc::new(
        prometheos_lite::api::AppState::new(db_path, runtime, embedding, memory_service)
            .expect("app state"),
    );

    (state, db_dir)
}

#[tokio::test]
async fn api_server_router_constructs_and_serves_health() {
    let app = Router::new().route("/health", get(prometheos_lite::api::health::health_check));

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/health")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn api_server_router_constructs_and_serves_runtime_stack() {
    let app = Router::new().route(
        "/runtime/stack",
        get(prometheos_lite::api::health::runtime_stack),
    );

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/runtime/stack")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn assembled_api_router_serves_safe_endpoints() {
    let (state, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/health")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/runtime/stack")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn assembled_api_router_rejects_unknown_route() {
    let (state, _db_dir) = test_app_state();
    let app = prometheos_lite::api::router::create_router(state);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/definitely-not-a-real-route")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
}
