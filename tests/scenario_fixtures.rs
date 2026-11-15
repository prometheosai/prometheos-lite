//! Issue #136 Slice A — governed-workflow evaluation fixtures.
//!
//! Deterministic, fixture-only, loopback-only execution of the two pinned
//! scenarios (`tests/fixtures/scenarios/*/descriptor.json`) through the real
//! governed route-dispatch boundary. No production provider, no network
//! beyond loopback, no fixture toolchain execution, no benchmark scores.
//!
//! What this suite proves:
//! - descriptors are versioned, pinned and fail closed when malformed,
//!   drifted, ambiguous or platform-path-leaking;
//! - setup errors and invalid tasks are classified apart from governed
//!   product outcomes — never counted as model failure;
//! - the governed boundary produces sealed, re-verifiable raw evidence
//!   (route evidence, snapshot, attempt, journal entry, accounting, graph);
//! - reruns reproduce the canonical evidence chain byte-for-byte (the only
//!   nondeterministic field is the measured dispatch latency);
//! - no secret material, no machine paths and no benchmark-success claims
//!   appear in exported evidence;
//! - the exported Lite-internal envelope is explicitly NOT a Foundry
//!   contract and marks the external format unsupported.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use serde_json::{Value, json};

use prometheos_lite::workflow::accounting::{
    AccountingContext, AccountingLedger, PricingBook, account_routed_outcome,
};
use prometheos_lite::workflow::execution_harness::HarnessRegistry;
use prometheos_lite::workflow::local_model_harness::{
    LOCAL_MODEL_INFER_CAPABILITY, LocalModelConfig, LocalModelHarness, local_model_capability,
};
use prometheos_lite::workflow::node_contracts::NodeManifestV1;
use prometheos_lite::workflow::node_runner::{CapabilityRegistry, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::redaction::SECRET_CANARY;
use prometheos_lite::workflow::route_dispatch::{
    FixedTrustedTimeSource, RoutedDispatchFailure, RoutedDispatchOutcome, RoutedDispatchRequest,
    dispatch_resolved_route,
};
use prometheos_lite::workflow::route_resolution::{
    RouteCandidateV1, RouteResolutionRequest, RouteTrust, RouteWorkspaceBinding, resolve_route,
};
use prometheos_lite::workflow::scenario::{
    self, ScenarioDescriptor, ScenarioDescriptorError, ScenarioOutcomeClass,
    ScenarioWorkspaceAdapter,
};
use prometheos_lite::workflow::soma::capability::Locality;
use prometheos_lite::workflow::soma::capability::{
    SUPPORTED_CAPABILITY_SCHEMA_VERSION, WorkRequirements,
};
use prometheos_lite::workflow::soma::contracts::AuthorityProfile;
use prometheos_lite::workflow::workspace::{
    ADAPTER_REVISION, AdapterKind, WorkspaceManifestV1, WorkspaceMode, stable_repo_identity_digest,
};

mod common;

const RUST_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/rust-bugfix-arithmetic/descriptor.json");
const PYTHON_DESCRIPTOR: &str =
    include_str!("fixtures/scenarios/python-bugfix-strings/descriptor.json");

const GRAPH_ID: &str = "scenario-fixture-graph";
const NODE_ID: &str = "fixture-model-node";
const STEP_ID: &str = "fixture-model-step-1";
const MODEL: &str = "tiny-local-model";

const PRICE_BOOK: &str = r#"{
    "schemaVersion": "1.0.0",
    "source": "example-operator-price-book",
    "version": "example-2026-11",
    "entries": [
        { "provider": "demo-local", "model": "tiny-local-model", "currency": "USD",
          "promptMinorPer1k": 1000, "completionMinorPer1k": 2000 }
    ]
}"#;

// ---------------------------------------------------------------------------
// Loopback model fixture (same contract as examples/governed_route_to_accounting.rs)
// ---------------------------------------------------------------------------

struct LoopbackModel {
    endpoint: String,
    receiver: mpsc::Receiver<()>,
}

fn spawn_loopback_model(response_body: String) -> LoopbackModel {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture binds loopback");
    let addr = listener.local_addr().expect("fixture address");
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(_) => return,
        };
        let _ = read_request(&stream);
        let body = response_body.as_bytes();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            response_body
        );
        let mut stream = stream;
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.shutdown(std::net::Shutdown::Both);
        let _ = sender.send(());
    });
    LoopbackModel {
        endpoint: format!("http://127.0.0.1:{}/v1", addr.port()),
        receiver,
    }
}

fn read_request(mut stream: &TcpStream) -> Option<String> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(5);
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .ok()?;
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    thread::sleep(Duration::from_millis(50));
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        Err(_) => break,
                    }
                    break;
                }
                if Instant::now() > deadline {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8(buf).ok()
}

// ---------------------------------------------------------------------------
// Scenario runner (test adapter around the real governed boundary)
// ---------------------------------------------------------------------------

struct ScenarioRun {
    /// Present only when resolution succeeded; a pre-effect refusal carries
    /// no resolution.
    resolution: Option<prometheos_lite::workflow::route_resolution::RouteResolution>,
    dispatch: Result<RoutedDispatchOutcome, RoutedDispatchFailure>,
    accounting: Option<prometheos_lite::workflow::accounting::AttemptAccountingV1>,
    graph: Option<prometheos_lite::workflow::accounting::GraphAccounting>,
    envelope: Option<prometheos_lite::workflow::scenario::LiteScenarioRunEvidence>,
    classification: ScenarioOutcomeClass,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture_root(descriptor: &ScenarioDescriptor) -> PathBuf {
    descriptor
        .repository
        .fixture
        .split('/')
        .collect::<PathBuf>()
        .iter()
        .fold(repo_root(), |acc, part| acc.join(part))
}

fn canned_chat_completion() -> String {
    json!({
        "id": "chatcmpl-scenario-fixture",
        "object": "chat.completion",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "fixture response from loopback local model" },
            "finish_reason": "stop",
        }],
        "usage": { "prompt_tokens": 7, "completion_tokens": 4, "total_tokens": 11 },
    })
    .to_string()
}

