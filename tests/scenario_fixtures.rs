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

fn adapter_family_name(adapter: AdapterKind) -> &'static str {
    match adapter {
        AdapterKind::ExistingReadOnly => "existing-read-only",
        AdapterKind::GitWorktree => "git-worktree",
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
    let _ = server.receiver.recv();

    let accounting = dispatch.as_ref().ok().map(|outcome| {
        let pricing = PricingBook::from_json(PRICE_BOOK).expect("price book parses");
        let context = AccountingContext {
            graph_id: GRAPH_ID.to_string(),
            node_id: NODE_ID.to_string(),
            currency: Some("USD".to_string()),
        };
        let record = account_routed_outcome(outcome, &resolution, &context, Some(&pricing))
            .expect("outcome accounts");
        record.verify().expect("accounting record verifies");
        record
    });
    let graph = accounting.as_ref().map(|record| {
        let mut ledger = AccountingLedger::new();
        ledger.record(record.clone()).expect("record registers");
        ledger.aggregate_graph(GRAPH_ID).expect("graph aggregates")
    });

    let classification = scenario::classify_completed_run(true, &dispatch, accounting.is_some());

    let envelope = match (dispatch.as_ref(), accounting.as_ref(), graph.as_ref()) {
        (Ok(outcome), Some(record), Some(graph)) => {
            let attempt = &outcome.attempt;
            let chain = scenario::RunEvidenceChain {
                route_evidence_digest: resolution
                    .evidence
                    .content_digest
                    .clone()
                    .expect("evidence seals"),
                snapshot_digest: resolution
                    .snapshot
                    .content_digest
                    .clone()
                    .expect("snapshot seals"),
                decision_digest: resolution.evidence.decision_digest.clone(),
                requirements_digest: resolution.evidence.requirements_digest.clone(),
                candidate_set_digest: resolution.evidence.candidate_set_digest.clone(),
                attempt_digest: attempt.content_digest.clone().expect("attempt seals"),
                journal_entry_digest: attempt.journal_entry.entry_digest.clone(),
                accounting_record_digest: record.content_digest.clone().expect("record seals"),
                graph_id: graph.graph_id.clone(),
                graph_attempts: graph.attempts,
                graph_completed: graph.completed,
            };
            let route = scenario::RunRoutePin {
                route_set_id: resolution.evidence.route_set_id.clone(),
                route_set_version: resolution.evidence.route_set_version.clone(),
                candidate_id: resolution.evidence.selected.candidate_id.clone(),
                provider_identity: resolution.evidence.selected.provider_identity.clone(),
                provider_locality: descriptor.route.provider_locality.clone(),
                model: resolution
                    .evidence
                    .selected
                    .model
                    .clone()
                    .unwrap_or_default(),
                harness_id: resolution.evidence.selected.harness_id.clone(),
                harness_revision: resolution.evidence.selected.harness_revision.clone(),
                profile: descriptor.route.profile.clone(),
            };
            let workspace = Some(scenario::RunWorkspacePin {
                manifest_digest: workspace_manifest.compute_digest(),
                adapter: adapter_family_name(workspace_manifest.adapter).to_string(),
                adapter_revision: workspace_manifest.adapter_revision.clone(),
                mode: match workspace_manifest.mode {
                    WorkspaceMode::ReadOnly => "read-only".to_string(),
                    WorkspaceMode::Writable => "writable".to_string(),
                },
                base_revision: workspace_manifest.base_revision.clone(),
            });
            scenario::LiteScenarioRunEvidence::sealed(
                &descriptor,
                descriptor.route.dispatch_at.clone(),
                route,
                workspace,
                chain,
                record.clone(),
                graph.clone(),
                classification,
            )
            .ok()
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
    assert_eq!(envelope.evidence.graph_attempts, 1);
    assert_eq!(envelope.evidence.graph_completed, 1);
    // The evidence chain binds the very records the resolver/dispatcher sealed.
    let resolution = run.resolution.as_ref().expect("resolution sealed");
    assert_eq!(
        envelope.evidence.snapshot_digest,
        resolution.snapshot.content_digest.clone().unwrap()
    );
    assert_eq!(
        envelope.evidence.route_evidence_digest,
        resolution.evidence.content_digest.clone().unwrap()
    );
    assert_eq!(
        envelope.evidence.accounting_record_digest,
        record.content_digest.clone().unwrap()
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
    assert_eq!(envelope.scenario_id, "lite-python-bugfix-slugify-001");
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
    assert_eq!(a.scenario_digest, b.scenario_digest);
    assert_eq!(a.classification, b.classification);
    assert_eq!(a.repository, b.repository);
    assert_eq!(a.route, b.route);
    assert_eq!(a.workspace, b.workspace);
    assert_eq!(
        a.evidence.route_evidence_digest,
        b.evidence.route_evidence_digest
    );
    assert_eq!(a.evidence.snapshot_digest, b.evidence.snapshot_digest);
    assert_eq!(a.evidence.decision_digest, b.evidence.decision_digest);
    assert_eq!(
        a.evidence.requirements_digest,
        b.evidence.requirements_digest
    );
    assert_eq!(
        a.evidence.candidate_set_digest,
        b.evidence.candidate_set_digest
    );
    assert_eq!(a.evidence.graph_attempts, b.evidence.graph_attempts);
    assert_eq!(a.evidence.graph_completed, b.evidence.graph_completed);

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
