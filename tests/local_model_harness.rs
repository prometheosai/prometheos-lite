//! Governed local-model harness (issue #154, Slice H) — integration tests.
//!
//! Every effectful test drives the REAL governed path:
//! `ExecutionHarness (LocalModelHarness) -> HarnessDispatch -> NodeRunner ->
//! governed capability (local_model_capability)`, over a REAL local HTTP
//! fixture server (tokio `TcpListener` exchanging actual HTTP requests and
//! responses). No in-memory mock replay is used as proof of transport.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use anyhow::Result;
use serde_json::{Value, json};
use std::thread::JoinHandle;

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
    assert!(parsed["output"]["latencyMs"].as_u64().unwrap() >= 0);
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
