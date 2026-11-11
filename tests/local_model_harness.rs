//! Governed local-model harness (issue #154, Slice H) — integration tests.
//!
//! Every effectful test drives the REAL governed path:
//! `ExecutionHarness (LocalModelHarness) -> HarnessDispatch -> NodeRunner ->
//! governed capability (local_model_capability)`, over a REAL local HTTP
//! fixture server (tokio `TcpListener` exchanging actual HTTP requests and
//! responses). No in-memory mock replay is used as proof of transport.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Result;
use serde_json::{Value, json};

use prometheos_lite::workflow::execution_harness::{
    ExecutionHarness, HarnessDispatch, HarnessFamily, HarnessOutcome, HarnessRegistry, HarnessStep,
};
use prometheos_lite::workflow::local_model_harness::{
    LOCAL_MODEL_CAPS_ID, LOCAL_MODEL_HARNESS_ID, LOCAL_MODEL_INFER_CAPABILITY, LocalModelConfig,
    LocalModelHarness, local_model_capability, local_model_declaration,
};
use prometheos_lite::workflow::node_contracts::NodeManifestV1;
use prometheos_lite::workflow::node_runner::{CapabilityRegistry, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::redaction::REDACTED_PLACEHOLDER;
use prometheos_lite::workflow::soma::capability::Locality;

mod common;

const NODE_ID: &str = "local-model-step";
const MODEL: &str = "tiny-local-model";

fn manifest() -> NodeManifestV1 {
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": "1.0.0",
            "nodeId": NODE_ID,
            "purpose": "governed local model fixture",
            "inputs": [],
            "outputs": [{"name": "out", "typeRef": "string"}],
            "readableScopes": ["repo://x"],
            "writableScopes": ["work://y"],
            "retry": {"maxAttempts": 2, "retryableClasses": ["infra"]}
        })
        .to_string(),
    )
    .expect("manifest parses")
}

fn restrictions() -> LocalRestrictions {
    LocalRestrictions {
        readable_scopes: vec!["repo://x".into()],
        writable_scopes: vec!["work://y".into()],
        token_budget_ceiling: None,
        denied_providers: vec![],
        forbidden_paths: vec![],
        max_attempts: 3,
        escalation_target: "human-review".into(),
    }
}

fn runner_for(
    config: LocalModelConfig,
    workspace_root: std::path::PathBuf,
    permit_caps: &[&str],
) -> NodeRunner {
    let mut registry = CapabilityRegistry::new();
    registry.declare(
        LOCAL_MODEL_INFER_CAPABILITY,
        local_model_capability(config, workspace_root),
    );
    NodeRunner::new(registry, common::permit_for(&[NODE_ID], permit_caps))
}

fn harness() -> LocalModelHarness {
    LocalModelHarness::new()
}

fn dispatch<'a>(
    harness: &'a dyn ExecutionHarness,
    m: &'a NodeManifestV1,
    r: &'a LocalRestrictions,
    runner: &'a mut NodeRunner,
) -> HarnessDispatch<'a> {
    HarnessDispatch::new(harness, m, r, vec![], runner)
}

fn dispatch_with_secrets<'a>(
    harness: &'a dyn ExecutionHarness,
    m: &'a NodeManifestV1,
    r: &'a LocalRestrictions,
    runner: &'a mut NodeRunner,
    secrets: Vec<String>,
) -> HarnessDispatch<'a> {
    HarnessDispatch::new(harness, m, r, secrets, runner)
}

fn step(args: Value) -> HarnessStep {
    HarnessStep {
        step_id: "step-1".to_string(),
        capability: LOCAL_MODEL_INFER_CAPABILITY.to_string(),
        args,
    }
}