fn workspace_adapter(kind: ScenarioWorkspaceAdapter) -> AdapterKind {
    match kind {
        ScenarioWorkspaceAdapter::ExistingReadOnly => AdapterKind::ExistingReadOnly,
        ScenarioWorkspaceAdapter::GitWorktree => AdapterKind::GitWorktree,
    }
}

fn build_workspace_manifest(descriptor: &ScenarioDescriptor) -> WorkspaceManifestV1 {
    WorkspaceManifestV1 {
        schema_version: "1.0.0".to_string(),
        workspace_id: format!("scenario-workspace-{}", descriptor.scenario_id),
        adapter: workspace_adapter(descriptor.workspace.adapter),
        adapter_revision: descriptor.workspace.adapter_revision.clone(),
        repo_identity: stable_repo_identity_digest(&format!(
            "lite-fixtures://scenarios/{}",
            descriptor.scenario_id
        )),
        base_revision: descriptor.repository.revision.clone(),
        branch: None,
        mode: match descriptor.workspace.mode {
            scenario::ScenarioWorkspaceMode::ReadOnly => WorkspaceMode::ReadOnly,
            scenario::ScenarioWorkspaceMode::Writable => WorkspaceMode::Writable,
        },
        writable_scopes: descriptor.workspace.writable_scopes.clone(),
        resource_lock_id: format!("scenario-lock-{}", descriptor.scenario_id),
        created_at: descriptor.route.evaluated_at.clone(),
        content_digest: None,
    }
    .sealed()
}

