//! Governed protocol-bridged harness (issue #154, slice G) — integration
//! tests over a real worker process.
//!
//! Every test drives the REAL bridge: an [`ExecutionHarness`] adapter whose
//! `run` invokes the declared `harness.protocol.bridge` capability through
//! [`HarnessDispatch::call`] (the only sanctioned effect path), a
//! [`ProtocolBridgeHarness`] registration in the [`HarnessRegistry`], and
//! the capability handler that spawns the deterministic
//! `protocol_test_worker` binary (a real separate process) inside a
//! governed workspace and exchanges one `lite.protocol.bridge` v1 JSON
//! envelope over stdin/stdout. No in-memory replay, no ScriptedProtocolHarness
//! fixture is used as a production claim.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use prometheos_lite::workflow::execution_harness::{
    ExecutionHarness, HarnessDispatch, HarnessOutcome, HarnessRegistry, HarnessStep,
    canonical_harness_declarations,
};
use prometheos_lite::workflow::native_harness::{NATIVE_HARNESS_ID, NativeHarness};
use prometheos_lite::workflow::node_contracts::NodeManifestV1;
use prometheos_lite::workflow::node_runner::{CapabilityRegistry, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::protocol_harness::{
    PROTOCOL_BRIDGE_CAPABILITY, PROTOCOL_HARNESS_ID, PROTOCOL_REVISION, ProtocolBridgeConfig,
    ProtocolBridgeHarness, protocol_bridge_capability,
};
use prometheos_lite::workflow::subprocess_harness::{SUBPROCESS_HARNESS_ID, SubprocessHarness};

mod common;

const NODE_ID: &str = "protocol-bridge-step";

fn worker_exe() -> String {
    env!("CARGO_BIN_EXE_protocol_test_worker").to_string()
}

fn manifest() -> NodeManifestV1 {
    NodeManifestV1::parse_json(
        &serde_json::json!({
            "schemaVersion": "1.0.0",
            "nodeId": NODE_ID,
            "purpose": "governed protocol bridge fixture",
            "inputs": [],
            "outputs": [{"name": "out", "typeRef": "string"}],
            "readableScopes": ["repo://x"],
            "writableScopes": ["work://y"],
            "retry": {"maxAttempts": 2, "retryableClasses": ["infra"]}
        })
        .to_string(),
    )
    .unwrap()
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

/// A governed runner with the bridge capability declared under the execute
/// cap, bound to `workspace_root`. `permit_caps` controls the governance
/// grant; the registry always declares the real capability.
fn runner_for(
    config: ProtocolBridgeConfig,
    workspace_root: std::path::PathBuf,
    permit_caps: &[&str],
) -> NodeRunner {
    let mut registry = CapabilityRegistry::new();
    registry.declare(
        PROTOCOL_BRIDGE_CAPABILITY,
        protocol_bridge_capability(config, workspace_root),
    );
    NodeRunner::new(registry, common::permit_for(&[NODE_ID], permit_caps))
}

fn harness() -> ProtocolBridgeHarness {
    ProtocolBridgeHarness::new()
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

fn step(input: serde_json::Value) -> HarnessStep {
    HarnessStep {
        step_id: "step-1".to_string(),
        capability: PROTOCOL_BRIDGE_CAPABILITY.to_string(),
        args: serde_json::json!({"exe": worker_exe(), "argv": [], "input": input}),
    }
}

/// Step with explicit worker argv (P1 regression hooks `--never-read` /
/// `--exit-now` select worker behavior that never touches stdin).
fn step_with_argv(argv: Vec<String>, input: serde_json::Value) -> HarnessStep {
    HarnessStep {
        step_id: "step-1".to_string(),
        capability: PROTOCOL_BRIDGE_CAPABILITY.to_string(),
        args: serde_json::json!({"exe": worker_exe(), "argv": argv, "input": input}),
    }
}

/// Large-but-valid input: envelope larger than the OS pipe capacity on both
/// Windows (4 KiB) and Unix (64 KiB), below raised message budgets.
fn large_input() -> serde_json::Value {
    serde_json::json!({"mode": "echo", "payload": "X".repeat(150_000)})
}

fn large_config() -> ProtocolBridgeConfig {
    let mut cfg = ProtocolBridgeConfig::default()
        .allowed_executables(vec![worker_exe()])
        .max_request_bytes(256 * 1024)
        .max_response_bytes(256 * 1024)
        .max_output_bytes(256 * 1024);
    cfg.max_args_bytes = 256 * 1024;
    cfg
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn config() -> ProtocolBridgeConfig {
    ProtocolBridgeConfig::default().allowed_executables(vec![worker_exe()])
}

// ---------------------------------------------------------------------------
// Positive: real handshake, round-trip, correlation, evidence, cleanup
// ---------------------------------------------------------------------------

/// 1 — Real worker handshake succeeds and the structured request reaches
/// the worker (echo round-trip through a separate process).
#[tokio::test]
async fn protocol_handshake_and_echo_round_trip() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let outcome = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(serde_json::json!({"mode": "echo", "payload": "hello-bridge"})),
                &mut d,
            )
            .await
            .expect("handshake run succeeds")
    };

    assert_eq!(outcome.harness_id, PROTOCOL_HARNESS_ID);
    let parsed: serde_json::Value =
        serde_json::from_str(&outcome.output).expect("result JSON parses");
    assert_eq!(parsed["success"], true);
    assert_eq!(parsed["status"], "ok");
    // The worker echoed the full input: the request genuinely crossed the
    // process boundary and the response was parsed and verified.
    assert_eq!(
        parsed["output"]["echoed"]["payload"], "hello-bridge",
        "output: {}",
        outcome.output
    );
    assert_eq!(parsed["output"]["echoed"]["mode"], "echo");
    // Correlation identity is preserved end to end.
    assert_eq!(outcome.events.len(), 2);
    assert_eq!(outcome.events[0].correlation_id, "step-1");
    assert_eq!(outcome.events[1].correlation_id, "step-1");
    outcome.verify().expect("sealed outcome verifies");
    assert!(is_hex64(&outcome.evidence_entry_digest));
    // Governed dispatch produced exactly one attributable journal entry.
    assert_eq!(runner.journal().len(), 1);
    assert_eq!(runner.journal()[0].identity_key, "step-1");
}

/// 2 — Nested structured input round-trips losslessly.
#[tokio::test]
async fn nested_structured_input_round_trips() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();

    let outcome = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(serde_json::json!({"mode": "echo", "nested": {"a": [1, 2, {"b": true}], "s": "unicode-☃"}})),
                &mut d,
            )
            .await
            .expect("nested run succeeds")
    };
    let parsed: serde_json::Value =
        serde_json::from_str(&outcome.output).expect("result JSON parses");
    assert_eq!(
        parsed["output"]["echoed"]["nested"]["a"][2]["b"], true,
        "output: {}",
        outcome.output
    );
    assert_eq!(parsed["output"]["echoed"]["nested"]["s"], "unicode-☃");
    assert_eq!(runner.journal().len(), 1);
}