/// Local HTTP fixture server: binds an ephemeral loopback port and answers
/// one OpenAI-compatible chat-completions style JSON response per connection.
/// `None` body keeps the connection open without responding (timeout tests).
fn spawn_local_model_server(
    response_body: Option<String>,
    status_code: u16,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("local model fixture bind");
    let addr = listener.local_addr().expect("local model fixture addr");
    eprintln!("SERVER: Listening on {}", addr);
    let response_body_clone = response_body.clone();
    let handle = std::thread::spawn(move || {
        eprintln!("SERVER: Thread started, waiting for connections");
        for stream in listener.incoming() {
            eprintln!("SERVER: New connection incoming");
            let mut socket = match stream {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("SERVER: Accept error: {:?}", e);
                    continue;
                }
            };
            // Read request: headers until \r\n\r\n
            let mut head_buf = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                let n = socket.read(&mut tmp);
                match n {
                    Ok(0) => {
                        eprintln!("SERVER: Client closed connection");
                        break;
                    }
                    Err(e) => {
                        eprintln!("SERVER: Read error: {:?}", e);
                        break;
                    }
                    Ok(n) => {
                        head_buf.extend_from_slice(&tmp[..n]);
                        if head_buf.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                }
            }
            if head_buf.is_empty() {
                eprintln!("SERVER: No headers received");
                continue;
            }
            let head = String::from_utf8_lossy(&head_buf).into_owned();
            eprintln!("SERVER: Received request:\n{}", head);
            let content_length = head
                .lines()
                .find_map(|l| l.split_once(':'))
                .and_then(|(k, v)| {
                    k.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            eprintln!("SERVER: Content-Length: {}", content_length);
            // Read body
            if content_length > 0 {
                let mut body_buf = vec![0u8; content_length];
                let mut read = 0;
                while read < content_length {
                    match socket.read(&mut body_buf[read..]) {
                        Ok(0) => break,
                        Err(e) => {
                            eprintln!("SERVER: Body read error: {:?}", e);
                            break;
                        }
                        Ok(n) => read += n,
                    }
                }
                eprintln!("SERVER: Read body bytes: {}", read);
            }
            if let Some(ref response_body) = response_body_clone {
                let response = format!(
                    "HTTP/1.1 {status_code} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                eprintln!(
                    "SERVER: Sending response (body len: {}):\n{}",
                    response_body.len(),
                    response
                );
                if socket.write_all(response.as_bytes()).is_err() {
                    eprintln!("SERVER: write_all failed");
                    continue;
                }
                eprintln!("SERVER: write_all succeeded");
                let _ = socket.shutdown(std::net::Shutdown::Both);
                eprintln!("SERVER: shutdown done");
                break; // Only handle one request for this test
            }
        }
        eprintln!("SERVER: Thread exiting");
    });
    (format!("http://127.0.0.1:{}/v1", addr.port()), handle)
}

fn local_model_config(endpoint: String, model: &str) -> LocalModelConfig {
    LocalModelConfig::new(endpoint, model)
        .timeout_ms(30_000)
        .max_output_bytes(256 * 1024)
}

fn inference_args(endpoint: &str, model: &str) -> Value {
    json!({
        "model": model,
        "endpoint": endpoint,
        "messages": [{"role": "user", "content": "hi"}],
        "maxTokens": 64,
    })
}

// ---------------------------------------------------------------------------
// Positive: real local HTTP fixture round trip, structured output, registry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn local_model_http_fixture_round_trip_through_governed_dispatch() {
    let content = "hello from the governed local fixture";
    let (endpoint, server) = spawn_local_model_server(
        Some(
            serde_json::json!({
                "id": "chatcmpl-local-fixture",
                "object": "chat.completion",
                "model": MODEL,
                "choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 7, "completion_tokens": 4, "total_tokens": 11},
            })
            .to_string(),
        ),
        200,
    )
    ;
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let outcome = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect("governed local model inference over the local http fixture succeeds")
    };
    assert_eq!(outcome.harness_id, LOCAL_MODEL_HARNESS_ID);
    let parsed: Value = serde_json::from_str(&outcome.output).expect("governed result is JSON");
    assert_eq!(parsed["success"], json!(true));
    assert_eq!(parsed["status"], "ok");
    // The governed output structure has the content at output.content.content
    assert_eq!(parsed["output"]["content"]["content"], json!(content));
    assert_eq!(parsed["output"]["content"]["model"], json!(MODEL));
    assert_eq!(
        parsed["output"]["content"]["usage"]["totalTokens"],
        json!(11)
    );
    assert!(parsed["output"]["latencyMs"].is_u64());
    assert_eq!(parsed["output"]["requestId"], json!("req-step-1"));
    assert_eq!(parsed["output"]["correlationId"], json!("step-1"));
    server.join().expect("server thread joins");
}

#[tokio::test]
async fn local_model_structured_output_json_mode() {
    let structured_content = r#"{"answer":42}"#;
    let (endpoint, server) = spawn_local_model_server(
        Some(
            serde_json::json!({
                "id": "chatcmpl-local-fixture",
                "object": "chat.completion",
                "model": MODEL,
                "choices": [{"index": 0, "message": {"role": "assistant", "content": structured_content}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
            })
            .to_string(),
        ),
        200,
    )
    ;
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let args = json!({
        "model": MODEL,
        "endpoint": endpoint,
        "messages": [{"role": "user", "content": "hi"}],
        "maxTokens": 64,
        "responseFormat": {"type": "json_object"},
    });

    let outcome = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(args), &mut d)
            .await
            .expect("governed local model inference succeeds")
    };
    let parsed: Value = serde_json::from_str(&outcome.output).expect("governed result is JSON");
    assert_eq!(parsed["success"], json!(true));
    assert_eq!(parsed["status"], "ok");
    assert_eq!(
        parsed["output"]["content"]["content"],
        json!({"answer": 42})
    );
    server.join().expect("server thread joins");
}