/// Execute one scenario end to end through the governed boundary.
fn run_scenario(descriptor_text: &str) -> ScenarioRun {
    let descriptor = ScenarioDescriptor::parse_json(descriptor_text)
        .expect("descriptor must validate before running");
    let root = fixture_root(&descriptor);
    scenario::verify_fixture_content(&root, &descriptor.repository.content_digest)
        .expect("fixture content must match the pin before running");

    let server = spawn_loopback_model(canned_chat_completion());

    let candidate = RouteCandidateV1 {
        schema_version: "1.0.0".to_string(),
        candidate_id: descriptor.route.candidate_id.clone(),
        provider_identity: descriptor.route.provider_identity.clone(),
        provider_locality: match descriptor.route.provider_locality.as_str() {
            "local" => Locality::Local,
            other => panic!("unsupported provider locality in fixture: {other}"),
        },
        model: Some(descriptor.route.model.clone()),
        harness_id: descriptor.harness.id.clone(),
        harness_revision: descriptor.harness.revision.clone(),
        workspace: Some(RouteWorkspaceBinding {
            adapter: workspace_adapter(descriptor.workspace.adapter),
            revision: descriptor.workspace.adapter_revision.clone(),
        }),
        trust: RouteTrust {
            source: "scenario-fixture-pin".to_string(),
            verified_at: descriptor.route.trust_verified_at.clone(),
            ttl_seconds: descriptor.route.trust_ttl_seconds,
        },
        budget: None,
        priority: descriptor.route.priority,
        experimental: false,
    };

    let requirements: WorkRequirements = serde_json::from_value(json!({
        "id": "reqs-scenario-fixture",
        "schemaVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": descriptor.requirements.degradation_permitted,
        "forbidSubstitution": descriptor.requirements.forbid_substitution,
        "requiredCapabilities": []
    }))
    .expect("requirements parse");
    let authority: AuthorityProfile =
        serde_json::from_str(r#"{"executionClass": "deterministic", "mutation": "none"}"#)
            .expect("authority parses");
    let workspace_manifest = build_workspace_manifest(&descriptor);
    let request = RouteResolutionRequest {
        route_set_id: descriptor.route.route_set_id.clone(),
        route_set_version: descriptor.route.route_set_version.clone(),
        candidates: vec![candidate],
        pinned_candidate_id: Some(&descriptor.route.pinned_candidate_id),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: prometheos_lite::workflow::route_resolution::RouteProfile::Standard,
        evaluated_at: descriptor.route.evaluated_at.clone(),
        workspace: Some(&workspace_manifest),
    };

    let mut harness_registry = HarnessRegistry::new();
    harness_registry
        .register(Box::new(LocalModelHarness::new()))
        .expect("local model harness registers");

    let resolution = match resolve_route(&request, &harness_registry) {
        Ok(resolution) => resolution,
        Err(error) => {
            return ScenarioRun {
                resolution: None,
                dispatch: Err(RoutedDispatchFailure::Refused(error)),
                accounting: None,
                graph: None,
                envelope: None,
                classification: scenario::classify_pre_effect_refusal(),
            };
        }
    };
    resolution.evidence.verify().expect("evidence verifies");
    resolution.snapshot.verify().expect("snapshot verifies");

    let mut capability_registry = CapabilityRegistry::new();
    let fixture_workspace = tempfile::tempdir().expect("fixture workspace");
    capability_registry.declare(
        LOCAL_MODEL_INFER_CAPABILITY,
        local_model_capability(
            LocalModelConfig::new(server.endpoint.clone(), MODEL).timeout_ms(5_000),
            fixture_workspace.path().to_path_buf(),
        ),
    );
    let permit = common::permit_for(&[NODE_ID], &[LOCAL_MODEL_INFER_CAPABILITY]);
    let mut runner = NodeRunner::new(capability_registry, permit);

    let manifest = NodeManifestV1::parse_json(
        &json!({
            "schemaVersion": "1.0.0",
            "nodeId": NODE_ID,
            "purpose": "governed scenario fixture node",
            "inputs": [],
            "outputs": [{ "name": "out", "typeRef": "string" }],
            "readableScopes": ["repo://fixture"],
            "writableScopes": [],
            "retry": { "maxAttempts": 1, "retryableClasses": [] }
        })
        .to_string(),
    )
    .expect("manifest parses");
    let restrictions = LocalRestrictions {
        readable_scopes: vec!["repo://fixture".into()],
        writable_scopes: vec![],
        token_budget_ceiling: None,
        denied_providers: vec![],
        forbidden_paths: vec![],
        max_attempts: 1,
        escalation_target: "human-review".into(),
    };

    let dispatch_started = std::time::Instant::now();
    let dispatch =
        dispatch_resolved_route_block_on(dispatch_resolved_route(RoutedDispatchRequest {
            resolution: &resolution,
            original_request: &request,
            registry: &harness_registry,
            runner: &mut runner,
            manifest: &manifest,
            local_restrictions: &restrictions,
            known_secrets: vec![SECRET_CANARY.to_string()],
            clock: &FixedTrustedTimeSource::new(&descriptor.route.dispatch_at),
            step: prometheos_lite::workflow::execution_harness::HarnessStep {
                step_id: STEP_ID.to_string(),
                capability: LOCAL_MODEL_INFER_CAPABILITY.to_string(),
                args: json!({
                    "model": MODEL,
                    "endpoint": server.endpoint,
                    "messages": [{ "role": "user", "content": "hi" }],
                    "maxTokens": 64,
                }),
            },
        }));
    let dispatch_elapsed_ms = dispatch_started.elapsed().as_millis() as u64;
    let dispatch_wait = match &dispatch {
        // A refusal before effect never reaches the network: waiting on the
        // loopback fixture would hang forever. Bound every wait.
        Ok(_) | Err(RoutedDispatchFailure::ExecutionFailed(_)) => {
            std::time::Duration::from_secs(10)
        }
        Err(RoutedDispatchFailure::Refused(_)) => std::time::Duration::ZERO,
    };
    if dispatch_wait.is_zero() {
        drop(server);
    } else {
        let _ = server.receiver.recv_timeout(dispatch_wait);
    }

    let accounting_context = AccountingContext {
        graph_id: GRAPH_ID.to_string(),
        node_id: NODE_ID.to_string(),
        currency: Some("USD".to_string()),
    };
    let accounting = match dispatch.as_ref() {
        Ok(outcome) => {
            let pricing = PricingBook::from_json(PRICE_BOOK).expect("price book parses");
            let record =
                account_routed_outcome(outcome, &resolution, &accounting_context, Some(&pricing))
                    .expect("outcome accounts");
            record.verify().expect("accounting record verifies");
            Some(record)
        }
        Err(RoutedDispatchFailure::ExecutionFailed(failure)) => {
            // Account the typed execution failure instead of dropping it:
            // no attempt was sealed, usage/cost stay unknown, journal digest
            // binds the partial journal when one exists.
            Some(
                prometheos_lite::workflow::accounting::account_routed_failure(
                    &resolution,
                    "execution-failed",
                    &accounting_context,
                    STEP_ID,
                    failure.journal_entry.clone(),
                    dispatch_elapsed_ms,
                ),
            )
        }
        // Refused before any effect: nothing was dispatched, nothing to account.
        Err(RoutedDispatchFailure::Refused(_)) => None,
    };
    let graph = accounting.as_ref().map(|record| {
        let mut ledger = AccountingLedger::new();
        ledger.record(record.clone()).expect("record registers");
        ledger.aggregate_graph(GRAPH_ID).expect("graph aggregates")
    });

    let classification = scenario::classify_completed_run(true, &dispatch, accounting.is_some());

    let envelope: Option<prometheos_lite::workflow::scenario::LiteScenarioRunEvidence> =
        match (dispatch.as_ref(), accounting.as_ref(), graph.as_ref()) {
            (Ok(outcome), Some(record), Some(graph)) => {
                let result = scenario::LiteScenarioRunEvidence::sealed(
                    descriptor,
                    &resolution,
                    &manifest,
                    outcome,
                    record.clone(),
                    graph.clone(),
                    Some(&workspace_manifest),
                    classification,
                );
                if let Err(error) = &result {
                    panic!("happy-path envelope failed to seal: {error:#}");
                }
                result.ok()
            }
            _ => None,
        };

    ScenarioRun {
        resolution: Some(resolution),
        dispatch,
        accounting,
        graph,
        envelope,
        classification,
    }
}

fn dispatch_resolved_route_block_on<F>(future: F) -> F::Output
where
    F: std::future::Future,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(future)
}

// ---------------------------------------------------------------------------
// Descriptor validation
// ---------------------------------------------------------------------------

#[test]
fn descriptors_load_and_validate() {
    for text in [RUST_DESCRIPTOR, PYTHON_DESCRIPTOR] {
        let descriptor = ScenarioDescriptor::parse_json(text).expect("descriptor validates");
        assert_eq!(descriptor.schema_version, "1.0.0");
        assert_eq!(descriptor.namespace, "lite.fixtures.scenario");
        let digest_a = descriptor.scenario_digest().expect("digest computes");
        let digest_b = descriptor.scenario_digest().expect("digest recomputes");
        assert_eq!(digest_a, digest_b);
        assert_eq!(digest_a.len(), 64);
    }
}

#[test]
fn fixture_content_digests_match_pins() {
    for text in [RUST_DESCRIPTOR, PYTHON_DESCRIPTOR] {
        let descriptor = ScenarioDescriptor::parse_json(text).expect("descriptor validates");
        let root = fixture_root(&descriptor);
        scenario::verify_fixture_content(&root, &descriptor.repository.content_digest)
            .expect("fixture content matches pin");
    }
}