/// 3 — Portable checkpoint round-trip succeeds (no live-process resume).
#[tokio::test]
async fn portable_checkpoint_round_trip() {
    let root = tempfile::tempdir().expect("tempdir");
    let outcome = {
        let mut runner = runner_for(
            config(),
            root.path().to_path_buf(),
            &[PROTOCOL_BRIDGE_CAPABILITY],
        );
        let harness = harness();
        let m = manifest();
        let r = restrictions();
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "echo"})), &mut d)
            .await
            .expect("run succeeds")
    };
    let harness = harness();
    let digest = outcome.evidence_entry_digest.clone();
    let checkpoint = futures::executor::block_on(harness.checkpoint(&outcome, &digest))
        .expect("checkpoint exports");
    checkpoint.verify().expect("checkpoint verifies");
    let continuation =
        futures::executor::block_on(harness.resume(&checkpoint, &digest)).expect("resume succeeds");
    assert_eq!(continuation.harness_id, PROTOCOL_HARNESS_ID);
    assert_eq!(continuation.harness_revision, PROTOCOL_REVISION);
    assert_eq!(continuation.portable_state_digest, digest);
}

// ---------------------------------------------------------------------------
// Registry / catalog: three real production adapters, drift detectable
// ---------------------------------------------------------------------------