#[test]
fn local_model_declaration_registers_and_catalogs_truthfully() {
    let declaration = local_model_declaration();
    assert!(declaration.validate().is_empty());
    assert_eq!(declaration.harness_id, LOCAL_MODEL_HARNESS_ID);
    assert_eq!(declaration.family, HarnessFamily::LocalModel);
    assert_eq!(LOCAL_MODEL_CAPS_ID, "lite.harness.local_model.caps");

    let mut registry = HarnessRegistry::new();
    registry
        .register(Box::new(LocalModelHarness::new()))
        .expect("local model harness declaration registers fail-closed");
    let registered = registry
        .get(LOCAL_MODEL_HARNESS_ID)
        .expect("local model harness is registered");
    assert_eq!(registered.declaration().harness_id, LOCAL_MODEL_HARNESS_ID);
    let catalog = registry.catalog();
    let entry = catalog
        .entries
        .iter()
        .find(|e| e.harness_id == LOCAL_MODEL_HARNESS_ID)
        .expect("local model harness appears in the generated catalog");
    assert_eq!(entry.family, HarnessFamily::LocalModel);
    assert_eq!(entry.effect_classes, vec!["network-io".to_string()]);
    assert_eq!(entry.locality, Some(Locality::Local));
}

// ---------------------------------------------------------------------------
// Positive: portable checkpoint round trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn local_model_checkpoint_resume_round_trip() -> Result<()> {
    let harness = LocalModelHarness::new();
    let outcome = HarnessOutcome {
        harness_id: LOCAL_MODEL_HARNESS_ID.to_string(),
        output: "{\"success\":true,\"status\":\"ok\"}".to_string(),
        evidence_entry_digest: "0".repeat(64),
        events: vec![],
        content_digest: None,
    }
    .sealed();
    let portable_state_digest = "a".repeat(64);
    let checkpoint = harness.checkpoint(&outcome, &portable_state_digest).await?;
    assert_eq!(checkpoint.harness_id, LOCAL_MODEL_HARNESS_ID);
    assert_eq!(checkpoint.harness_revision, "v1");
    let continuation = harness.resume(&checkpoint, &portable_state_digest).await?;
    assert_eq!(continuation.harness_id, LOCAL_MODEL_HARNESS_ID);
    assert_eq!(continuation.portable_state_digest, portable_state_digest);
    let wrong = harness.resume(&checkpoint, &"b".repeat(64)).await;
    assert!(wrong.is_err());
    Ok(())
}

// ---------------------------------------------------------------------------
// Negative: governed refusal paths (fail closed, before/without network effect)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn local_model_refuses_nonlocal_endpoint() {
    let config = local_model_config("http://192.168.0.10:8080/v1".to_string(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(inference_args("http://192.168.0.10:8080/v1", MODEL)),
                &mut d,
            )
            .await
            .expect_err("nonlocal local model endpoint must be refused")
    };
    assert!(format!("{err:#}").contains("did not succeed"));
}

#[tokio::test]
async fn local_model_refuses_unsupported_capability() {
    let config = local_model_config("http://127.0.0.1:8080/v1".to_string(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let step = HarnessStep {
        step_id: "step-1".to_string(),
        capability: "harness.local_model.unsupported".to_string(),
        args: inference_args("http://127.0.0.1:8080/v1", MODEL),
    };
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step, &mut d)
            .await
            .expect_err("undocumented capability must be refused")
    };
    eprintln!("Unsupported capability error: {err:#}");
    assert!(format!("{err:#}").contains("outside its declaration"));
}

#[tokio::test]
async fn local_model_refuses_unauthorized_capability() {
    let config = local_model_config("http://127.0.0.1:8080/v1".to_string(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(config, root.path().to_path_buf(), &[]);
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(inference_args("http://127.0.0.1:8080/v1", MODEL)),
                &mut d,
            )
            .await
            .expect_err(
                "unauthorized local model capability must be refused by the governed runner",
            )
    };
    assert!(format!("{err:#}").contains("SOMA-AUTH"));
}

#[tokio::test]
async fn local_model_refuses_invalid_model_identifier() {
    let config = local_model_config("http://127.0.0.1:8080/v1".to_string(), "bad model!");
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(inference_args("http://127.0.0.1:8080/v1", "bad model!")),
                &mut d,
            )
            .await
            .expect_err("invalid local model identifier must be refused")
    };
    assert!(format!("{err:#}").contains("did not succeed"));
}