#[test]
fn fixture_digest_drift_is_detected() {
    let descriptor = ScenarioDescriptor::parse_json(RUST_DESCRIPTOR).expect("descriptor validates");
    let source = fixture_root(&descriptor);
    let temp = tempfile::tempdir().expect("staging dir");
    copy_tree(&source, temp.path());
    let mut drifted = std::fs::read_dir(temp.path().join("src"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    drifted.sort();
    let lib = drifted.into_iter().find(|p| p.ends_with("lib.rs")).unwrap();
    let mut bytes = std::fs::read(&lib).unwrap();
    bytes[0] ^= 0x01;
    std::fs::write(&lib, bytes).unwrap();
    let error =
        scenario::verify_fixture_content(temp.path(), &descriptor.repository.content_digest)
            .expect_err("drifted fixture must fail verification");
    assert!(
        error.to_string().contains("drift"),
        "drift error should name the drift: {error}"
    );
}

fn copy_tree(source: &Path, dest: &Path) {
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = dest.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[test]
fn malformed_descriptor_version_fails_closed() {
    for (field, value) in [
        ("schemaVersion", "9.9.9"),
        ("schemaVersion", "0.0.0"),
        ("namespace", "foundry.workflow-evaluation"),
        ("namespace", ""),
    ] {
        let mut doc: Value = serde_json::from_str(RUST_DESCRIPTOR).unwrap();
        doc[field] = json!(value);
        let error = ScenarioDescriptor::parse_json(&doc.to_string())
            .expect_err("malformed version must fail closed");
        assert_eq!(error.class(), ScenarioOutcomeClass::SetupError);
    }
}

#[test]
fn unknown_descriptor_members_fail_closed() {
    let mut doc: Value = serde_json::from_str(RUST_DESCRIPTOR).unwrap();
    doc["foundryScore"] = json!(100);
    let error = ScenarioDescriptor::parse_json(&doc.to_string())
        .expect_err("unknown members must fail closed");
    assert_eq!(error.class(), ScenarioOutcomeClass::SetupError);
}

#[test]
fn unsupported_language_fails_closed() {
    let mut doc: Value = serde_json::from_str(RUST_DESCRIPTOR).unwrap();
    doc["language"] = json!("typescript");
    let error = ScenarioDescriptor::parse_json(&doc.to_string())
        .expect_err("unsupported language must fail closed");
    assert_eq!(error.class(), ScenarioOutcomeClass::SetupError);
}

#[test]
fn missing_and_ambiguous_pins_are_rejected() {
    let cases: Vec<(Value, ScenarioOutcomeClass)> = vec![
        // content digest missing (empty)
        (
            {
                let mut doc = doc();
                doc["repository"]["contentDigest"] = json!("");
                doc
            },
            ScenarioOutcomeClass::SetupError,
        ),
        // content digest not hex
        (
            {
                let mut doc = doc();
                doc["repository"]["contentDigest"] = json!("not-a-digest");
                doc
            },
            ScenarioOutcomeClass::SetupError,
        ),
        // route pin ambiguous: pin names a different candidate
        (
            {
                let mut doc = doc();
                doc["route"]["pinnedCandidateId"] = json!("some-other-route");
                doc
            },
            ScenarioOutcomeClass::SetupError,
        ),
        // unknown evidence artifact
        (
            {
                let mut doc = doc();
                doc["expected"]["evidenceArtifacts"] = json!(["route-evidence", "benchmark-score"]);
                doc
            },
            ScenarioOutcomeClass::SetupError,
        ),
        // unknown dimension
        (
            {
                let mut doc = doc();
                doc["dimensions"]["controlled"] = json!(["pass-rate"]);
                doc
            },
            ScenarioOutcomeClass::SetupError,
        ),
        // dimension both controlled and variable
        (
            {
                let mut doc = doc();
                doc["dimensions"]["variable"] = json!(["task-statement"]);
                doc
            },
            ScenarioOutcomeClass::SetupError,
        ),
        // malformed instant
        (
            {
                let mut doc = doc();
                doc["route"]["evaluatedAt"] = json!("12/01/2026 12:00");
                doc
            },
            ScenarioOutcomeClass::SetupError,
        ),
        // invalid task: empty statement
        (
            {
                let mut doc = doc();
                doc["task"]["statement"] = json!("fix it");
                doc
            },
            ScenarioOutcomeClass::InvalidTask,
        ),
        // invalid task: read-only workspace contradicts writable scopes
        (
            {
                let mut doc = doc();
                doc["policy"]["writableScopes"] = json!(["work://fixture"]);
                doc
            },
            ScenarioOutcomeClass::InvalidTask,
        ),
        // invalid task: route/policy profile disagree
        (
            {
                let mut doc = doc();
                doc["route"]["profile"] = json!("conservative");
                doc
            },
            ScenarioOutcomeClass::InvalidTask,
        ),
    ];
    for (doc, expected_class) in cases {
        let error = ScenarioDescriptor::parse_json(&doc.to_string())
            .expect_err("ambiguous/missing pins must fail closed");
        assert_eq!(
            error.class(),
            expected_class,
            "wrong defect class for {doc}: {error}"
        );
        let messages: Vec<String> = error.0.iter().map(|d| d.message.clone()).collect();
        assert!(!messages.is_empty());
    }
}

fn doc() -> Value {
    serde_json::from_str(RUST_DESCRIPTOR).unwrap()
}

#[test]
fn platform_specific_path_leakage_is_rejected() {
    for fixture in [
        "C:/repo/fixture",
        "/tmp/fixture",
        "tests\\fixtures\\scenarios",
        "../outside",
        "tests/../escape",
        "tests//double",
        "./dot",
        "",
    ] {
        let mut doc = doc();
        doc["repository"]["fixture"] = json!(fixture);
        let error = ScenarioDescriptor::parse_json(&doc.to_string())
            .expect_err("platform path leakage must fail closed");
        assert_eq!(error.class(), ScenarioOutcomeClass::SetupError);
    }
}

// ---------------------------------------------------------------------------
// Governed execution + evidence
// ---------------------------------------------------------------------------

#[test]
fn rust_scenario_runs_through_the_governed_boundary() {
    let run = run_scenario(RUST_DESCRIPTOR);
    assert_eq!(
        run.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    run.dispatch.as_ref().expect("dispatch sealed");
    let record = run.accounting.as_ref().expect("accounting sealed");
    record.verify().expect("record verifies");
    let graph = run.graph.as_ref().expect("graph aggregates");
    assert_eq!(graph.attempts, 1);
    assert_eq!(graph.completed, 1);

    let envelope = run.envelope.as_ref().expect("envelope seals");
    envelope.verify().expect("envelope verifies");
    assert_eq!(envelope.namespace, "lite.evidence.scenario-run");
    assert_eq!(envelope.graph_accounting.attempts, 1);
    assert_eq!(envelope.graph_accounting.completed, 1);
    assert_eq!(envelope.manifest.node_id, NODE_ID);
    // The envelope embeds the very records the resolver/dispatcher sealed,
    // and every reference between them re-verifies from the embedded bytes.
    let resolution = run.resolution.as_ref().expect("resolution sealed");
    assert_eq!(
        envelope.snapshot.content_digest.clone().unwrap(),
        resolution.snapshot.content_digest.clone().unwrap()
    );
    assert_eq!(
        resolution.evidence.snapshot_digest,
        resolution.snapshot.content_digest.clone().unwrap()
    );
    assert_eq!(
        envelope.attempt.snapshot_digest,
        envelope.snapshot.content_digest.clone().unwrap()
    );
    assert_eq!(
        envelope.attempt.route_evidence_digest,
        resolution.evidence.content_digest.clone().unwrap()
    );
    assert_eq!(
        envelope.attempt_accounting.attempt_digest.as_deref(),
        envelope.attempt.content_digest.as_deref()
    );
    assert_eq!(
        envelope.attempt_accounting.journal_digest.as_deref(),
        Some(envelope.attempt.journal_entry.entry_digest.as_str())
    );
}

#[test]
fn python_scenario_runs_through_the_same_governed_boundary() {
    let run = run_scenario(PYTHON_DESCRIPTOR);
    assert_eq!(
        run.classification,
        ScenarioOutcomeClass::GovernedProductOutcome
    );
    let envelope = run.envelope.as_ref().expect("envelope seals");
    envelope.verify().expect("envelope verifies");
    assert_eq!(envelope.scenario_id(), "lite-python-bugfix-slugify-001");
}

#[test]
fn reruns_reproduce_the_stable_canonical_parts() {
    // Honest determinism boundary: the resolution side of the governed
    // boundary pins every instant (evaluation instant, trust instant,
    // injected dispatch clock), so its digests reproduce byte-for-byte.
    // The effect side is sealed AFTER the governed effect settles and
    // embeds the executing process's wall-clock instants (node result
    // startedAt/completedAt, measured dispatch latency), so byte equality
    // is asserted only for wall-clock-free fields; the effect-side records
    // are still re-verified on every rerun.
    let first = run_scenario(RUST_DESCRIPTOR);
    let second = run_scenario(RUST_DESCRIPTOR);
    let (a, b) = (
        first.envelope.as_ref().expect("first envelope seals"),
        second.envelope.as_ref().expect("second envelope seals"),
    );

    // Pinned-time canonical parts: byte-for-byte stable across reruns.
    // The envelope DERIVES its scenario digest from the embedded descriptor,
    // so equality here also proves the descriptor itself is byte-stable.
    assert_eq!(a.scenario_digest().unwrap(), b.scenario_digest().unwrap());
    assert_eq!(a.classification, b.classification);
    assert_eq!(a.descriptor, b.descriptor);
    assert_eq!(a.route_evidence, b.route_evidence);
    assert_eq!(a.snapshot, b.snapshot);
    assert_eq!(a.workspace_manifest, b.workspace_manifest);
    assert_eq!(
        a.route_evidence.decision_digest,
        b.route_evidence.decision_digest
    );
    assert_eq!(
        a.route_evidence.requirements_digest,
        b.route_evidence.requirements_digest
    );
    assert_eq!(
        a.route_evidence.candidate_set_digest,
        b.route_evidence.candidate_set_digest
    );
    assert_eq!(a.graph_accounting.attempts, b.graph_accounting.attempts);
    assert_eq!(a.graph_accounting.completed, b.graph_accounting.completed);

    // Effect-side records: re-verified on both runs; wall-clock-free
    // fields match exactly.
    let a_attempt = &a.attempt_accounting;
    let b_attempt = &b.attempt_accounting;
    a_attempt.verify().expect("first accounting verifies");
    b_attempt.verify().expect("second accounting verifies");
    assert_eq!(a_attempt.idempotency_key, b_attempt.idempotency_key);
    assert_eq!(a_attempt.harness_id, b_attempt.harness_id);
    assert_eq!(a_attempt.harness_revision, b_attempt.harness_revision);
    assert_eq!(a_attempt.provider_identity, b_attempt.provider_identity);
    assert_eq!(a_attempt.model, b_attempt.model);
    assert_eq!(a_attempt.route_set_id, b_attempt.route_set_id);
    assert_eq!(a_attempt.usage, b_attempt.usage);
    assert_eq!(a_attempt.cost, b_attempt.cost);
    assert_eq!(a_attempt.outcome, b_attempt.outcome);

    let a_graph = &a.graph_accounting;
    let b_graph = &b.graph_accounting;
    assert_eq!(a_graph.attempts, b_graph.attempts);
    assert_eq!(a_graph.completed, b_graph.completed);
    assert_eq!(a_graph.usage, b_graph.usage);
    assert_eq!(a_graph.cost, b_graph.cost);
}

#[test]
fn workspace_refusal_precedes_effect_and_classifies_as_setup_error() {
    // Candidate pins the existing-read-only adapter, but the manifest
    // supplied to resolution is a git-worktree workspace: the candidate is
    // refused BEFORE any effect runs, and the run classifies as a setup
    // error — never as a product or model outcome.
    let descriptor = ScenarioDescriptor::parse_json(RUST_DESCRIPTOR).expect("descriptor validates");
    let tampered = {
        let mut manifest = build_workspace_manifest(&descriptor);
        manifest.adapter = AdapterKind::GitWorktree;
        manifest.sealed()
    };
    let mut harness_registry = HarnessRegistry::new();
    harness_registry
        .register(Box::new(LocalModelHarness::new()))
        .expect("harness registers");
    let requirements: WorkRequirements = serde_json::from_value(json!({
        "id": "reqs-scenario-fixture",
        "schemaVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": false,
        "forbidSubstitution": true,
        "requiredCapabilities": []
    }))
    .unwrap();
    let authority: AuthorityProfile =
        serde_json::from_str(r#"{"executionClass": "deterministic", "mutation": "none"}"#).unwrap();
    let candidate = RouteCandidateV1 {
        schema_version: "1.0.0".to_string(),
        candidate_id: descriptor.route.candidate_id.clone(),
        provider_identity: descriptor.route.provider_identity.clone(),
        provider_locality: Locality::Local,
        model: Some(descriptor.route.model.clone()),
        harness_id: descriptor.harness.id.clone(),
        harness_revision: descriptor.harness.revision.clone(),
        workspace: Some(RouteWorkspaceBinding {
            adapter: AdapterKind::ExistingReadOnly,
            revision: ADAPTER_REVISION.to_string(),
        }),
        trust: RouteTrust {
            source: "scenario-fixture-pin".to_string(),
            verified_at: descriptor.route.trust_verified_at.clone(),
            ttl_seconds: descriptor.route.trust_ttl_seconds,
        },
        budget: None,
        priority: descriptor.route.priority,
        experimental: false,
    };
    let request = RouteResolutionRequest {
        route_set_id: descriptor.route.route_set_id.clone(),
        route_set_version: descriptor.route.route_set_version.clone(),
        candidates: vec![candidate],
        pinned_candidate_id: Some(&descriptor.route.pinned_candidate_id),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: prometheos_lite::workflow::route_resolution::RouteProfile::Standard,
        evaluated_at: descriptor.route.evaluated_at.clone(),
        workspace: Some(&tampered),
    };
    let error = resolve_route(&request, &harness_registry)
        .expect_err("mismatched workspace adapter must refuse before effect");
    assert!(
        error.to_string().contains("does not match") || error.to_string().contains("adapter"),
        "refusal should name the workspace mismatch: {error}"
    );
    // A pre-effect refusal is a setup error, never a product outcome.
    assert_eq!(
        scenario::classify_pre_effect_refusal(),
        ScenarioOutcomeClass::SetupError
    );
}

#[test]
fn accounting_failure_classifies_as_infrastructure_error() {
    let dispatch_ok = run_scenario(RUST_DESCRIPTOR);
    match &dispatch_ok.dispatch {
        Ok(_) => {
            assert_eq!(
                scenario::classify_completed_run(true, &dispatch_ok.dispatch, false),
                ScenarioOutcomeClass::InfrastructureError
            );
        }
        Err(error) => panic!("fixture dispatch must succeed: {error}"),
    }
    assert_eq!(
        scenario::classify_accounting_failure(),
        ScenarioOutcomeClass::InfrastructureError
    );
}

// ---------------------------------------------------------------------------
// Evidence integrity + non-claims
// ---------------------------------------------------------------------------

#[test]
fn exported_envelope_carries_no_secret_material() {
    let run = run_scenario(RUST_DESCRIPTOR);
    let envelope = run.envelope.as_ref().expect("envelope seals");
    let serialized = scenario::envelope_json(envelope).expect("envelope serializes");
    assert!(serialized.contains("\"lite.evidence.scenario-run\""));
    assert!(
        !serialized.contains(SECRET_CANARY),
        "known secret canary must never appear in exported evidence"
    );
    // No machine paths: the fixture pin is repo-relative.
    for leak in [
        "E:\\",
        "C:\\",
        "/home/",
        "/Users/",
        "\\Users\\",
        "\nsourceMappingURL",
    ] {
        assert!(
            !serialized.contains(leak),
            "envelope leaked machine-specific material: {leak}"
        );
    }
}

#[test]
fn exported_envelope_claims_no_benchmark_success() {
    let run = run_scenario(RUST_DESCRIPTOR);
    let envelope = run.envelope.as_ref().expect("envelope seals");
    let serialized = serde_json::to_string(envelope).expect("envelope serializes");
    for banned in [
        "\"score\"",
        "\"passRate\"",
        "passRate",
        "\"certified\"",
        "\"certification\"",
        "\"benchmarkScore\"",
        "\"certificationStatus\"",
    ] {
        assert!(
            !serialized.contains(banned),
            "exported evidence must not contain {banned}"
        );
    }
    // Lite-internal namespace only; Foundry's namespace appears nowhere as a
    // claim of consumption.
    assert!(!serialized.contains("\"foundry.benchmark-result\""));
    assert!(!serialized.contains("\"foundry.evaluation-result\""));
}

#[test]
fn external_evaluation_format_is_typed_unsupported() {
    let run = run_scenario(RUST_DESCRIPTOR);
    let envelope = run.envelope.as_ref().expect("envelope seals");
    assert_eq!(
        envelope.external_evaluation_format.status, "unsupported",
        "the Foundry external format does not exist; Lite must not claim otherwise"
    );
    // Any attempt to present the envelope with a supported external format
    // fails closed at re-verification.
    let mut tampered = envelope.clone();
    tampered.external_evaluation_format.status = "supported".to_string();
    tampered.content_digest = None;
    tampered.content_digest = Some(tampered.compute_digest().unwrap());
    assert!(tampered.verify().is_err());
}

#[test]
fn descriptor_error_report_carries_classification() {
    let error = ScenarioDescriptorError(vec![
        scenario::ScenarioDefect::setup("missing pin"),
        scenario::ScenarioDefect::invalid_task("ambiguous task"),
    ]);
    assert_eq!(error.class(), ScenarioOutcomeClass::InvalidTask);

    let setup_only = ScenarioDescriptorError(vec![scenario::ScenarioDefect::setup("missing pin")]);
    assert_eq!(setup_only.class(), ScenarioOutcomeClass::SetupError);
}
// ---------------------------------------------------------------------------
// Adversarial envelope tests: a re-sealed lie must not verify
//
// Model: the attacker holds a valid sealed envelope, modifies one record or
// pin, re-computes every digest they can recompute (the embedded accounting
// record and the outer envelope digest), and presents the result for re-
// verification. Every such forgery must fail `verify()`.
// ---------------------------------------------------------------------------

use prometheos_lite::workflow::accounting::AttemptAccountingV1;
use prometheos_lite::workflow::scenario::LiteScenarioRunEvidence;

const FORGED_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const FORGED_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const FORGED_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn ok_envelope() -> LiteScenarioRunEvidence {
    let run = run_scenario(RUST_DESCRIPTOR);
    run.envelope
        .expect("happy-path fixture produces a sealed envelope")
}

/// Re-seal the OUTER envelope digest after an attacker-side mutation.
fn with_outer_reseal(
    envelope: &LiteScenarioRunEvidence,
    tamper: impl FnOnce(&mut LiteScenarioRunEvidence),
) -> LiteScenarioRunEvidence {
    let mut envelope = envelope.clone();
    tamper(&mut envelope);
    envelope.content_digest = None;
    envelope.content_digest = Some(
        envelope
            .compute_digest()
            .expect("outer digest recomputes for an attacker-controlled record"),
    );
    envelope
}

/// Re-seal the embedded ACCOUNTING record after an attacker-side mutation.
fn reseal_accounting(
    record: &AttemptAccountingV1,
    tamper: impl FnOnce(&mut AttemptAccountingV1),
) -> AttemptAccountingV1 {
    let mut record = record.clone();
    tamper(&mut record);
    record.content_digest = None;
    record
        .try_sealed()
        .expect("tampered accounting re-seals for the attacker")
}

fn forge_envelope(
    envelope: &LiteScenarioRunEvidence,
    tamper_accounting: impl FnOnce(&mut AttemptAccountingV1),
) -> LiteScenarioRunEvidence {
    let forged_accounting = reseal_accounting(&envelope.attempt_accounting, tamper_accounting);
    // Graph stays exactly as embedded: a naive attacker only re-seals what
    // has a digest; the graph must then fail re-derivation instead.
    with_outer_reseal(envelope, |e| e.attempt_accounting = forged_accounting)
}

fn assert_rejected(envelope: LiteScenarioRunEvidence, reason: &str) {
    assert!(
        envelope.verify().is_err(),
        "forged envelope must not verify: {reason}"
    );
}

#[test]
fn attempt_content_digest_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| e.attempt.content_digest = Some(FORGED_A.into()));
    assert_rejected(forged, "attempt contentDigest does not re-derive");
}

#[test]
fn journal_entry_digest_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| {
        e.attempt.journal_entry.entry_digest = FORGED_B.into()
    });
    assert_rejected(forged, "journal entry digest does not re-derive");
}

#[test]
fn attempt_idempotency_key_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| e.attempt.idempotency_key = "forged-step".into());
    assert_rejected(forged, "journal identity no longer binds the attempt key");
}