/// 4 — The registry holds three distinct real production adapters.
#[test]
fn registry_contains_three_real_production_adapters() {
    let native = NativeHarness::new();
    let subprocess = SubprocessHarness::new();
    let bridge = ProtocolBridgeHarness::new();
    let declarations = vec![
        native.declaration().clone(),
        subprocess.declaration().clone(),
        bridge.declaration().clone(),
    ];
    let mut registry = HarnessRegistry::new();
    registry
        .register(Box::new(native))
        .expect("native registers");
    registry
        .register(Box::new(subprocess))
        .expect("subprocess registers");
    registry
        .register(Box::new(bridge))
        .expect("protocol bridge registers");
    let catalog = registry.catalog();
    assert_eq!(catalog.entries.len(), 3);
    let mut ids: Vec<&str> = catalog
        .entries
        .iter()
        .map(|e| e.harness_id.as_str())
        .collect();
    ids.sort_unstable();
    let mut want = vec![
        NATIVE_HARNESS_ID,
        SUBPROCESS_HARNESS_ID,
        PROTOCOL_HARNESS_ID,
    ];
    want.sort_unstable();
    assert_eq!(ids, want);
    let families: std::collections::BTreeSet<_> =
        catalog.entries.iter().map(|e| e.family.as_str()).collect();
    assert!(families.contains("native"));
    assert!(families.contains("subprocess"));
    assert!(families.contains("protocol-bridged"));
    let bridge_entry = catalog
        .entries
        .iter()
        .find(|e| e.harness_id == PROTOCOL_HARNESS_ID)
        .expect("bridge entry present");
    assert_eq!(
        bridge_entry.declared_capabilities,
        vec![PROTOCOL_BRIDGE_CAPABILITY.to_string()]
    );
    assert!(
        catalog
            .verify_against_declarations(&declarations)
            .is_empty()
    );
}

/// 5 — The canonical inventory names the bridge and catalog drift is
/// detectable from a declaration change.
#[test]
fn catalog_contains_bridge_declaration_and_drift_is_detectable() {
    let decls = canonical_harness_declarations();
    assert!(
        decls.iter().any(|d| d.harness_id == PROTOCOL_HARNESS_ID),
        "canonical inventory must include the protocol bridge harness"
    );
    // A drifted copy of the bridge declaration must move the catalog digest.
    let mut drifted = decls.clone();
    for d in drifted.iter_mut() {
        if d.harness_id == PROTOCOL_HARNESS_ID {
            d.declared_capabilities = vec!["harness.protocol.other".to_string()];
            d.claimed_capabilities = vec!["harness.protocol.other".to_string()];
        }
        d.content_digest = None;
        *d = d.clone().sealed();
    }
    let base = {
        struct StaticHarness {
            declaration: prometheos_lite::workflow::execution_harness::HarnessDeclarationV1,
        }
        #[async_trait::async_trait]
        impl ExecutionHarness for StaticHarness {
            fn declaration(
                &self,
            ) -> &prometheos_lite::workflow::execution_harness::HarnessDeclarationV1 {
                &self.declaration
            }
            async fn run(
                &self,
                _s: HarnessStep,
                _d: &mut HarnessDispatch<'_>,
            ) -> anyhow::Result<HarnessOutcome> {
                anyhow::bail!("static probe")
            }
        }
        let mut reg = HarnessRegistry::new();
        for d in &decls {
            reg.register(Box::new(StaticHarness {
                declaration: d.clone(),
            }))
            .expect("registers");
        }
        reg.catalog()
    };
    let drifted_catalog = {
        struct StaticHarness {
            declaration: prometheos_lite::workflow::execution_harness::HarnessDeclarationV1,
        }
        #[async_trait::async_trait]
        impl ExecutionHarness for StaticHarness {
            fn declaration(
                &self,
            ) -> &prometheos_lite::workflow::execution_harness::HarnessDeclarationV1 {
                &self.declaration
            }
            async fn run(
                &self,
                _s: HarnessStep,
                _d: &mut HarnessDispatch<'_>,
            ) -> anyhow::Result<HarnessOutcome> {
                anyhow::bail!("static probe")
            }
        }
        let mut reg = HarnessRegistry::new();
        for d in &drifted {
            reg.register(Box::new(StaticHarness {
                declaration: d.clone(),
            }))
            .expect("registers");
        }
        reg.catalog()
    };
    assert_ne!(
        drifted_catalog.content_digest, base.content_digest,
        "a capability change must move the catalog digest"
    );
    assert!(
        !drifted_catalog
            .verify_against_declarations(&decls)
            .is_empty(),
        "drifted catalog must not verify against the canonical declarations"
    );
}