#[tokio::test]
async fn local_model_refuses_silent_model_substitution() {
    let config = local_model_config("http://127.0.0.1:8080/v1".to_string(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(inference_args("http://127.0.0.1:8080/v1", "other-model")),
                &mut d,
            )
            .await
            .expect_err("silent local model substitution must be refused")
    };
    assert!(format!("{err:#}").contains("did not succeed"));
}

#[tokio::test]
async fn local_model_refuses_silent_endpoint_substitution() {
    let config = local_model_config("http://127.0.0.1:8080/v1".to_string(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(inference_args("http://127.0.0.1:9999/v1", MODEL)),
                &mut d,
            )
            .await
            .expect_err("silent local endpoint substitution must be refused")
    };
    assert!(format!("{err:#}").contains("did not succeed"));
}

#[tokio::test]
async fn local_model_refuses_connection_refused() {
    let config = local_model_config("http://127.0.0.1:1/v1".to_string(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args("http://127.0.0.1:1/v1", MODEL)), &mut d)
            .await
            .expect_err("connection refusal must be classified and refused")
    };
    assert!(format!("{err:#}").contains("did not succeed"));
}

#[tokio::test]
async fn local_model_refuses_unexpected_http_status() {
    let (endpoint, server) = spawn_local_model_server(
        Some(
            serde_json::json!({
                "id": "chatcmpl-local-fixture",
                "object": "chat.completion",
                "model": MODEL,
                "choices": [{"index": 0, "message": {"role": "assistant", "content": "x"}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
            })
            .to_string(),
        ),
        500,
    )
    ;
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect_err("unexpected non-2xx local model response must be refused")
    };
    assert!(format!("{err:#}").contains("did not succeed"));
    server.join().expect("server thread joins");
}

#[tokio::test]
async fn local_model_refuses_invalid_json_response() {
    let (endpoint, server) =
        spawn_local_model_server(Some("this is not a json response".to_string()), 200);
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect_err("invalid local model JSON response must be refused")
    };
    eprintln!("Invalid JSON response error: {err:#}");
    assert!(format!("{err:#}").contains("invalid-json"));
    server.join().expect("server thread joins");
}

#[tokio::test]
async fn local_model_refuses_oversized_response() {
    let (endpoint, server) = spawn_local_model_server(Some("x".repeat(1024)), 200);
    let config = local_model_config(endpoint.clone(), MODEL).max_output_bytes(64);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect_err("oversized local model response must be refused")
    };
    assert!(format!("{err:#}").contains("did not succeed"));
    server.join().expect("server thread joins");
}

#[tokio::test]
async fn local_model_timeout() {
    let (endpoint, server) = spawn_local_model_server(None, 200);
    let config = local_model_config(endpoint.clone(), MODEL).timeout_ms(300);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect_err("local model inference timeout must be classified and refused")
    };
    assert!(format!("{err:#}").contains("did not succeed"));
    // Server thread runs indefinitely waiting for connections; don't join.
    drop(server);
}

#[tokio::test]
async fn local_model_cancellation() {
    let config = local_model_config("http://127.0.0.1:8080/v1".to_string(), MODEL)
        .cancel(Arc::new(AtomicBool::new(true)));
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(inference_args("http://127.0.0.1:8080/v1", MODEL)),
                &mut d,
            )
            .await
            .expect_err("cancelled local model inference must be classified and refused")
    };
    assert!(format!("{err:#}").contains("did not succeed"));
}

#[tokio::test]
async fn local_model_secret_redaction_before_persistence() {
    let secret = "PROMETHEOS_TEST_SECRET_DO_NOT_PERSIST_9f83c0ffee";
    let content = format!("echo {secret}");
    let (endpoint, server) = spawn_local_model_server(
        Some(
            serde_json::json!({
                "id": "chatcmpl-local-fixture",
                "object": "chat.completion",
                "model": MODEL,
                "choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
            })
            .to_string(),
        ),
        200,
    )
    ;
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let outcome = {
        let mut d = dispatch_with_secrets(&harness, &m, &r, &mut runner, vec![secret.to_string()]);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect("governed local model inference succeeds")
    };
    let parsed: Value = serde_json::from_str(&outcome.output).expect("governed result is JSON");
    assert_eq!(parsed["success"], json!(true));
    assert_eq!(
        parsed["output"]["content"]["content"],
        json!(format!("echo {REDACTED_PLACEHOLDER}"))
    );
    server.join().expect("server thread joins");
}

// ---------------------------------------------------------------------------
// Adversarial, real-socket tests (bounded IO, strict endpoint, secret safety)
// ---------------------------------------------------------------------------

