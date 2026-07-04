//! Minimal API server smoke test for the experimental `prometheos serve` surface.
//!
//! Verifies safe endpoints without model invocation, API keys, or external services.
//! The API server remains experimental — this is a smoke gate, not an alpha promise.

use axum::Router;
use axum::routing::get;
use tower::ServiceExt;

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