// ---------------------------------------------------------------------------
// Security negatives: malformed protocol refuses before evidence
// ---------------------------------------------------------------------------

/// Unsupported protocol version refuses (worker reports 9.9.9).
#[tokio::test]
async fn unsupported_protocol_version_refuses() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "bad-version"})), &mut d)
            .await
            .expect_err("bad version refuses")
    };
    assert!(err.to_string().contains("protocol version"), "got: {err}");
    assert!(runner.journal().is_empty(), "refusals never journal");
}

/// Malformed JSON refuses.
#[tokio::test]
async fn malformed_json_refuses() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "malformed"})), &mut d)
            .await
            .expect_err("malformed refuses")
    };
    assert!(err.to_string().contains("protocol response"), "got: {err}");
    assert!(runner.journal().is_empty());
}

/// Oversized request refuses before any worker is spawned.
#[tokio::test]
async fn oversized_request_refuses_before_spawn() {
    let root = tempfile::tempdir().expect("tempdir");
    let cfg = ProtocolBridgeConfig::default()
        .allowed_executables(vec![worker_exe()])
        .max_request_bytes(1024);
    let mut runner = runner_for(
        cfg,
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(serde_json::json!({"mode": "echo", "payload": "X".repeat(8192)})),
                &mut d,
            )
            .await
            .expect_err("oversized request refuses")
    };
    assert!(err.to_string().contains("protocol request"), "got: {err}");
    assert!(runner.journal().is_empty());
}

/// Missing required envelope fields refuse.
#[tokio::test]
async fn missing_required_fields_refuse() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "missing-field"})), &mut d)
            .await
            .expect_err("missing fields refuse")
    };
    assert!(err.to_string().contains("protocol response"), "got: {err}");
    assert!(runner.journal().is_empty());
}

/// Unknown capability refuses before any worker execution (dispatch law).
#[tokio::test]
async fn unknown_capability_refuses_before_execution() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut bad = step(serde_json::json!({"mode": "echo"}));
    bad.capability = "harness.protocol.unknown".to_string();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(bad, &mut d)
            .await
            .expect_err("unknown capability refuses")
    };
    assert!(
        err.to_string().contains("outside its declaration"),
        "got: {err}"
    );
    assert!(runner.journal().is_empty());
}

/// A worker reporting an undeclared capability refuses.
#[tokio::test]
async fn worker_reported_unknown_capability_refuses() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(
                step(serde_json::json!({"mode": "unknown-capability"})),
                &mut d,
            )
            .await
            .expect_err("worker capability refuses")
    };
    assert!(err.to_string().contains("capability"), "got: {err}");
    assert!(runner.journal().is_empty());
}

/// Unapproved executable refuses before spawn.
#[tokio::test]
async fn unapproved_executable_refuses() {
    let root = tempfile::tempdir().expect("tempdir");
    let cfg =
        ProtocolBridgeConfig::default().allowed_executables(vec!["not-the-worker".to_string()]);
    let mut runner = runner_for(
        cfg,
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "echo"})), &mut d)
            .await
            .expect_err("unapproved exe refuses")
    };
    assert!(err.to_string().contains("not allowlisted"), "got: {err}");
    assert!(runner.journal().is_empty());
}

/// Unauthorized workspace refuses without fallback (missing root and
/// escaping cwd).
#[tokio::test]
async fn unauthorized_workspace_refuses() {
    let base = tempfile::tempdir().expect("base tempdir");
    let workspace = base.path().join("ws");
    std::fs::create_dir(&workspace).expect("workspace dir");
    // A sibling directory that EXISTS outside the governed root so the
    // escaping cwd resolves and hits the escapes refusal (a non-existent
    // target hits the does-not-exist refusal first).
    std::fs::create_dir(base.path().join("outside")).expect("outside dir");

    // Escaping cwd refuses.
    let mut runner = runner_for(config(), workspace.clone(), &[PROTOCOL_BRIDGE_CAPABILITY]);
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let mut escaping = step(serde_json::json!({"mode": "echo"}));
    escaping.args["cwd"] = serde_json::json!("../outside");
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(escaping, &mut d)
            .await
            .expect_err("escaping cwd refuses")
    };
    assert!(err.to_string().contains("escapes"), "got: {err}");
    assert!(runner.journal().is_empty());

    // Missing workspace root refuses.
    let missing = base.path().join("does-not-exist");
    let mut runner = runner_for(config(), missing, &[PROTOCOL_BRIDGE_CAPABILITY]);
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "echo"})), &mut d)
            .await
            .expect_err("missing workspace refuses")
    };
    assert!(err.to_string().contains("missing"), "got: {err}");
    assert!(runner.journal().is_empty());
}