/// Raw-socket HTTP fixture: binds an ephemeral loopback port, reads the full
/// request head, and replies with the exact `raw_response` bytes.
fn spawn_local_model_server_raw(raw_response: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("raw fixture bind");
    let addr = listener.local_addr().expect("raw fixture addr");
    let handle = std::thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut head = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                match socket.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        head.extend_from_slice(&tmp[..n]);
                        if head.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let head_text = String::from_utf8_lossy(&head).into_owned();
            let content_length = head_text
                .lines()
                .find_map(|l| l.split_once(':'))
                .and_then(|(k, v)| {
                    k.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            let mut body_buf = vec![0u8; content_length];
            let mut read = 0;
            while read < content_length {
                match socket.read(&mut body_buf[read..]) {
                    Ok(0) => break,
                    Ok(n) => read += n,
                    Err(_) => break,
                }
            }
            let _ = socket.write_all(&raw_response);
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    });
    (format!("http://127.0.0.1:{}/v1", addr.port()), handle)
}

/// Raw-socket HTTP fixture: streams `bytes` without a header terminator and
/// never responds; used to exercise the header bound.
fn spawn_local_model_server_unbounded(bytes: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("unbounded fixture bind");
    let addr = listener.local_addr().expect("unbounded fixture addr");
    let handle = std::thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut head = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                match socket.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        head.extend_from_slice(&tmp[..n]);
                        if head.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = socket.write_all(&bytes);
            std::thread::sleep(Duration::from_secs(5));
        }
    });
    (format!("http://127.0.0.1:{}/v1", addr.port()), handle)
}

/// Slow-trickle fixture: sends the complete head first, then `body_prefix`
/// followed by one byte every `delay` until `total_bytes` are sent; used to
/// exercise the end-to-end deadline and blocked-read cancellation.
fn spawn_local_model_server_trickle(
    body_prefix: Vec<u8>,
    total_bytes: usize,
    delay: Duration,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("trickle fixture bind");
    let addr = listener.local_addr().expect("trickle fixture addr");
    let handle = std::thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut head = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                match socket.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        head.extend_from_slice(&tmp[..n]);
                        if head.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                total_bytes
            );
            if socket.write_all(response.as_bytes()).is_err() {
                return;
            }
            if socket.write_all(&body_prefix).is_err() {
                return;
            }
            let mut sent = body_prefix.len();
            while sent < total_bytes {
                if socket.write_all(b"x").is_err() {
                    return;
                }
                sent += 1;
                std::thread::sleep(delay);
            }
        }
    });
    (format!("http://127.0.0.1:{}/v1", addr.port()), handle)
}

/// Path-recording fixture: captures the request line for assertions.
fn spawn_local_model_server_recording() -> (
    String,
    Arc<std::sync::Mutex<String>>,
    std::thread::JoinHandle<()>,
) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("recording fixture bind");
    let addr = listener.local_addr().expect("recording fixture addr");
    let captured = Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();
    let body = serde_json::json!({
        "id": "chatcmpl-local-fixture",
        "object": "chat.completion",
        "model": MODEL,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
    })
    .to_string();
    let handle = std::thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                match socket.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let head_text = String::from_utf8_lossy(&buf).into_owned();
            let request_line = head_text.lines().next().unwrap_or("").to_string();
            *captured_clone.lock().unwrap() = request_line;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes());
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    });
    (
        format!("http://127.0.0.1:{}/v1", addr.port()),
        captured,
        handle,
    )
}

#[tokio::test]
async fn local_model_refuses_unlimited_header_stream() {
    let junk = vec![b'a'; 24 * 1024];
    let (endpoint, server) = spawn_local_model_server_unbounded(junk);
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect_err("unbounded streamed header must be refused")
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("oversized-header"), "unexpected error: {msg}");
    drop(server);
}

#[tokio::test]
async fn local_model_refuses_oversized_declared_content_length() {
    let mut raw_bytes =
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n".to_vec();
    raw_bytes.extend_from_slice(&vec![b'x'; 4096]);
    let (endpoint, server) = spawn_local_model_server_raw(raw_bytes);
    let config = local_model_config(endpoint.clone(), MODEL).max_output_bytes(64);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect_err("oversized declared content-length must be refused before reading the body")
    };
    let msg = format!("{err:#}");
    assert!(
        msg.contains("oversized-response"),
        "unexpected error: {msg}"
    );
    server.join().expect("server thread joins");
}

#[tokio::test]
async fn local_model_refuses_conflicting_content_length() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 10\r\nContent-Length: 20\r\nConnection: close\r\n\r\n01234567890123456789".to_vec();
    let (endpoint, server) = spawn_local_model_server_raw(raw);
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect_err("conflicting content-length must be refused")
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("invalid-response"), "unexpected error: {msg}");
    server.join().expect("server thread joins");
}

