//! External coding-agent compatibility subject (issue #154, Slice J) —
//! integration tests.
//!
//! Every effectful test drives the REAL governed path:
//! `ExecutionHarness (ExternalSubjectHarness) -> HarnessDispatch -> NodeRunner ->
//! governed capability (external_subject_capability)`, over the REAL
//! separately-spawned `external_subject_worker` process speaking JSON-RPC 2.0
//! (a published standard envelope). The fixture is a deterministic
//! compatibility subject — NOT a real third-party coding agent; these tests
//! prove the governed transport/protocol/governance path, not live
//! interoperability with Claude Code, Codex, Goose, OpenCode, Aider, ACP or
//! any vendor. No live external-CLI smoke was run.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use serde_json::{Value, json};

use prometheos_lite::workflow::execution_harness::{
    ExecutionHarness, HarnessDispatch, HarnessFamily, HarnessOutcome, HarnessRegistry, HarnessStep,
    canonical_harness_declarations,
};
use prometheos_lite::workflow::external_subject_harness::{
    EXTERNAL_SUBJECT_CAPS_ID, EXTERNAL_SUBJECT_COMPLETION_CAPABILITY, EXTERNAL_SUBJECT_HARNESS_ID,
    ExternalSubjectConfig, ExternalSubjectHarness, external_subject_capability,
    external_subject_declaration,
};
use prometheos_lite::workflow::node_contracts::NodeManifestV1;
use prometheos_lite::workflow::node_runner::{CapabilityRegistry, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::redaction::REDACTED_PLACEHOLDER;
use prometheos_lite::workflow::soma::capability::Locality;

mod common;

const NODE_ID: &str = "external-subject-step";

fn worker_exe() -> String {
    env!("CARGO_BIN_EXE_external_subject_worker").to_string()
}

fn manifest() -> NodeManifestV1 {
    NodeManifestV1::parse_json(
        &json!({
            "schemaVersion": "1.0.0",
            "nodeId": NODE_ID,
            "purpose": "governed external subject fixture",
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
    config: ExternalSubjectConfig,
    workspace_root: std::path::PathBuf,
    permit_caps: &[&str],
) -> NodeRunner {
    let mut registry = CapabilityRegistry::new();
    registry.declare(
        EXTERNAL_SUBJECT_COMPLETION_CAPABILITY,
        external_subject_capability(config, &workspace_root),
    );
    NodeRunner::new(registry, common::permit_for(&[NODE_ID], permit_caps))
}

fn harness() -> ExternalSubjectHarness {
    ExternalSubjectHarness::new()
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
        capability: EXTERNAL_SUBJECT_COMPLETION_CAPABILITY.to_string(),
        args,
    }
}

fn inference_args() -> Value {
    json!({
        "exe": worker_exe(),
        "prompt": "say hello",
    })
}

fn inference_args_with(argv: Vec<&str>) -> Value {
    json!({
        "exe": worker_exe(),
        "argv": argv,
        "prompt": "say hello",
    })
}

fn config_for_fixture() -> ExternalSubjectConfig {
    ExternalSubjectConfig::default()
        .allowed_executables(vec![worker_exe()])
        .expected_model("external-subject-model")
}

/// Extract the typed `(status code)` marker from a governed refusal error.
fn status_of(err: &anyhow::Error) -> String {
    let text = format!("{err:#}");
    let marker = "(status ";
    let start = text
        .find(marker)
        .map(|i| i + marker.len())
        .unwrap_or_else(|| panic!("error carries no typed status: {text}"));
    let end = text[start..]
        .find(')')
        .map(|i| start + i)
        .unwrap_or_else(|| panic!("typed status is not parenthesized: {text}"));
    text[start..end].to_string()
}

async fn run_governed(config: ExternalSubjectConfig, args: Value) -> Result<HarnessOutcome> {
    let root = tempfile::tempdir().expect("workspace tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch_with_secrets(&harness, &m, &r, &mut runner, vec![]);
    harness.run(step(args), &mut d).await
}

// ---------------------------------------------------------------------------
// Declaration / registry / catalog
// ---------------------------------------------------------------------------

#[tokio::test]
async fn external_subject_declaration_is_production_grade() {
    let declaration = external_subject_declaration();
    assert_eq!(declaration.harness_id, EXTERNAL_SUBJECT_HARNESS_ID);
    assert_eq!(declaration.family, HarnessFamily::ProtocolBridged);
    assert_eq!(
        declaration.claimed_capabilities,
        vec![EXTERNAL_SUBJECT_COMPLETION_CAPABILITY]
    );
    assert_eq!(declaration.capabilities.locality, Some(Locality::Local));
    assert_eq!(
        EXTERNAL_SUBJECT_CAPS_ID,
        "lite.harness.external_subject.caps"
    );

    let mut registry = HarnessRegistry::new();
    registry
        .register(Box::new(ExternalSubjectHarness::new()))
        .expect("external subject declaration registers fail-closed");
    let registered = registry
        .get(EXTERNAL_SUBJECT_HARNESS_ID)
        .expect("external subject is registered");
    assert_eq!(
        registered.declaration().harness_id,
        EXTERNAL_SUBJECT_HARNESS_ID
    );
    let catalog = registry.catalog();
    let entry = catalog
        .entries
        .iter()
        .find(|e| e.harness_id == EXTERNAL_SUBJECT_HARNESS_ID)
        .expect("external subject appears in the generated catalog");
    assert_eq!(entry.family, HarnessFamily::ProtocolBridged);
    assert_eq!(entry.locality, Some(Locality::Local));
    assert_eq!(entry.effect_classes, vec!["process-exec".to_string()]);

    let inventory = canonical_harness_declarations();
    assert!(
        inventory
            .iter()
            .any(|d| d.harness_id == EXTERNAL_SUBJECT_HARNESS_ID)
    );
}

// ---------------------------------------------------------------------------
// Positive: governed round trip over a real process boundary
// ---------------------------------------------------------------------------

#[tokio::test]
async fn external_subject_completion_round_trip() -> Result<()> {
    let config = config_for_fixture();
    let outcome = run_governed(config, inference_args()).await?;
    let output: Value = serde_json::from_str(&outcome.output)?;
    assert_eq!(output["success"], json!(true));
    assert_eq!(
        output["output"]["content"]["content"],
        json!("fixture completion")
    );
    assert_eq!(
        output["output"]["content"]["model"],
        json!("external-subject-model")
    );
    assert_eq!(
        output["output"]["content"]["usage"]["totalTokens"],
        json!(18)
    );
    assert!(output["output"]["latencyMs"].as_u64().is_some());
    assert_eq!(output["output"]["requestId"], json!("req-step-1"));
    assert_eq!(outcome.harness_id, EXTERNAL_SUBJECT_HARNESS_ID);
    Ok(())
}

#[tokio::test]
async fn external_subject_bearer_secret_never_leaves_the_channel() -> Result<()> {
    let canary = "fixture-canary-93f4a1b2";
    let config = config_for_fixture().known_secrets(vec![canary.to_string()]);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--echo-prompt"],
        "prompt": format!("the secret is {canary}; do not leak it"),
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch_with_secrets(&harness, &m, &r, &mut runner, vec![canary.to_string()]);
    let outcome = harness.run(step(args), &mut d).await?;
    assert!(
        !outcome.output.contains(canary),
        "known secret must not appear anywhere in the persisted outcome"
    );
    assert!(outcome.output.contains(REDACTED_PLACEHOLDER));
    Ok(())
}

// ---------------------------------------------------------------------------
// Handshake refusals: identity / version / capability checked BEFORE completion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn external_subject_wrong_server_identity_refuses() -> Result<()> {
    let config = config_for_fixture();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--bad-server-name"],
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("foreign subject identity must refuse the handshake");
    assert_eq!(status_of(&err), "handshake-refused");
    Ok(())
}

#[tokio::test]
async fn external_subject_unsupported_protocol_version_refuses() -> Result<()> {
    let config = config_for_fixture();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--bad-protocol-version"],
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("unsupported protocol version must refuse the handshake");
    assert_eq!(status_of(&err), "handshake-refused");
    Ok(())
}

#[tokio::test]
async fn external_subject_missing_completion_capability_refuses() -> Result<()> {
    // The subject advertises no completions capability: the adapter must
    // refuse BEFORE sending any completion request.
    let dir = tempfile::tempdir().expect("tempdir");
    let record = dir.path().join("protocol.log");
    let config = config_for_fixture();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--no-completions", "--record-file", record.to_str().expect("path")],
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("missing completions capability must refuse the handshake");
    assert_eq!(status_of(&err), "handshake-refused");
    let log = std::fs::read_to_string(&record).unwrap_or_default();
    assert!(
        log.contains("initialize"),
        "the handshake must be recorded; got: {log:?}"
    );
    assert!(
        !log.contains("completion/complete"),
        "no completion request may be sent after a refused handshake; got: {log:?}"
    );
    Ok(())
}

#[tokio::test]
async fn external_subject_forged_response_id_refuses() -> Result<()> {
    let config = config_for_fixture();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--wrong-id"],
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("a response with a mismatched id must be refused");
    assert_eq!(status_of(&err), "invalid-response");
    Ok(())
}

// ---------------------------------------------------------------------------
// Protocol failures mid-exchange
// ---------------------------------------------------------------------------

#[tokio::test]
async fn external_subject_jsonrpc_error_is_typed_refusal() -> Result<()> {
    let config = config_for_fixture();
    let outcome = run_governed(config, inference_args_with(vec!["--jsonrpc-error"])).await;
    let err = outcome.expect_err("subject error object must refuse the invocation");
    assert_eq!(status_of(&err), "external-subject-error");
    Ok(())
}

#[tokio::test]
async fn external_subject_malformed_response_is_refused() -> Result<()> {
    let config = config_for_fixture();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--malformed"],
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("a non-JSON response must be refused");
    assert_eq!(status_of(&err), "invalid-response");
    Ok(())
}

#[tokio::test]
async fn external_subject_oversized_response_is_refused() -> Result<()> {
    let config = config_for_fixture().max_output_bytes(4096);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--flood"],
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("an oversized response must be refused");
    assert!(["invalid-response", "timeout"].contains(&status_of(&err).as_str()));
    Ok(())
}

#[tokio::test]
async fn external_subject_slow_fixture_times_out() -> Result<()> {
    let config = config_for_fixture().timeout_ms(300);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--slow"],
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let started = std::time::Instant::now();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(args), &mut d)
            .await
            .expect_err("a held-open subject must hit the deadline")
    };
    let elapsed = started.elapsed();
    assert_eq!(status_of(&err), "timeout");
    assert!(
        elapsed < Duration::from_secs(5),
        "deadline must fire inside the lifecycle, not after it ({elapsed:?})"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_subject_cancellation_aborts_invocation() -> Result<()> {
    let cancel = Arc::new(AtomicBool::new(false));
    let config = config_for_fixture()
        .timeout_ms(30_000)
        .cancel(cancel.clone());
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--slow"],
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let r = restrictions();
    let handle = tokio::spawn(async move {
        let m2 = manifest();
        let hs = harness();
        let mut d = dispatch_with_secrets(&hs, &m2, &r, &mut runner, vec![]);
        hs.run(step(args), &mut d).await
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    cancel.store(true, Ordering::SeqCst);
    let err = handle.await.expect("task joins");
    let err = err.expect_err("cancelled invocation must be refused");
    assert_eq!(status_of(&err), "cancelled");
    Ok(())
}

// ---------------------------------------------------------------------------
// Governance: allowlist, capability, secrets, identifiers, checkpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn external_subject_executable_not_allowlisted_is_refused() {
    let config =
        config_for_fixture().allowed_executables(vec!["/bin/definitely-not-allowed".to_string()]);
    let root = tempfile::tempdir().expect("workspace tempdir");
    let config_endpoint = worker_exe();
    let args = json!({
        "exe": config_endpoint,
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("non-allowlisted executable must be refused");
    assert_eq!(status_of(&err), "allowlist-refused");
}

#[tokio::test]
async fn external_subject_unsupported_argument_is_refused() {
    let config = config_for_fixture();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "prompt": "say hello",
        "apiKey": "definitely-not-supported",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("unsupported arguments must be refused");
    assert_eq!(status_of(&err), "unsupported-argument");
}

#[tokio::test]
async fn external_subject_unauthorized_capability_is_refused() {
    let config = config_for_fixture();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &["harness.local_model.infer"],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let wrong = HarnessStep {
        step_id: "step-1".to_string(),
        capability: "harness.local_model.infer".to_string(),
        args: inference_args(),
    };
    let err = harness
        .run(wrong, &mut d)
        .await
        .expect_err("undeclared capability must refuse");
    assert!(!format!("{err:#}").is_empty());
}

#[tokio::test]
async fn external_subject_over_long_step_identifier_is_refused() {
    let config = config_for_fixture();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let oversized = HarnessStep {
        step_id: "s".repeat(513),
        capability: EXTERNAL_SUBJECT_COMPLETION_CAPABILITY.to_string(),
        args: inference_args(),
    };
    let err = harness
        .run(oversized, &mut d)
        .await
        .expect_err("over-long step identifier must be refused before dispatch");
    assert!(
        format!("{err:#}").contains("step identifier exceeds"),
        "unexpected error: {err:#}"
    );
}

#[tokio::test]
async fn external_subject_checkpoint_resume_round_trip() -> Result<()> {
    let harness = ExternalSubjectHarness::new();
    let outcome = HarnessOutcome {
        harness_id: EXTERNAL_SUBJECT_HARNESS_ID.to_string(),
        output: "{\"success\":true,\"status\":\"ok\"}".to_string(),
        evidence_entry_digest: "0".repeat(64),
        events: vec![],
        content_digest: None,
    }
    .sealed();
    let portable_state_digest = "a".repeat(64);
    let checkpoint = harness.checkpoint(&outcome, &portable_state_digest).await?;
    assert_eq!(checkpoint.harness_id, EXTERNAL_SUBJECT_HARNESS_ID);
    assert_eq!(checkpoint.harness_revision, "v1");
    let continuation = harness.resume(&checkpoint, &portable_state_digest).await?;
    assert_eq!(continuation.harness_id, EXTERNAL_SUBJECT_HARNESS_ID);
    let wrong = harness.resume(&checkpoint, &"b".repeat(64)).await;
    assert!(wrong.is_err());
    Ok(())
}

#[tokio::test]
async fn external_subject_expected_model_is_recorded_truthfully() -> Result<()> {
    let config = config_for_fixture();
    let outcome = run_governed(config, inference_args()).await?;
    let output: Value = serde_json::from_str(&outcome.output)?;
    // The model is what the subject attested, not what was requested.
    assert_eq!(
        output["output"]["content"]["model"],
        json!("external-subject-model")
    );
    Ok(())
}

#[tokio::test]
async fn external_subject_model_swap_is_refused() -> Result<()> {
    let config = config_for_fixture();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--model-swap"],
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("a different responding model must be refused");
    assert_eq!(status_of(&err), "model-mismatch");
    Ok(())
}

// ---------------------------------------------------------------------------
// Lifecycle repair: bounded/blocked channels, post-spawn cleanup, config
// ---------------------------------------------------------------------------

#[tokio::test]
async fn external_subject_flood_short_messages_are_bounded() {
    let config = config_for_fixture().timeout_ms(30_000);
    let started = std::time::Instant::now();
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--flood-short"],
        "prompt": "say hello",
    });
    let err = run_governed(config, args)
        .await
        .expect_err("a flooding subject must refuse");
    let elapsed = started.elapsed();
    assert_eq!(status_of(&err), "invalid-response");
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "a flooding subject must be bound by the lifecycle, not by the OS pipe ({elapsed:?})"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn external_subject_missing_model_reaps_hung_process() -> Result<()> {
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("pid");
    let config = config_for_fixture().timeout_ms(30_000);
    let started = std::time::Instant::now();
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--missing-model-hang", format!("--record-pid={}", pid_file.to_string_lossy())],
        "prompt": "say hello",
    });
    let err = run_governed(config, args)
        .await
        .expect_err("a completion missing model must be refused");
    let elapsed = started.elapsed();
    assert_eq!(status_of(&err), "invalid-response");
    assert!(elapsed < std::time::Duration::from_secs(5));
    // The subject deliberately stays alive after its broken response; the
    // governed teardown must reap it regardless.
    for _ in 0..100 {
        if pid_file.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let pid_text = std::fs::read_to_string(&pid_file)?;
    let pid: u32 = pid_text
        .trim()
        .strip_prefix("pid ")
        .and_then(|p| p.parse().ok())
        .expect("pid file carries the worker pid");
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "fixture process {pid} must not survive a refusal"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn external_subject_missing_content_reaps_hung_process() -> Result<()> {
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("pid");
    let config = config_for_fixture().timeout_ms(30_000);
    let started = std::time::Instant::now();
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--missing-content-hang", format!("--record-pid={}", pid_file.to_string_lossy())],
        "prompt": "say hello",
    });
    let err = run_governed(config, args)
        .await
        .expect_err("a completion missing content must be refused");
    let elapsed = started.elapsed();
    assert_eq!(status_of(&err), "invalid-response");
    assert!(elapsed < std::time::Duration::from_secs(5));
    for _ in 0..100 {
        if pid_file.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let pid_text = std::fs::read_to_string(&pid_file)?;
    let pid: u32 = pid_text
        .trim()
        .strip_prefix("pid ")
        .and_then(|p| p.parse().ok())
        .expect("pid file carries the worker pid");
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "fixture process {pid} must not survive a refusal"
    );
    Ok(())
}

#[tokio::test]
async fn external_subject_non_reading_child_with_large_prompt_times_out() -> Result<()> {
    let config = config_for_fixture().timeout_ms(2_000);
    let started = std::time::Instant::now();
    let args = json!({
        "exe": worker_exe(),
        "argv": ["--never-read"],
        "prompt": "a".repeat(30_000),
    });
    let err = run_governed(config, args)
        .await
        .expect_err("a silent child must hit the deadline");
    let elapsed = started.elapsed();
    assert_eq!(status_of(&err), "timeout");
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "the lifecycle must be bound by the deadline ({elapsed:?})"
    );
    Ok(())
}

#[tokio::test]
async fn external_subject_invalid_argv_shapes_refused() {
    for bad in [
        json!("not-an-array"),
        json!(["ok", 5]),
        json!(["ok", true]),
        json!(["ok", null]),
        json!({"0": "ok"}),
    ] {
        let config = config_for_fixture();
        let root = tempfile::tempdir().expect("workspace tempdir");
        let args = json!({
            "exe": worker_exe(),
            "argv": bad,
            "prompt": "say hello",
        });
        let mut runner = runner_for(
            config,
            root.path().to_path_buf(),
            &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
        );
        let harness = harness();
        let m = manifest();
        let r = restrictions();
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        let err = harness
            .run(step(args), &mut d)
            .await
            .expect_err("invalid argv shapes must refuse");
        assert_eq!(status_of(&err), "invalid-argv", "input {bad} must refuse");
    }
}

#[tokio::test]
async fn external_subject_too_many_known_secrets_is_refused() {
    // Secrets flow through the dispatch boundary, which always carries
    // strings; the handler enforces count and byte-length bounds there.
    let secrets: Vec<String> = (0..33).map(|i| format!("secret-{i}")).collect();
    let config = config_for_fixture();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch_with_secrets(&harness, &m, &r, &mut runner, secrets);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("over-count known secrets must refuse");
    assert_eq!(status_of(&err), "invalid-secrets");
}

#[tokio::test]
async fn external_subject_oversized_known_secret_is_refused() {
    let secrets = vec!["x".repeat(4097)];
    let config = config_for_fixture();
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch_with_secrets(&harness, &m, &r, &mut runner, secrets);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("overlong known secret must refuse");
    assert_eq!(status_of(&err), "invalid-secrets");
}

#[tokio::test]
async fn external_subject_operator_timeout_misconfiguration_refused() {
    let mut config = config_for_fixture();
    config.default_timeout_ms = 500;
    config.max_timeout_ms = 100; // default exceeds operator maximum
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("default timeout above the operator maximum must fail closed");
    assert_eq!(status_of(&err), "invalid-config");
}

#[tokio::test]
async fn external_subject_zero_operator_timeout_refused() {
    let mut config = config_for_fixture();
    config.default_timeout_ms = 0;
    let root = tempfile::tempdir().expect("workspace tempdir");
    let args = json!({
        "exe": worker_exe(),
        "prompt": "say hello",
    });
    let mut runner = runner_for(
        config,
        root.path().to_path_buf(),
        &[EXTERNAL_SUBJECT_COMPLETION_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut d = dispatch(&harness, &m, &r, &mut runner);
    let err = harness
        .run(step(args), &mut d)
        .await
        .expect_err("a zero operator timeout must fail closed");
    assert_eq!(status_of(&err), "invalid-config");
}