// ---------------------------------------------------------------------------
// Timeout, cancellation, flooding, exit
// ---------------------------------------------------------------------------

/// Timeout kills and reaps the worker promptly.
#[tokio::test]
async fn timeout_terminates_safely() {
    let root = tempfile::tempdir().expect("tempdir");
    let cfg = config().timeout_ms(10_000);
    let mut runner = runner_for(
        cfg,
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let started = Instant::now();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        let mut sleeping = step(serde_json::json!({"mode": "sleep"}));
        sleeping.args["timeoutMs"] = serde_json::json!(500);
        harness
            .run(sleeping, &mut d)
            .await
            .expect_err("sleeping worker must time out")
    };
    let elapsed = started.elapsed();
    assert!(err.to_string().contains("timed out"), "got: {err}");
    assert!(elapsed < Duration::from_secs(15), "wall time: {elapsed:?}");
}

/// Cancellation terminates the worker promptly.
#[tokio::test]
async fn cancellation_terminates_safely() {
    let root = tempfile::tempdir().expect("tempdir");
    let cancel = Arc::new(AtomicBool::new(false));
    let cfg = config().timeout_ms(60_000).cancel(cancel.clone());
    let mut runner = runner_for(
        cfg,
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let started = Instant::now();
    let err = std::thread::scope(|scope| {
        let harness_ref = &harness;
        let mut d = dispatch(harness_ref, &m, &r, &mut runner);
        let handle = scope.spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime")
                .block_on(harness_ref.run(step(serde_json::json!({"mode": "sleep"})), &mut d))
        });
        std::thread::sleep(Duration::from_millis(200));
        cancel.store(true, Ordering::Relaxed);
        handle.join().expect("run thread")
    })
    .expect_err("cancellation must refuse");
    let elapsed = started.elapsed();
    assert!(err.to_string().contains("cancelled"), "got: {err}");
    assert!(elapsed < Duration::from_secs(15), "wall time: {elapsed:?}");
}

/// Output flooding is bounded.
#[tokio::test]
async fn output_flooding_is_bounded() {
    let root = tempfile::tempdir().expect("tempdir");
    let cfg = config().max_output_bytes(4096);
    let mut runner = runner_for(
        cfg,
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "flood"})), &mut d)
            .await
            .expect_err("flood refuses")
    };
    assert!(err.to_string().contains("budget"), "got: {err}");
    assert!(runner.journal().is_empty());
}

/// Unexpected worker exit produces a typed failure.
#[tokio::test]
async fn unexpected_worker_exit_is_typed_failure() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "exit-nonzero"})), &mut d)
            .await
            .expect_err("non-zero exit refuses")
    };
    assert!(err.to_string().contains("exited with code"), "got: {err}");
    assert!(runner.journal().is_empty());
}

// ---------------------------------------------------------------------------
// Identity, revision, state, tool-request, secret negatives
// ---------------------------------------------------------------------------

/// Mismatched response identity refuses.
#[tokio::test]
async fn mismatched_response_identity_refuses() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "wrong-id"})), &mut d)
            .await
            .expect_err("wrong id refuses")
    };
    assert!(err.to_string().contains("requestId"), "got: {err}");
    assert!(runner.journal().is_empty());
}

/// Duplicate response identity refuses (second JSON document).
#[tokio::test]
async fn duplicate_response_refuses() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "extra-line"})), &mut d)
            .await
            .expect_err("duplicate response refuses")
    };
    assert!(err.to_string().contains("protocol response"), "got: {err}");
    assert!(runner.journal().is_empty());
}

/// Wrong harness revision refuses the response.
#[tokio::test]
async fn wrong_harness_revision_refuses() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "wrong-revision"})), &mut d)
            .await
            .expect_err("wrong revision refuses")
    };
    assert!(err.to_string().contains("harnessRevision"), "got: {err}");
    assert!(runner.journal().is_empty());
}

/// Worker-typed error is never reported as success.
#[tokio::test]
async fn worker_typed_error_refuses() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "error"})), &mut d)
            .await
            .expect_err("worker error refuses")
    };
    assert!(err.to_string().contains("worker.error"), "got: {err}");
    assert!(runner.journal().is_empty());
}