#[tokio::test]
async fn local_model_refuses_missing_content_length() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\nnope"
        .to_vec();
    let (endpoint, server) = spawn_local_model_server_raw(raw);
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect_err("missing content-length must be refused")
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("invalid-response"), "unexpected error: {msg}");
    server.join().expect("server thread joins");
}

#[tokio::test]
async fn local_model_slow_trickle_response_times_out() {
    let (endpoint, server) = spawn_local_model_server_trickle(
        b"{\"choices\":[{".to_vec(),
        512,
        Duration::from_millis(100),
    );
    let config = local_model_config(endpoint.clone(), MODEL).timeout_ms(300);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect_err("slow-trickle local model response must hit the deadline")
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("timeout"), "unexpected error: {msg}");
    drop(server);
}

#[tokio::test]
async fn local_model_cancellation_during_blocked_read() {
    // Server sends headers and a 4-byte prefix of a long body, then stalls.
    let body_prefix = b"{\"ch".to_vec();
    let (endpoint, server) =
        spawn_local_model_server_trickle(body_prefix, 4096, Duration::from_secs(3600));
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_trigger = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        cancel_trigger.store(true, Ordering::SeqCst);
    });
    let config = local_model_config(endpoint.clone(), MODEL)
        .timeout_ms(30_000)
        .cancel(cancel);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect_err("cancellation during a blocked read must interrupt the inference")
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("cancelled"), "unexpected error: {msg}");
    drop(server);
}