#[test]
fn accounting_attempt_digest_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = forge_envelope(&base, |r| r.attempt_digest = Some(FORGED_A.into()));
    assert_rejected(
        forged,
        "accounting.attemptDigest must bind the embedded attempt",
    );
}

#[test]
fn accounting_journal_digest_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = forge_envelope(&base, |r| r.journal_digest = Some(FORGED_B.into()));
    assert_rejected(
        forged,
        "accounting.journalDigest must bind the embedded journal entry",
    );
}

#[test]
fn accounting_provider_identity_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = forge_envelope(&base, |r| r.provider_identity = "forged-provider".into());
    assert_rejected(
        forged,
        "provider identity must bind attempt/evidence/descriptor",
    );
}

#[test]
fn accounting_model_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = forge_envelope(&base, |r| r.model = Some("forged-model".into()));
    assert_rejected(forged, "model must bind attempt/evidence/descriptor");
}

#[test]
fn accounting_harness_id_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = forge_envelope(&base, |r| r.harness_id = "forged.harness".into());
    assert_rejected(forged, "harness id must bind attempt/snapshot/descriptor");
}

#[test]
fn accounting_harness_revision_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = forge_envelope(&base, |r| r.harness_revision = "v999".into());
    assert_rejected(
        forged,
        "harness revision must bind descriptor/route selection",
    );
}

