//! Deterministic, operator-facing walkthrough of the governed
//! route-to-accounting path (issue #133, Slice D).
//!
//! Run:
//!
//! ```bash
//! cargo run --example governed_route_to_accounting
//! ```
//!
//! What it does, end to end, with NO production provider and NO network
//! beyond a loopback fixture:
//!
//! 1. spawns a deterministic loopback local-model fixture (127.0.0.1, one
//!    canned chat completion carrying token usage),
//! 2. registers the shipped `local-model` harness and its capability,
//! 3. resolves ONE route with the standard profile against fresh-attested
//!    trust metadata (injected evaluation instant),
//! 4. dispatches the governed effect exactly once (no retry, no fallback),
//! 5. re-verifies the attempt, its journal and the sealed outcome binding,
//! 6. accounts the outcome with an example operator price book,
//! 7. records it in an accounting ledger and aggregates the graph,
//! 8. prints an operator summary — identities, digests, evidence class
//!    classification, usage, cost — and refuses to let any known secret
//!    appear in any artifact it prints.
//!
//! Everything is deterministic: the evaluation instant, the trusted time
//! source and the fixture bytes are fixed. There is NO production provider
//! certification, no live claim, and zero effects outside loopback.
//!
//! The price book below is an EXAMPLE operator price book for the demo
//! fixture only; production pricing is operator data, not code.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result};
use prometheos_lite::workflow::accounting::{
    AccountingContext, AccountingLedger, CostAccounting, PricingBook, UsageAccounting,
    account_routed_outcome,
};
use prometheos_lite::workflow::execution_harness::HarnessRegistry;
use prometheos_lite::workflow::governance_compiler;
use prometheos_lite::workflow::governance_permit::GovernancePermit;
use prometheos_lite::workflow::local_model_harness::{
    LOCAL_MODEL_HARNESS_ID, LOCAL_MODEL_INFER_CAPABILITY, LocalModelConfig, LocalModelHarness,
    local_model_capability,
};
use prometheos_lite::workflow::node_contracts::NodeManifestV1;
use prometheos_lite::workflow::node_runner::{CapabilityRegistry, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::route_dispatch::{
    FixedTrustedTimeSource, RoutedDispatchOutcome, RoutedDispatchRequest, dispatch_resolved_route,
};
use prometheos_lite::workflow::route_resolution::{
    RouteCandidateV1, RouteProfile, RouteResolutionRequest, RouteTrust, resolve_route,
};
use prometheos_lite::workflow::soma::capability::Locality;
use prometheos_lite::workflow::soma::capability::{
    SUPPORTED_CAPABILITY_SCHEMA_VERSION, WorkRequirements,
};
use prometheos_lite::workflow::soma::contracts::AuthorityProfile;
use serde_json::json;

const GRAPH_ID: &str = "graph-demo";
const NODE_ID: &str = "demo-model-node";
const STEP_ID: &str = "demo-model-step-1";
const MODEL: &str = "tiny-local-model";

/// Well-formed but fictitious canary used to prove that the digest-sealed
/// artifacts and the error paths never carry raw secret material.
const DEMO_SECRET: &str = "PROMETHEOS_DEMO_EXAMPLE_SECRET_5f83c0ffee";

const PRICE_BOOK: &str = r#"{
    "schemaVersion": "1.0.0",
    "source": "example-operator-price-book",
    "version": "example-2026-11",
    "entries": [
        { "provider": "demo-local", "model": "tiny-local-model", "currency": "USD",
          "promptMinorPer1k": 1000, "completionMinorPer1k": 2000 }
    ]
}"#;

/// One-shot loopback HTTP server returning a fixed chat-completion body.
struct DemoModelServer {
    endpoint: String,
    receiver: mpsc::Receiver<()>,
}