#[tokio::test]
async fn local_model_refuses_https_endpoint() {
    let config = local_model_config("https://127.0.0.1:8080/v1".to_string(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(inference_args("https://127.0.0.1:8080/v1", MODEL)),
                &mut d,
            )
            .await
            .expect_err("https endpoints must be refused until TLS is implemented")
    };
    let msg = format!("{err:#}");
    assert!(
        msg.contains("unsupported-scheme"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn local_model_refuses_credentials_and_query_and_fragment() {
    for endpoint in [
        "http://user:pass@127.0.0.1:8080/v1",
        "http://127.0.0.1:8080/v1?token=abc",
        "http://127.0.0.1:8080/v1#frag",
    ] {
        let config = local_model_config(endpoint.to_string(), MODEL);
        let root = tempfile::tempdir().expect("tempdir");
        let mut runner = runner_for(
            config,
            root.path().to_path_buf(),
            &[LOCAL_MODEL_INFER_CAPABILITY],
        );
        let harness = harness();
        let m = manifest();
        let r = restrictions();
        let err = {
            let mut d = dispatch(&harness, &m, &r, &mut runner);
            harness
                .run(step(inference_args(endpoint, MODEL)), &mut d)
                .await
                .expect_err("endpoint with credentials, query or fragment must be refused")
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("invalid-endpoint"),
            "endpoint {endpoint:?}: {msg}"
        );
    }
}

#[tokio::test]
async fn local_model_refuses_path_traversal() {
    let config = local_model_config("http://127.0.0.1:8080/../secret".to_string(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(inference_args("http://127.0.0.1:8080/../secret", MODEL)),
                &mut d,
            )
            .await
            .expect_err("path traversal must be refused")
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("invalid-endpoint"), "unexpected error: {msg}");
}

#[tokio::test]
async fn local_model_refuses_invalid_ports() {
    for endpoint in [
        "http://127.0.0.1:99999/v1",
        "http://127.0.0.1:abc/v1",
        "http://127.0.0.1:/v1",
        "http://127.0.0.1:0/v1",
    ] {
        let config = local_model_config(endpoint.to_string(), MODEL);
        let root = tempfile::tempdir().expect("tempdir");
        let mut runner = runner_for(
            config,
            root.path().to_path_buf(),
            &[LOCAL_MODEL_INFER_CAPABILITY],
        );
        let harness = harness();
        let m = manifest();
        let r = restrictions();
        let err = {
            let mut d = dispatch(&harness, &m, &r, &mut runner);
            harness
                .run(step(inference_args(endpoint, MODEL)), &mut d)
                .await
                .expect_err("invalid port must be refused")
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("invalid-endpoint"),
            "endpoint {endpoint:?}: {msg}"
        );
    }
}

#[tokio::test]
async fn local_model_preserves_v1_path_in_request() {
    let (endpoint, captured, server) = spawn_local_model_server_recording();
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect("governed local model inference succeeds");
    }
    let request_line = captured.lock().unwrap().clone();
    assert!(
        request_line.starts_with("POST /v1/chat/completions "),
        "recorded request line: {request_line:?}"
    );
    server.join().expect("server thread joins");
}

#[tokio::test]
async fn local_model_secret_in_malformed_response_not_reflected() {
    let secret = "PROMETHEOS_TEST_SECRET_DO_NOT_PERSIST_9f83c0ffee";
    let body = format!("this is not json {secret} trailing bytes");
    let raw = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes();
    let (endpoint, server) = spawn_local_model_server_raw(raw);
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch_with_secrets(&harness, &m, &r, &mut runner, vec![secret.to_string()]);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect_err("secret-bearing malformed response must be refused")
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("invalid-json"), "unexpected error: {msg}");
    assert!(!msg.contains(secret), "secret leaked in error: {msg}");
    server.join().expect("server thread joins");
}

#[tokio::test]
async fn local_model_secret_in_failed_structured_output_not_reflected() {
    let secret = "PROMETHEOS_TEST_SECRET_DO_NOT_PERSIST_9f83c0ffee";
    let content = format!("not valid json {secret}");
    let (endpoint, server) = spawn_local_model_server(
        Some(
            serde_json::json!({
                "id": "chatcmpl-local-fixture",
                "object": "chat.completion",
                "model": MODEL,
                "choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
            })
            .to_string(),
        ),
        200,
    );
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let args = json!({
        "model": MODEL,
        "endpoint": endpoint,
        "messages": [{"role": "user", "content": "hi"}],
        "maxTokens": 64,
        "responseFormat": {"type": "json_object"},
    });
    let err = {
        let mut d = dispatch_with_secrets(&harness, &m, &r, &mut runner, vec![secret.to_string()]);
        harness
            .run(step(args), &mut d)
            .await
            .expect_err("secret-bearing structured output must be refused")
    };
    let msg = format!("{err:#}");
    assert!(
        msg.contains("invalid-structured-output"),
        "unexpected error: {msg}"
    );
    assert!(!msg.contains(secret), "secret leaked in error: {msg}");
    server.join().expect("server thread joins");
}

// ---------------------------------------------------------------------------
// Review repair fixtures: ceiling enforcement, aggregate budget, latency
// ---------------------------------------------------------------------------

/// Counting fixture: tracks TCP accepts (must remain 0 for refusals that
/// happen before any network effect).
fn spawn_local_model_server_counting(
    response_body: Option<String>,
) -> (String, Arc<AtomicUsize>, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("counting fixture bind");
    let addr = listener.local_addr().expect("counting fixture addr");
    let accepts = Arc::new(AtomicUsize::new(0));
    let accepts_clone = accepts.clone();
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let _ = accepts_clone.fetch_add(1, Ordering::SeqCst);
            let mut socket = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut head = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                match socket.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        head.extend_from_slice(&tmp[..n]);
                        if head.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            if head.is_empty() {
                continue;
            }
            let head_text = String::from_utf8_lossy(&head).into_owned();
            let content_length = head_text
                .lines()
                .find_map(|l| l.split_once(':'))
                .and_then(|(k, v)| {
                    k.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            let mut body_buf = vec![0u8; content_length];
            let mut read = 0;
            while read < content_length {
                match socket.read(&mut body_buf[read..]) {
                    Ok(0) => break,
                    Ok(n) => read += n,
                    Err(_) => break,
                }
            }
            if let Some(response_body) = &response_body {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                let _ = socket.write_all(response.as_bytes());
                let _ = socket.shutdown(std::net::Shutdown::Both);
            }
        }
    });
    (
        format!("http://127.0.0.1:{}/v1", addr.port()),
        accepts,
        handle,
    )
}

/// Delayed fixture: accepts, reads the request, sleeps `delay`, then sends a
/// valid OpenAI-compatible response.
fn spawn_local_model_server_delayed(
    response_body: String,
    delay: Duration,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("delayed fixture bind");
    let addr = listener.local_addr().expect("delayed fixture addr");
    let handle = std::thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut head = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                match socket.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        head.extend_from_slice(&tmp[..n]);
                        if head.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let head_text = String::from_utf8_lossy(&head).into_owned();
            let content_length = head_text
                .lines()
                .find_map(|l| l.split_once(':'))
                .and_then(|(k, v)| {
                    k.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            let mut body_buf = vec![0u8; content_length];
            let mut read = 0;
            while read < content_length {
                match socket.read(&mut body_buf[read..]) {
                    Ok(0) => break,
                    Ok(n) => read += n,
                    Err(_) => break,
                }
            }
            std::thread::sleep(delay);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            let _ = socket.write_all(response.as_bytes());
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    });
    (format!("http://127.0.0.1:{}/v1", addr.port()), handle)
}