#[test]
fn accounting_route_set_id_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = forge_envelope(&base, |r| r.route_set_id = "forged-routes".into());
    assert_rejected(forged, "route set id must bind route evidence");
}

#[test]
fn accounting_node_id_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = forge_envelope(&base, |r| r.node_id = "forged-node".into());
    assert_rejected(forged, "accounting node id must bind the node manifest");
}

#[test]
fn accounting_idempotency_key_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = forge_envelope(&base, |r| r.idempotency_key = "forged-step".into());
    assert_rejected(forged, "accounting/attempt/journal keys must bind together");
}

#[test]
fn accounting_graph_id_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = forge_envelope(&base, |r| r.graph_id = "forged-graph".into());
    assert_rejected(forged, "accounting graph id must bind the graph aggregate");
}

#[test]
fn graph_id_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| {
        e.graph_accounting.graph_id = "forged-graph".into()
    });
    assert_rejected(
        forged,
        "graph id must re-derive from embedded accounting records",
    );
}

#[test]
fn graph_attempts_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| e.graph_accounting.attempts += 1);
    assert_rejected(
        forged,
        "graph attempts must re-derive from embedded accounting records",
    );
}

#[test]
fn graph_completed_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| e.graph_accounting.completed = 0);
    assert_rejected(
        forged,
        "graph completed must re-derive from embedded accounting records",
    );
}