fn spawn_model_server(response_body: String) -> DemoModelServer {
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
    DemoModelServer {
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
                    // Wait briefly for the full body to arrive.
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

fn permit_for(ids: &[&str], capabilities: &[&str]) -> GovernancePermit {
    let mut authority: Vec<String> = vec![
        "\"readable:repo://demo\"".to_string(),
        "\"writable:work://demo\"".to_string(),
    ];
    authority.extend(capabilities.iter().map(|c| format!("{c:?}")));
    let authority_json = authority.join(", ");
    let tools_json = capabilities
        .iter()
        .map(|c| format!("{c:?}: []"))
        .collect::<Vec<_>>()
        .join(", ");
    let units: Vec<String> = ids
        .iter()
        .map(|id| {
            format!(
                r#"{{ "schemaVersion": "1.1.0", "version": "1.1.0", "id": "{id}",
                    "executionClass": "deterministic",
                    "inputs": [{{"name": "goal", "type": "string", "acceptedOutcomes": ["Produced"]}}],
                    "outputs": [{{"name": "result", "type": "string", "emits": ["Produced"]}}],
                    "authority": [{authority_json}], "effects": [], "uses": [], "secrets": [], "context": [] }}"#
            )
        })
        .collect();
    let text = format!(
        r#"{{ "schemaVersion": "1.1.0", "version": "1.1.0", "id": "demo-governed",
            "name": "Governed", "kind": "atomic",
            "inputPorts": [{{"name": "goal", "direction": "input", "type": "string", "cardinality": "single", "requiredness": "required"}}],
            "outputPorts": [{{"name": "result", "direction": "output", "type": "string", "cardinality": "single", "requiredness": "required"}}],
            "body": [{}],
            "authority": {{ "executionClass": "deterministic", "mutation": "none",
                "tools": {{{tools_json}}},
                "readableScopes": ["repo://demo"],
                "writableScopes": ["work://demo"],
                "networkPolicy": {{"default": "deny"}},
                "providerPolicy": {{"allowlist": []}}, "secrets": [] }} }}"#,
        units.join(",")
    );
    let identity = governance_compiler::compile_workflow_text(&text)
        .expect("demo workflow compiles")
        .canonicalization
        .sha256;
    GovernancePermit::issue(&text, &identity).expect("permit issues for the reviewed identity")
}