#[tokio::test]
async fn local_model_refuses_timeout_above_config_ceiling() {
    let (endpoint, accepts, server) = spawn_local_model_server_counting(None);
    let config = local_model_config(endpoint.clone(), MODEL).timeout_ms(300);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut args = inference_args(&endpoint, MODEL);
    args["timeoutMs"] = json!(3_600_000u64);
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness.run(step(args), &mut d).await.expect_err(
            "task timeout above the configured ceiling must be refused before any network effect",
        )
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("invalid-timeout"), "unexpected error: {msg}");
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        0,
        "no connection may be accepted when the request is refused before transport"
    );
    drop(server);
}

#[tokio::test]
async fn local_model_refuses_zero_and_malformed_timeout() {
    for bad in [json!(0), json!("fast"), json!(1.5), json!(true), json!(-1)] {
        let bad_display = bad.to_string();
        let config =
            local_model_config("http://127.0.0.1:8080/v1".to_string(), MODEL).timeout_ms(30_000);
        let root = tempfile::tempdir().expect("tempdir");
        let mut runner = runner_for(
            config,
            root.path().to_path_buf(),
            &[LOCAL_MODEL_INFER_CAPABILITY],
        );
        let harness = harness();
        let m = manifest();
        let r = restrictions();
        let mut args = inference_args("http://127.0.0.1:8080/v1", MODEL);
        args["timeoutMs"] = bad;
        let err = {
            let mut d = dispatch(&harness, &m, &r, &mut runner);
            harness
                .run(step(args), &mut d)
                .await
                .expect_err("invalid timeoutMs must be refused")
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("invalid-timeout"),
            "timeoutMs={bad_display}: unexpected error: {msg}"
        );
    }
}

#[tokio::test]
async fn local_model_shorter_timeout_is_permitted() {
    // Stalled fixture: sends headers + prefix, then stalls forever.
    let (endpoint, server) =
        spawn_local_model_server_trickle(b"{\"ch".to_vec(), 4096, Duration::from_secs(3600));
    let config = local_model_config(endpoint.clone(), MODEL).timeout_ms(10_000);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut args = inference_args(&endpoint, MODEL);
    args["timeoutMs"] = json!(250u64);
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(args), &mut d)
            .await
            .expect_err("shorter requested timeout must bound the stalled inference")
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("timeout"), "unexpected error: {msg}");
    drop(server);
}

#[tokio::test]
async fn local_model_refuses_aggregate_message_budget() {
    let (endpoint, accepts, server) = spawn_local_model_server_counting(None);
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let messages: Vec<Value> = (0..33)
        .map(|i| json!({"role": "user", "content": format!("m{i}")}))
        .collect();
    let args = json!({
        "model": MODEL,
        "endpoint": endpoint,
        "messages": messages,
        "maxTokens": 64,
    });
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(args), &mut d)
            .await
            .expect_err("aggregate message budget must be refused before transport")
    };
    let msg = format!("{err:#}");
    assert!(
        msg.contains("request-budget-exceeded"),
        "unexpected error: {msg}"
    );
    assert_eq!(accepts.load(Ordering::SeqCst), 0);
    drop(server);
}

#[tokio::test]
async fn local_model_refuses_aggregate_request_bytes() {
    let (endpoint, accepts, server) = spawn_local_model_server_counting(None);
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let big = "x".repeat(8000);
    let messages: Vec<Value> = (0..5)
        .map(|_| json!({"role": "user", "content": big}))
        .collect();
    let args = json!({
        "model": MODEL,
        "endpoint": endpoint,
        "messages": messages,
        "maxTokens": 64,
    });
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(args), &mut d)
            .await
            .expect_err("aggregate request bytes must be refused before transport")
    };
    let msg = format!("{err:#}");
    assert!(
        msg.contains("request-budget-exceeded"),
        "unexpected error: {msg}"
    );
    assert_eq!(accepts.load(Ordering::SeqCst), 0);
    drop(server);
}

#[tokio::test]
async fn local_model_latency_reflects_actual_inference() {
    let response_body = serde_json::json!({
        "id": "chatcmpl-local-fixture",
        "object": "chat.completion",
        "model": MODEL,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "delayed answer"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
    })
    .to_string();
    let (endpoint, server) =
        spawn_local_model_server_delayed(response_body, Duration::from_millis(200));
    let config = local_model_config(endpoint.clone(), MODEL);
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[LOCAL_MODEL_INFER_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let outcome = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(inference_args(&endpoint, MODEL)), &mut d)
            .await
            .expect("governed local model inference succeeds")
    };
    let parsed: Value = serde_json::from_str(&outcome.output).expect("governed result is JSON");
    assert_eq!(parsed["success"], json!(true));
    let latency = parsed["output"]["latencyMs"]
        .as_u64()
        .expect("latencyMs is present and numeric");
    assert!(
        latency >= 100,
        "latencyMs {latency} is implausibly low for a 200ms delayed fixture"
    );
    server.join().expect("server thread joins");
}