/// An external worker tool-request attempt cannot bypass NodeRunner.
#[tokio::test]
async fn worker_tool_request_cannot_bypass_governance() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "tool-request"})), &mut d)
            .await
            .expect_err("tool request refuses")
    };
    assert!(err.to_string().contains("tool request"), "got: {err}");
    assert!(runner.journal().is_empty());
    // The requested mutation ("pwned.txt") never landed.
    assert!(!root.path().join("pwned.txt").exists());
}

/// Secret-bearing response cannot leak into evidence.
#[tokio::test]
async fn secret_bearing_response_is_redacted() {
    let root = tempfile::tempdir().expect("tempdir");
    let canary = "OPAQUE-CANARY-1234-no-regex-shape";
    let mut runner = runner_for(
        config(),
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let outcome = {
        let mut d = dispatch_with_secrets(&harness, &m, &r, &mut runner, vec![canary.to_string()]);
        harness
            .run(
                step(serde_json::json!({"mode": "echo", "payload": canary})),
                &mut d,
            )
            .await
            .expect("secret echo still succeeds with redaction")
    };
    assert!(
        !outcome.output.contains(canary),
        "canary leaked into evidence: {}",
        outcome.output
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&outcome.output).expect("output must remain valid JSON");
    assert_eq!(parsed["success"], true);
    // The verified output parsed cleanly and the secret is gone from every
    // retained string leaf.
    let leaf = parsed["output"]["echoed"]["payload"]
        .as_str()
        .expect("redacted leaf");
    assert!(!leaf.contains(canary), "canary in leaf: {leaf}");
}

/// Stale/mismatched portable state and wrong-revision checkpoints refuse.
#[tokio::test]
async fn stale_state_and_wrong_revision_resume_refuse() {
    let root = tempfile::tempdir().expect("tempdir");
    let outcome = {
        let mut runner = runner_for(
            config(),
            root.path().to_path_buf(),
            &[PROTOCOL_BRIDGE_CAPABILITY],
        );
        let harness = harness();
        let m = manifest();
        let r = restrictions();
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(serde_json::json!({"mode": "echo"})), &mut d)
            .await
            .expect("run succeeds")
    };
    let harness = harness();
    let digest = outcome.evidence_entry_digest.clone();
    let checkpoint = futures::executor::block_on(harness.checkpoint(&outcome, &digest))
        .expect("checkpoint exports");

    // Stale portable digest refuses.
    let err = futures::executor::block_on(harness.resume(&checkpoint, &"0".repeat(64)))
        .expect_err("stale digest refuses");
    assert!(err.to_string().contains("portable"), "got: {err}");

    // Wrong revision refuses.
    let mut wrong_rev = checkpoint.clone();
    wrong_rev.harness_revision = "v9".to_string();
    wrong_rev.content_digest = None;
    let wrong_rev = wrong_rev.sealed();
    let err = futures::executor::block_on(harness.resume(&wrong_rev, &digest))
        .expect_err("wrong revision refuses");
    assert!(err.to_string().contains("revision"), "got: {err}");

    // Tampered checkpoint (digest no longer verifies) refuses.
    let mut tampered = checkpoint.clone();
    tampered.portable_state_digest = "1".repeat(64);
    let err = futures::executor::block_on(harness.resume(&tampered, &"1".repeat(64)))
        .expect_err("tampered checkpoint refuses");
    assert!(err.to_string().contains("contentDigest"), "got: {err}");

    // A checkpoint from another harness refuses.
    let subprocess = SubprocessHarness::new();
    let foreign = futures::executor::block_on(subprocess.checkpoint(&outcome, &digest))
        .expect("subprocess exports");
    let err = futures::executor::block_on(harness.resume(&foreign, &digest))
        .expect_err("foreign checkpoint refuses");
    assert!(err.to_string().contains("was exported by"), "got: {err}");
}

// ---------------------------------------------------------------------------
// P1 — supervised stdin write (blocking-write lifecycle repair)
// ---------------------------------------------------------------------------