#[tokio::main]
async fn main() -> Result<()> {
    // ---- Deterministic fixture: one canned chat completion with usage.
    let canned = json!({
        "id": "chatcmpl-demo-fixture",
        "object": "chat.completion",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "fixture response from loopback local model" },
            "finish_reason": "stop",
        }],
        "usage": { "prompt_tokens": 7, "completion_tokens": 4, "total_tokens": 11 },
    })
    .to_string();
    let server = spawn_model_server(canned);

    // ---- One operator-owned route candidate.
    let candidate = RouteCandidateV1 {
        schema_version: "1.0.0".to_string(),
        candidate_id: "demo-local-route".to_string(),
        provider_identity: "demo-local".to_string(),
        provider_locality: Locality::Local,
        model: Some(MODEL.to_string()),
        harness_id: LOCAL_MODEL_HARNESS_ID.to_string(),
        harness_revision: "v1".to_string(),
        workspace: None,
        trust: RouteTrust {
            source: "demo-operator-fixture".to_string(),
            verified_at: "2026-12-01T00:00:00Z".to_string(),
            ttl_seconds: 86_400,
        },
        budget: None,
        priority: 10,
        experimental: false,
    };

    // ---- Requirements/authority: deterministic, empty capabilities, no
    // degradation permitted. Facts only — never grants.
    let requirements: WorkRequirements = serde_json::from_value(json!({
        "id": "reqs-demo",
        "schemaVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": false,
        "forbidSubstitution": true,
        "requiredCapabilities": []
    }))
    .context("requirements parse")?;
    let authority: AuthorityProfile =
        serde_json::from_str(r#"{"executionClass": "deterministic", "mutation": "none"}"#)
            .context("authority parses")?;
    let request = RouteResolutionRequest {
        route_set_id: "routes-demo".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![candidate],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-12-01T12:00:00Z".to_string(),
        workspace: None,
    };

    // ---- Harness registry with the shipped local-model harness.
    let mut harness_registry = HarnessRegistry::new();
    harness_registry
        .register(Box::new(LocalModelHarness::new()))
        .context("local model harness registers")?;

    // ---- Capability registry with a governed local-model capability.
    let workspace = tempfile::tempdir().context("fixture workspace")?;
    let mut capability_registry = CapabilityRegistry::new();
    capability_registry.declare(
        LOCAL_MODEL_INFER_CAPABILITY,
        local_model_capability(
            LocalModelConfig::new(server.endpoint.clone(), MODEL).timeout_ms(5_000),
            workspace.path().to_path_buf(),
        ),
    );
    let permit = permit_for(&[NODE_ID], &[LOCAL_MODEL_INFER_CAPABILITY]);
    let mut runner = NodeRunner::new(capability_registry, permit);

    // ---- 1. Resolve (immutable snapshot + sealed evidence; no effects).
    let resolution = resolve_route(&request, &harness_registry).context("route resolves")?;
    resolution.evidence.verify().context("evidence verifies")?;
    resolution
        .snapshot
        .verify()
        .context("snapshot verifies (incl. SPEC 006 ExecutionProfile audit)")?;

    // ---- 2. Dispatch (exactly once; failure refuses without replay).
    let manifest = NodeManifestV1::parse_json(
        &json!({
            "schemaVersion": "1.0.0",
            "nodeId": NODE_ID,
            "purpose": "governed routing demo node",
            "inputs": [],
            "outputs": [{ "name": "out", "typeRef": "string" }],
            "readableScopes": ["repo://demo"],
            "writableScopes": ["work://demo"],
            "retry": { "maxAttempts": 1, "retryableClasses": [] }
        })
        .to_string(),
    )?;
    let restrictions = LocalRestrictions {
        readable_scopes: vec!["repo://demo".into()],
        writable_scopes: vec!["work://demo".into()],
        token_budget_ceiling: None,
        denied_providers: vec![],
        forbidden_paths: vec![],
        max_attempts: 1,
        escalation_target: "human-review".into(),
    };
    let outcome: RoutedDispatchOutcome = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &request,
        registry: &harness_registry,
        runner: &mut runner,
        manifest: &manifest,
        local_restrictions: &restrictions,
        // The canary never reaches the loopback fixture; the demo proves it
        // cannot appear in any emitted artifact either.
        known_secrets: vec![DEMO_SECRET.to_string()],
        clock: &FixedTrustedTimeSource::new("2026-12-01T12:00:01Z"),
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
    })
    .await
    .map_err(|error| anyhow::anyhow!("dispatch refused: {error}"))?;
    let _ = server.receiver.recv();

    // ---- 3. Re-verify the attempt and journal binding.
    outcome.attempt.verify().context("attempt verifies")?;
    let attempt = &outcome.attempt;

    // ---- 4. Account with the example price book.
    let pricing = PricingBook::from_json(PRICE_BOOK)
        .map_err(|defect| anyhow::anyhow!("price book parses: {}", defect.0))?;
    let context = AccountingContext {
        graph_id: GRAPH_ID.to_string(),
        node_id: NODE_ID.to_string(),
        currency: Some("USD".to_string()),
    };
    let record = account_routed_outcome(&outcome, &resolution, &context, Some(&pricing))
        .context("outcome accounts")?;
    record.verify().context("accounting record verifies")?;

    let mut ledger = AccountingLedger::new();
    ledger.record(record.clone()).context("record registers")?;
    let graph = ledger
        .aggregate_graph(GRAPH_ID)
        .context("graph aggregates")?;

    // ---- 5. Operator summary (opaque digests only; zero secrets).
    let cost_line = match &record.cost {
        CostAccounting::Known(computation) => format!(
            "known {} minor units {} (method={:?})",
            computation.minor_units, computation.currency, computation.method
        ),
        CostAccounting::Unknown { reason } => format!("unknown ({reason})"),
    };
    let usage_line = match &record.usage {
        UsageAccounting::Known(usage) => format!(
            "prompt={:?} completion={:?} total={:?}",
            usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
        ),
        UsageAccounting::Unknown { reason } => format!("unknown ({reason})"),
    };
    println!("== governed route -> dispatch -> accounting example ==");
    println!(
        "route_set.id               = {}",
        resolution.evidence.route_set_id
    );
    println!(
        "route_set.version          = {}",
        resolution.evidence.route_set_version
    );
    println!(
        "profile                    = {:?}",
        resolution.evidence.profile
    );
    println!(
        "selected.candidate_id      = {}",
        resolution.evidence.selected.candidate_id
    );
    println!(
        "selected.provider          = {}",
        resolution.evidence.selected.provider_identity
    );
    println!(
        "selected.model             = {:?}",
        resolution.evidence.selected.model
    );
    println!(
        "selected.harness_id        = {}",
        resolution.evidence.selected.harness_id
    );
    println!(
        "selected.harness_revision  = {}",
        resolution.evidence.selected.harness_revision
    );
    println!(
        "requirements_digest        = {}",
        resolution.evidence.requirements_digest
    );
    println!(
        "decision_digest            = {}",
        resolution.evidence.decision_digest
    );
    println!(
        "snapshot_digest            = {}",
        resolution.evidence.snapshot_digest
    );
    println!(
        "evidence_digest            = {}",
        resolution.evidence.content_digest.as_deref().unwrap_or("")
    );
    println!(
        "attempt_digest             = {}",
        attempt.content_digest.as_deref().unwrap_or("")
    );
    println!(
        "journal_entry_digest       = {}",
        attempt.journal_entry.entry_digest
    );
    println!("usage                      = {usage_line}");
    println!("cost                       = {cost_line}");
    println!("graph.attempts             = {}", graph.attempts);
    println!("graph.completed            = {}", graph.completed);
    println!(
        "graph.usage.knownAttempts  = {}",
        graph.usage.known_attempts
    );
    println!("graph.latencyMsTotal       = {}", graph.latency_ms_total);
    println!(
        "graph.cost.totalKnownExact = {}",
        graph.cost.total_known_exact
    );
    println!("graph.cost.currencies      = {:?}", graph.cost.currencies);

    let artifacts = [
        serde_json::to_string(&resolution.evidence)?,
        serde_json::to_string(&resolution.snapshot)?,
        serde_json::to_string(&attempt)?,
        serde_json::to_string(&record)?,
        serde_json::to_string(&graph)?,
    ];
    for artifact in artifacts {
        assert!(
            !artifact.contains(DEMO_SECRET),
            "demo secret leaked into a sealed artifact"
        );
    }
    println!("secret_canary_in_artifacts = none");
    Ok(())
}