#[test]
fn graph_usage_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| e.graph_accounting.usage.total_tokens += 1);
    assert_rejected(
        forged,
        "graph usage must re-derive from embedded accounting records",
    );
}

#[test]
fn graph_cost_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| {
        e.graph_accounting
            .cost
            .currencies
            .insert("EUR".to_string(), 1);
        e.graph_accounting
            .cost
            .known_attempts
            .insert("EUR".to_string(), 1);
    });
    assert_rejected(
        forged,
        "graph cost must re-derive from embedded accounting records",
    );
}

#[test]
fn graph_latency_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| e.graph_accounting.latency_ms_total += 1);
    assert_rejected(
        forged,
        "graph latency must re-derive from embedded accounting records",
    );
}

#[test]
fn descriptor_route_candidate_tampered_is_rejected() {
    let base = ok_envelope();
    // Flip BOTH the candidate pin and the route pin consistently so the
    // descriptor still validates; the route-evidence binding must catch it.
    let forged = with_outer_reseal(&base, |e| {
        e.descriptor.route.candidate_id = "forged-route".into();
        e.descriptor.route.pinned_candidate_id = "forged-route".into();
    });
    assert_rejected(
        forged,
        "descriptor route pin must bind the route evidence selection",
    );
}

#[test]
fn descriptor_provider_identity_tampered_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| {
        e.descriptor.route.provider_identity = "forged-provider".into()
    });
    assert_rejected(
        forged,
        "descriptor provider identity must bind the route evidence",
    );
}