/// P1a — A non-reading worker cannot block the invocation: deadline covers
/// spawn, write, and wait; the blocked writer is terminated and joined.
#[tokio::test]
async fn non_reading_worker_times_out_promptly() {
    let root = tempfile::tempdir().expect("tempdir");
    let cfg = large_config().timeout_ms(10_000);
    let mut runner = runner_for(
        cfg,
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let started = Instant::now();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        // Large envelope (> OS pipe capacity, < raised budgets) into a
        // worker that never reads stdin: the writer blocks mid-write and
        // must be killed/joined by the deadline, not hang forever.
        let mut stuck = step_with_argv(vec!["--never-read".to_string()], large_input());
        stuck.args["timeoutMs"] = serde_json::json!(500);
        harness
            .run(stuck, &mut d)
            .await
            .expect_err("non-reading worker must time out")
    };
    let elapsed = started.elapsed();
    assert!(
        err.to_string().contains("timed out"),
        "deadline must fire during blocked write, got: {err}"
    );
    assert!(
        elapsed < Duration::from_secs(15),
        "must return promptly, wall time: {elapsed:?}"
    );
    assert!(runner.journal().is_empty(), "refusals never journal");
}

/// P1b — Cancellation works during a blocked stdin write.
#[tokio::test]
async fn non_reading_worker_cancellation_unblocks_write() {
    let root = tempfile::tempdir().expect("tempdir");
    let cancel = Arc::new(AtomicBool::new(false));
    let cfg = large_config().timeout_ms(60_000).cancel(cancel.clone());
    let mut runner = runner_for(
        cfg,
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let started = Instant::now();
    let err = std::thread::scope(|scope| {
        let harness_ref = &harness;
        let mut d = dispatch(harness_ref, &m, &r, &mut runner);
        let handle = scope.spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime")
                .block_on(harness_ref.run(
                    step_with_argv(vec!["--never-read".to_string()], large_input()),
                    &mut d,
                ))
        });
        std::thread::sleep(Duration::from_millis(200));
        cancel.store(true, Ordering::Relaxed);
        handle.join().expect("run thread")
    })
    .expect_err("cancellation must refuse during blocked write");
    let elapsed = started.elapsed();
    assert!(
        err.to_string().contains("cancelled"),
        "cancel must fire during blocked write, got: {err}"
    );
    assert!(
        elapsed < Duration::from_secs(15),
        "must return promptly, wall time: {elapsed:?}"
    );
    assert!(runner.journal().is_empty());
}

/// P1c — Early worker exit (broken pipe) terminates and reaps the child.
#[tokio::test]
async fn early_worker_exit_broken_pipe_cleanup() {
    let root = tempfile::tempdir().expect("tempdir");
    let cfg = large_config().timeout_ms(10_000);
    let mut runner = runner_for(
        cfg,
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let started = Instant::now();
    let err = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        // Large request into a worker that exits immediately without
        // reading: the writer breaks (EPIPE) or the child exits; either
        // way the invocation ends promptly with the child reaped.
        harness
            .run(
                step_with_argv(vec!["--exit-now".to_string()], large_input()),
                &mut d,
            )
            .await
            .expect_err("early exit must refuse")
    };
    let elapsed = started.elapsed();
    let message = err.to_string();
    assert!(
        message.contains("stdin failed") || message.contains("exited with code"),
        "broken-pipe or exit refusal, got: {err}"
    );
    assert!(
        elapsed < Duration::from_secs(15),
        "must return promptly, wall time: {elapsed:?}"
    );
    assert!(runner.journal().is_empty());
}

/// P1d — A large request still succeeds when the worker does read stdin
/// (the supervised writer completes promptly; no regression).
#[tokio::test]
async fn large_request_succeeds_when_worker_reads() {
    let root = tempfile::tempdir().expect("tempdir");
    let cfg = large_config();
    let mut runner = runner_for(
        cfg,
        root.path().to_path_buf(),
        &[PROTOCOL_BRIDGE_CAPABILITY],
    );
    let harness = harness();
    let m = manifest();
    let r = restrictions();
    let outcome = {
        let mut d = dispatch(&harness, &m, &r, &mut runner);
        harness
            .run(step(large_input()), &mut d)
            .await
            .expect("large read request succeeds")
    };
    let parsed: serde_json::Value =
        serde_json::from_str(&outcome.output).expect("result JSON parses");
    let echoed = parsed["output"]["echoed"]["payload"]
        .as_str()
        .expect("echoed payload");
    assert_eq!(echoed.len(), 150_000, "full payload round-tripped");
    assert_eq!(runner.journal().len(), 1);
}
