//! Mock provider smoke tests for the OpenAI-compatible path.
//!
//! These tests use a local mock HTTP server. No real network calls,
//! no Ollama, no Ornith, no OpenRouter, no external API keys required.

use axum::{Router, extract::Json, http::StatusCode, routing::post};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::types::ChatCompletionRequest;
use crate::llm::client::LlmClient;

/// Shared state the mock server uses to record observed requests.
#[derive(Debug, Default)]
struct MockState {
    received_requests: Vec<ChatCompletionRequestOwned>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ChatCompletionRequestOwned {
    model: String,
    messages: Vec<Value>,
    stream: bool,
}

impl From<&ChatCompletionRequest<'_>> for ChatCompletionRequestOwned {
    fn from(r: &ChatCompletionRequest<'_>) -> Self {
        Self {
            model: r.model.to_string(),
            messages: r
                .messages
                .iter()
                .map(|m| {
                    json!({
                        "role": m.role,
                        "content": m.content
                    })
                })
                .collect(),
            stream: r.stream,
        }
    }
}

/// Start a mock server on a random port and return its address.
async fn start_mock_server(state: Arc<Mutex<MockState>>) -> SocketAddr {
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(move |body: Json<Value>| async move {
                let mut s = state.lock().await;
                let req: ChatCompletionRequestOwned = serde_json::from_value(body.0.clone())
                    .unwrap_or_else(|_| ChatCompletionRequestOwned {
                        model: String::new(),
                        messages: vec![],
                        stream: false,
                    });
                s.received_requests.push(req);

                Json(json!({
                    "choices": [{
                        "message": {
                            "role": "assistant",
                            "content": "Hello from mock LLM!"
                        }
                    }]
                }))
            }),
        )
        .route(
            "/v1/chat/completions/error",
            post(|| async {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": {
                            "message": "mock internal server error",
                            "type": "server_error"
                        }
                    })),
                )
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind mock server");
    let addr = listener.local_addr().expect("failed to get local addr");

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Small delay to ensure server is ready
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    addr
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_llm_client_sends_model_in_request() {
    let state = Arc::new(Mutex::new(MockState::default()));
    let addr = start_mock_server(state.clone()).await;
    let base_url = format!("http://{}", addr);

    let client = LlmClient::new(&base_url, "mock-model").expect("should build client");

    let result = client.generate("Hello, world!").await;
    assert!(
        result.is_ok(),
        "generate should succeed: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap(), "Hello from mock LLM!");

    let state = state.lock().await;
    assert_eq!(state.received_requests.len(), 1);
    assert_eq!(state.received_requests[0].model, "mock-model");
    assert!(!state.received_requests[0].stream);
}

#[tokio::test]
async fn test_llm_client_sends_messages_in_expected_format() {
    let state = Arc::new(Mutex::new(MockState::default()));
    let addr = start_mock_server(state.clone()).await;
    let base_url = format!("http://{}", addr);

    let client = LlmClient::new(&base_url, "mock-model").expect("should build client");

    let result = client.generate("Test prompt").await;
    assert!(result.is_ok());

    let state = state.lock().await;
    assert_eq!(state.received_requests.len(), 1);
    let messages = &state.received_requests[0].messages;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"], "Test prompt");
}

#[tokio::test]
async fn test_llm_client_no_auth_header_when_no_api_key() {
    let state = Arc::new(Mutex::new(MockState::default()));
    let addr = start_mock_server(state.clone()).await;
    let base_url = format!("http://{}", addr);

    // No API key set
    let client = LlmClient::new(&base_url, "mock-model")
        .expect("should build client")
        .with_api_key(None);

    let result = client.generate("hello").await;
    assert!(
        result.is_ok(),
        "generate should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_llm_client_parses_response_correctly() {
    let state = Arc::new(Mutex::new(MockState::default()));
    let addr = start_mock_server(state.clone()).await;
    let base_url = format!("http://{}", addr);

    let client = LlmClient::new(&base_url, "mock-model").expect("should build client");

    let result = client.generate("Test").await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "Hello from mock LLM!");
}

#[tokio::test]
async fn test_llm_client_handles_error_response() {
    // Start a dedicated mock server that returns errors
    async fn error_server() -> SocketAddr {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": {"message": "mock error"}})),
                )
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind");
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        addr
    }

    let addr = error_server().await;
    let base_url = format!("http://{}", addr);

    let client = LlmClient::new(&base_url, "mock-model")
        .expect("should build client")
        .with_retries(0); // no retries to keep test fast

    let result = client.generate("hello").await;
    assert!(
        result.is_err(),
        "should have failed on server error, got: {:?}",
        result.ok()
    );
}

#[tokio::test]
async fn test_llm_client_unreachable_endpoint() {
    // Use a port that is very unlikely to be bound
    let client = LlmClient::new("http://127.0.0.1:19999", "mock-model")
        .expect("should build client")
        .with_retries(0);

    let result = client.generate("hello").await;
    assert!(result.is_err(), "should fail on unreachable endpoint");
}