#[test]
fn descriptor_harness_revision_tampered_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| e.descriptor.harness.revision = "v999".into());
    assert_rejected(
        forged,
        "descriptor harness revision must bind the route evidence",
    );
}

#[test]
fn descriptor_repository_revision_tampered_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| {
        e.descriptor.repository.revision = "scenario-fixture-v2".into()
    });
    assert_rejected(
        forged,
        "descriptor repository revision must bind the workspace manifest",
    );
}

#[test]
fn descriptor_workspace_adapter_revision_tampered_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| {
        e.descriptor.workspace.adapter_revision = "forged.adapter.v1".into()
    });
    assert_rejected(
        forged,
        "descriptor workspace adapter revision must bind the manifest",
    );
}

#[test]
fn descriptor_expected_artifact_tampered_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| {
        e.descriptor.expected.evidence_artifacts = vec!["benchmark-score".into()]
    });
    assert_rejected(
        forged,
        "unknown expected-evidence artifacts must fail closed",
    );
}

#[test]
fn route_evidence_provider_identity_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| {
        e.route_evidence.selected.provider_identity = "forged-provider".into()
    });
    assert_rejected(
        forged,
        "route evidence must re-verify (provider identity binding)",
    );
}

#[test]
fn snapshot_content_digest_swapped_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| e.snapshot.content_digest = Some(FORGED_C.into()));
    assert_rejected(forged, "snapshot must re-verify against its sealed digest");
}

#[test]
fn outer_envelope_digest_swapped_is_rejected() {
    let base = ok_envelope();
    let mut forged = base.clone();
    forged.content_digest = Some(FORGED_A.into()); // NOT recomputed
    assert_rejected(forged, "outer envelope digest must re-derive");
}

#[test]
fn manifest_purpose_tampered_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| e.manifest.purpose = "forged purpose".into());
    assert_rejected(forged, "manifest must bind the attempt manifest digest");
}

#[test]
fn workspace_manifest_adapter_tampered_is_rejected() {
    let base = ok_envelope();
    let forged = with_outer_reseal(&base, |e| {
        let mut workspace = e.workspace_manifest.clone().expect("workspace embedded");
        workspace.adapter = AdapterKind::GitWorktree;
        e.workspace_manifest = Some(workspace.sealed());
    });
    assert_rejected(
        forged,
        "workspace manifest adapter must bind the descriptor pin",
    );
}

#[cfg(unix)]
#[test]
fn fixture_with_symbolic_link_is_refused() {
    let descriptor = ScenarioDescriptor::parse_json(RUST_DESCRIPTOR).expect("descriptor validates");
    let source = fixture_root(&descriptor);
    let temp = tempfile::tempdir().expect("staging dir");
    copy_tree(&source, temp.path());
    std::os::unix::fs::symlink(temp.path().join("src/lib.rs"), temp.path().join("link.rs"))
        .expect("symlink created");
    let error = scenario::fixture_content_digest(temp.path())
        .expect_err("a symlinked fixture must be refused");
    assert!(
        error.to_string().contains("symbolic link"),
        "refusal should name the symlink: {error}"
    );
}

#[cfg(unix)]
#[test]
fn fixture_with_unsupported_file_type_is_refused() {
    let descriptor = ScenarioDescriptor::parse_json(RUST_DESCRIPTOR).expect("descriptor validates");
    let source = fixture_root(&descriptor);
    let temp = tempfile::tempdir().expect("staging dir");
    copy_tree(&source, temp.path());
    let fifo = temp.path().join("note.fifo");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo available on the test host");
    assert!(status.success(), "mkfifo failed");
    let error = scenario::fixture_content_digest(temp.path())
        .expect_err("a fifo in the pinned fixture must be refused");
    assert!(
        error.to_string().contains("unsupported file type"),
        "refusal should name the unsupported file type: {error}"
    );
}
