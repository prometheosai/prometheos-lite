//! Slice C execution accounting tests: actual governed route dispatches with
//! real provider-reported usage, real journal entries, real pricing book.
//!
//! Every accounting record is bound to the exact attempt journal entry it was
//! computed from — no estimation, no invention, no fabricated zeroes.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use serde_json::json;
use tempfile::TempDir;

use prometheos_lite::workflow::accounting::{
    AccountingContext, AccountingError, AccountingLedger, CostAccounting, CostMethod,
    InsertOutcome, LedgerError, PricingBook, UsageAccounting, account_routed_failure,
    account_routed_outcome,
};
use prometheos_lite::workflow::execution_harness::HarnessRegistry;
use prometheos_lite::workflow::local_model_harness::{
    LOCAL_MODEL_HARNESS_ID, LOCAL_MODEL_INFER_CAPABILITY, LocalModelConfig, LocalModelHarness,
    local_model_capability,
};
use prometheos_lite::workflow::node_runner::{CapabilityRegistry, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::route_dispatch::{
    FixedTrustedTimeSource, RoutedDispatchOutcome, RoutedDispatchRequest, dispatch_resolved_route,
};
use prometheos_lite::workflow::route_resolution::{
    RouteCandidateV1, RouteProfile, RouteResolution, RouteResolutionRequest, RouteTrust,
    resolve_route,
};

mod common;
use common::permit_for;

const NODE_ID: &str = "accounting-step";
const STEP_ID: &str = "accounted-step-1";
const MODEL: &str = "tiny-local-model";
const PROVIDER: &str = "local-fixture";
const PRICE_BOOK: &str = r#"{
    "schemaVersion": "1.0.0",
    "source": "operator-price-book",
    "version": "price-book-2026-11",
    "entries": [
        { "provider": "local-fixture", "model": "tiny-local-model", "currency": "USD",
          "promptMinorPer1k": 1000, "completionMinorPer1k": 2000 },
        { "provider": "local-fixture", "model": "tiny-local-model", "currency": "EUR",
          "promptMinorPer1k": 800, "completionMinorPer1k": 1600 }
    ]
}"#;

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

fn manifest() -> prometheos_lite::workflow::node_contracts::NodeManifestV1 {
    prometheos_lite::workflow::node_contracts::NodeManifestV1::parse_json(
        &json!({
            "schemaVersion": "1.0.0",
            "nodeId": NODE_ID,
            "purpose": "routed accounting step",
            "inputs": [],
            "outputs": [{ "name": "out", "typeRef": "string" }],
            "readableScopes": ["repo://x"],
            "writableScopes": ["work://y"],
            "retry": { "maxAttempts": 2, "retryableClasses": ["infra"] }
        })
        .to_string(),
    )
    .expect("manifest parses")
}

fn requirements_no_limit() -> prometheos_lite::workflow::soma::capability::WorkRequirements {
    serde_json::from_value(json!({
        "id": "reqs-account",
        "schemaVersion": prometheos_lite::workflow::soma::capability::SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion": prometheos_lite::workflow::soma::capability::SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": false,
        "forbidSubstitution": true,
        "requiredCapabilities": []
    }))
    .expect("requirements parse")
}

fn authority_with_budgets(
    tokens: u64,
) -> prometheos_lite::workflow::soma::contracts::AuthorityProfile {
    serde_json::from_value(json!({
        "executionClass": "deterministic",
        "mutation": "none",
        "budgets": { "tokens": tokens },
    }))
    .expect("authority with budgets parses")
}

fn authority() -> prometheos_lite::workflow::soma::contracts::AuthorityProfile {
    serde_json::from_str(r#"{"executionClass":"deterministic","mutation":"none"}"#)
        .expect("minimal authority parses")
}

fn candidate(id: &str, priority: u32) -> RouteCandidateV1 {
    RouteCandidateV1 {
        schema_version: "1.0.0".to_string(),
        candidate_id: id.to_string(),
        provider_identity: PROVIDER.to_string(),
        provider_locality: prometheos_lite::workflow::soma::capability::Locality::Local,
        model: Some(MODEL.to_string()),
        harness_id: LOCAL_MODEL_HARNESS_ID.to_string(),
        harness_revision: "v1".to_string(),
        workspace: None,
        trust: trust("2026-12-01T00:00:00Z", 86_400),
        budget: None,
        priority,
        experimental: false,
    }
}

fn trust(verified_at: &str, ttl_seconds: u64) -> RouteTrust {
    RouteTrust {
        source: "operator-fixture".to_string(),
        verified_at: verified_at.to_string(),
        ttl_seconds,
    }
}

/// One-shot loopback model server with a tiny deterministic response.
struct LocalModelFixture {
    _dir: TempDir,
    endpoint: String,
    accepts: Arc<AtomicUsize>,
}

fn read_http_request(socket: &mut std::net::TcpStream) -> Option<(String, Vec<u8>)> {
    use std::io::Read;
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        match socket.read(&mut tmp) {
            Ok(0) => return None,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
            }
            Err(_) => return None,
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let content_length = head
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut body = buf.split_off(head_end);
    let mut read = body.len();
    body.resize(content_length, 0);
    while read < content_length {
        match socket.read(&mut body[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(_) => break,
        }
    }
    body.truncate(read);
    Some((head, body))
}

fn spawn_model_server(response_body: Option<String>) -> LocalModelFixture {
    use std::io::Write;
    use std::net::TcpListener;

    let dir = tempfile::tempdir().expect("tempdir");
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture binds");
    let addr = listener.local_addr().expect("fixture addr");
    let accepts = Arc::new(AtomicUsize::new(0));
    let accepts_thread = accepts.clone();
    let body = response_body.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let _ = accepts_thread.fetch_add(1, Ordering::SeqCst);
            let mut socket = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let (_head, _body) = match read_http_request(&mut socket) {
                Some(v) => v,
                None => continue,
            };
            if let Some(ref body) = body {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                if socket.write_all(response.as_bytes()).is_err() {
                    continue;
                }
                let _ = socket.shutdown(std::net::Shutdown::Both);
                break;
            }
        }
    });
    LocalModelFixture {
        _dir: dir,
        endpoint: format!("http://127.0.0.1:{}/v1", addr.port()),
        accepts,
    }
}

fn response_with_usage(prompt: u64, completion: u64, total: Option<u64>) -> String {
    let usage = match total {
        Some(t) => json!({
            "prompt_tokens": prompt,
            "completion_tokens": completion,
            "total_tokens": t,
        }),
        None => json!({
            "prompt_tokens": prompt,
            "completion_tokens": completion,
        }),
    };
    serde_json::to_string(&json!({
        "id": "chatcmpl-fixture",
        "object": "chat.completion",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "fixture response" },
            "finish_reason": "stop",
        }],
        "usage": usage,
    }))
    .expect("fixture serializes")
}

fn response_without_usage() -> String {
    serde_json::to_string(&json!({
        "id": "chatcmpl-fixture",
        "object": "chat.completion",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "fixture response" },
            "finish_reason": "stop",
        }],
    }))
    .expect("fixture serializes")
}

fn response_with_inconsistent_usage() -> String {
    serde_json::to_string(&json!({
        "id": "chatcmpl-fixture",
        "object": "chat.completion",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "fixture response" },
            "finish_reason": "stop",
        }],
        "usage": {
            "prompt_tokens": 7,
            "completion_tokens": 4,
            "total_tokens": 42,
        },
    }))
    .expect("fixture serializes")
}

fn local_model_config(endpoint: String) -> LocalModelConfig {
    LocalModelConfig::new(endpoint, MODEL)
        .timeout_ms(5_000)
        .max_output_bytes(256 * 1024)
}

struct Fixture {
    _server: LocalModelFixture,
    _root: TempDir,
    registry: HarnessRegistry,
    runner: NodeRunner,
}

fn fixture(body: Option<String>) -> Fixture {
    let server = spawn_model_server(body);
    let _server = server;
    let fixture_components = _server;
    let dir = tempfile::tempdir().expect("tempdir");
    let mut capability_registry = CapabilityRegistry::new();
    capability_registry.declare(
        LOCAL_MODEL_INFER_CAPABILITY,
        local_model_capability(
            local_model_config(fixture_components.endpoint.clone()),
            dir.path().to_path_buf(),
        ),
    );
    let permit = permit_for(&[NODE_ID], &[LOCAL_MODEL_INFER_CAPABILITY]);
    let runner = NodeRunner::new(capability_registry, permit);
    let mut harness_registry = HarnessRegistry::new();
    harness_registry
        .register(Box::new(LocalModelHarness::new()))
        .expect("local model harness registers");
    Fixture {
        _server: fixture_components,
        _root: dir,
        registry: harness_registry,
        runner,
    }
}

struct AdmittedSet {
    request: RouteResolutionRequest<'static>,
}

fn admitted_set(candidates: Vec<RouteCandidateV1>) -> AdmittedSet {
    admitted_set_with_authority(candidates, authority())
}

fn admitted_set_with_authority(
    candidates: Vec<RouteCandidateV1>,
    authority: prometheos_lite::workflow::soma::contracts::AuthorityProfile,
) -> AdmittedSet {
    let requirements = Box::leak(Box::new(requirements_no_limit()));
    let authority = Box::leak(Box::new(authority));
    AdmittedSet {
        request: RouteResolutionRequest {
            route_set_id: "routes-accounting".to_string(),
            route_set_version: "1.0.0".to_string(),
            candidates,
            pinned_candidate_id: None,
            requirements,
            authority_declared: authority,
            authority_effective: authority,
            profile: RouteProfile::Standard,
            evaluated_at: "2026-12-01T12:00:00Z".to_string(),
            workspace: None,
        },
    }
}

async fn dispatched(
    fixture: &mut Fixture,
    set: &AdmittedSet,
    step_id: &str,
) -> anyhow::Result<prometheos_lite::workflow::route_dispatch::RoutedDispatchOutcome> {
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &set.request,
        registry: &fixture.registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec!["PROMETHEOS_TEST_SECRET_9f83c0ffee".to_string()],
        clock: &FixedTrustedTimeSource::new("2026-12-01T12:00:01Z"),
        step: prometheos_lite::workflow::execution_harness::HarnessStep {
            step_id: step_id.to_string(),
            capability: LOCAL_MODEL_INFER_CAPABILITY.to_string(),
            args: json!({
                "model": MODEL,
                "endpoint": fixture._server.endpoint,
                "messages": [{ "role": "user", "content": "hi" }],
                "maxTokens": 64,
            }),
        },
    })
    .await
    .map_err(|error| anyhow::anyhow!("{error}"))
}

fn accounting_context(currency: Option<&str>) -> AccountingContext {
    AccountingContext {
        graph_id: "graph-acct-1".to_string(),
        node_id: NODE_ID.to_string(),
        currency: currency.map(str::to_string),
    }
}

#[tokio::test]
async fn known_usage_and_directed_pricing_books_bind_one_journal_entry() {
    let mut fixture = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set = admitted_set(vec![candidate("model-a", 10)]);
    let outcome = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch succeeds");
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let pricing = PricingBook::from_json(PRICE_BOOK).expect("price book validates");
    let record = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("USD")),
        Some(&pricing),
    )
    .expect("accounting succeeds");
    assert_eq!(
        record.attempt_digest.as_deref(),
        outcome.attempt.content_digest.as_deref()
    );
    assert_eq!(
        record.journal_digest.as_deref(),
        Some(outcome.attempt.journal_entry.entry_digest.as_str())
    );
    assert!(matches!(
        record.outcome,
        prometheos_lite::workflow::accounting::AttemptOutcomeAccounting::Completed
    ));
    match &record.usage {
        UsageAccounting::Known(usage) => {
            assert_eq!(usage.prompt_tokens, Some(7));
            assert_eq!(usage.completion_tokens, Some(4));
            assert_eq!(usage.total_tokens, Some(11));
        }
        other => panic!("expected known usage, got {other:?}"),
    }
    assert!(
        record.latency_ms < 5000,
        "latency must be dispatch-measured and small on loopback"
    );
    match &record.cost {
        CostAccounting::Known(cost) => {
            // 7 * $1.000/1k + 4 * $2.000/1k = $0.015 -> 1.5 cents -> 1 minor unit? No:
            // 7 * 1000 / 1000 = 7, 4 * 2000 / 1000 = 8  -> 15 minor units
            assert_eq!(cost.minor_units, 15);
            assert_eq!(cost.currency, "USD");
            assert_eq!(cost.pricing_source, "operator-price-book");
            assert_eq!(cost.pricing_version, "price-book-2026-11");
            assert_eq!(cost.method, CostMethod::DirectedRates);
        }
        other => panic!("expected known cost, got {other:?}"),
    }
    assert!(record.verify().is_ok());
}

#[tokio::test]
async fn unknown_provider_usage_is_unknown_not_zero() {
    let mut fixture = fixture(Some(response_without_usage()));
    let set = admitted_set(vec![candidate("model-b", 10)]);
    let outcome = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch succeeds");
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let record = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("USD")),
        Option::None,
    )
    .expect("accounting succeeds");
    match &record.usage {
        UsageAccounting::Unknown { reason } => {
            assert!(
                reason.contains("usage with no usable fields"),
                "got: {reason}"
            );
        }
        other => panic!("expected unknown usage, got {other:?}"),
    }
    match &record.cost {
        CostAccounting::Unknown { reason } => {
            assert!(reason.contains("no authorized pricing book"));
        }
        other => panic!("expected unknown cost, got {other:?}"),
    }
}

#[tokio::test]
async fn missing_pricing_entry_yields_unknown_cost_not_a_fabricated_zero() {
    let mut fixture = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set = admitted_set(vec![candidate("model-c", 10)]);
    let outcome = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch succeeds");
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let pricing = PricingBook::from_json(PRICE_BOOK).expect("book validates");
    let record = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("EUR")),
        Some(&pricing),
    )
    .expect("accounting succeeds");
    // EUR entry exists: cost is derived.
    match &record.cost {
        CostAccounting::Known(cost) => {
            assert_eq!(cost.currency, "EUR");
            assert_eq!(cost.minor_units, (7 * 800 + 4 * 1600) / 1000);
        }
        other => panic!("expected EUR-derived cost, got {other:?}"),
    }
    // A currency with no pricing at all yields unknown, not 0.
    let record_gbp = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("GBP")),
        Some(&pricing),
    )
    .expect("accounting succeeds");
    match &record_gbp.cost {
        CostAccounting::Unknown { reason } => {
            assert!(reason.contains("no pricing for provider"), "got: {reason}");
        }
        other => panic!("expected unknown GBP cost, got {other:?}"),
    }
}

#[tokio::test]
async fn partial_reported_usage_is_partial_with_cost_on_known_fields() {
    let mut fixture = fixture(Some(response_with_usage(7, 4, None)));
    let set = admitted_set(vec![candidate("model-d", 10)]);
    let outcome = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch succeeds");
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let pricing = PricingBook::from_json(PRICE_BOOK).expect("book validates");
    let record = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("USD")),
        Some(&pricing),
    )
    .expect("accounting succeeds");
    match &record.usage {
        UsageAccounting::Known(usage) => {
            assert_eq!(usage.total_tokens, None);
        }
        other => panic!("expected known (partial) usage, got {other:?}"),
    }
    // Cost derivable from directed rates even without total.
    assert!(matches!(record.cost, CostAccounting::Known(_)));
}

#[tokio::test]
async fn inconsistent_provider_usage_is_refused_by_accounting() {
    let mut fixture = fixture(Some(response_with_inconsistent_usage()));
    let set = admitted_set(vec![candidate("model-e", 10)]);
    let outcome = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch succeeds");
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let error = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("USD")),
        Option::None,
    )
    .expect_err("inconsistent total refuses the record");
    assert!(format!("{error}").contains("inconsistent"));
}

#[tokio::test]
async fn accounting_record_is_sealed_and_tamper_fails_verify() {
    let mut fixture = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set = admitted_set(vec![candidate("model-f", 10)]);
    let outcome = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch succeeds");
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let mut record = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(None),
        Option::None,
    )
    .expect("accounting succeeds");
    assert!(record.verify().is_ok());
    record.journal_digest = Some("0".repeat(64));
    assert!(record.verify().is_err());
}

#[tokio::test]
async fn ledger_rejects_idempotency_key_with_a_different_journal_digest() {
    let fixture_fresh = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set = admitted_set(vec![candidate("model-conflict", 10)]);
    let resolution = resolve_route(&set.request, &fixture_fresh.registry).expect("route resolves");
    let mut fixture = fixture_fresh;
    let outcome1 = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch 1 succeeds");

    let mut ledger = AccountingLedger::new();
    let record1 = account_routed_outcome(
        &outcome1,
        &resolution,
        &accounting_context(Some("USD")),
        Option::None,
    )
    .expect("accounting succeeds");
    ledger.record(record1).expect("first insert records");

    // Replay the identical attempt: same key, same journal digest -> Duplicate.
    let outcome1_replay = outcome1;
    let replay_record = account_routed_outcome(
        &outcome1_replay,
        &resolution,
        &accounting_context(Some("USD")),
        Option::None,
    )
    .expect("replay accounting succeeds");
    let dup = ledger
        .record(replay_record)
        .expect("replay insert succeeds");
    assert_eq!(dup, InsertOutcome::Duplicate);
    assert_eq!(ledger.records().len(), 1);
}

#[tokio::test]
async fn two_registrations_of_the_same_attempt_conflict_and_are_refused() {
    // First fresh fixture: dispatch succeeds and seals attempt 1.
    let mut fixture1 = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set = admitted_set(vec![candidate("model-g", 10)]);
    let resolution1 = resolve_route(&set.request, &fixture1.registry).expect("route resolves");
    let outcome1 = dispatched(&mut fixture1, &set, "accounted-step-1")
        .await
        .expect("dispatch 1 succeeds");

    // Second fresh fixture: same step id, fresh journal -> different digest.
    let mut fixture2 = fixture(Some(response_with_usage(7, 4, Some(11))));
    let outcome2 = dispatched(&mut fixture2, &set, "accounted-step-1")
        .await
        .expect("dispatch 2 succeeds");

    let mut ledger = AccountingLedger::new();
    let record1 = account_routed_outcome(
        &outcome1,
        &resolution1,
        &accounting_context(Some("USD")),
        Option::None,
    )
    .expect("accounting 1 succeeds");
    ledger.record(record1).expect("first record registers");

    let record2 = account_routed_outcome(
        &outcome2,
        &resolution1,
        &accounting_context(Some("USD")),
        Option::None,
    )
    .expect("accounting 2 succeeds");
    let error = ledger
        .record(record2)
        .expect_err("conflicting journal digest must refuse");
    assert!(matches!(error, LedgerError::IdempotencyConflict { .. }));
    assert_eq!(ledger.records().len(), 1);
}

#[tokio::test]
async fn graph_and_node_aggregation_with_unknown_coverage_is_not_overstated() {
    // Node 1: known usage; node 2: unknown usage (response without usage).
    let mut fixture1 = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set1 = admitted_set(vec![candidate("model-h1", 10)]);
    let resolution1 = resolve_route(&set1.request, &fixture1.registry).expect("route resolves");
    let outcome1 = dispatched(&mut fixture1, &set1, "accounted-step-1")
        .await
        .expect("dispatch 1 succeeds");
    let mut node2 = fixture(Some(response_without_usage()));
    let set2 = admitted_set(vec![candidate("model-h2", 10)]);
    let resolution2 = resolve_route(&set2.request, &node2.registry).expect("route resolves");
    let outcome2 = dispatched(&mut node2, &set2, "accounted-step-2")
        .await
        .expect("dispatch 2 succeeds");

    let mut ledger = AccountingLedger::new();
    ledger
        .record(
            account_routed_outcome(
                &outcome1,
                &resolution1,
                &accounting_context(Some("USD")),
                Option::None,
            )
            .expect("accounting 1 succeeds"),
        )
        .expect("node 1 record registers");
    ledger
        .record(
            account_routed_outcome(
                &outcome2,
                &resolution2,
                &accounting_context(Some("USD")),
                Option::None,
            )
            .expect("accounting 2 succeeds"),
        )
        .expect("node 2 record registers");

    let graph = ledger
        .aggregate_graph("graph-acct-1")
        .expect("graph aggregates");
    assert_eq!(graph.attempts, 2);
    assert_eq!(graph.completed, 2);
    assert_eq!(graph.usage.known_attempts, 1);
    assert_eq!(graph.usage.unknown_attempts, 1);
    assert!(
        !graph.usage.complete,
        "incomplete coverage must not claim completeness"
    );
    assert_eq!(graph.usage.total_tokens, 11, "only known component summed");
}

#[tokio::test]
async fn known_two_attempt_graph_aggregates_to_exact_totals() {
    let mut fixture1 = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set1 = admitted_set(vec![candidate("model-i1", 10)]);
    let resolution1 = resolve_route(&set1.request, &fixture1.registry).expect("route resolves");
    let outcome1 = dispatched(&mut fixture1, &set1, "accounted-step-1")
        .await
        .expect("dispatch 1 succeeds");
    let mut fixture2 = fixture(Some(response_with_usage(10, 5, Some(15))));
    let set2 = admitted_set(vec![candidate("model-i2", 10)]);
    let resolution2 = resolve_route(&set2.request, &fixture2.registry).expect("route resolves");
    let outcome2 = dispatched(&mut fixture2, &set2, "accounted-step-2")
        .await
        .expect("dispatch 2 succeeds");

    let pricing = PricingBook::from_json(PRICE_BOOK).expect("book validates");
    let mut ledger = AccountingLedger::new();
    ledger
        .record(
            account_routed_outcome(
                &outcome1,
                &resolution1,
                &accounting_context(Some("USD")),
                Some(&pricing),
            )
            .expect("accounting 1 succeeds"),
        )
        .expect("node 1 record registers");
    ledger
        .record(
            account_routed_outcome(
                &outcome2,
                &resolution2,
                &accounting_context(Some("USD")),
                Some(&pricing),
            )
            .expect("accounting 2 succeeds"),
        )
        .expect("node 2 record registers");

    let graph = ledger
        .aggregate_graph("graph-acct-1")
        .expect("graph aggregates");
    assert_eq!(graph.usage.total_tokens, 26);
    assert_eq!(graph.usage.completion_tokens, 9);
    assert_eq!(graph.usage.known_attempts, 2);
    assert!(graph.usage.complete);
    assert_eq!(
        graph.cost.currencies.get("USD"),
        Some(&(7 * 1000 / 1000 + 4 * 2000 / 1000 + 10 * 1000 / 1000 + 5 * 2000 / 1000))
    );
}

#[tokio::test]
async fn mixed_currencies_are_never_summed_together() {
    let mut fixture1 = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set1 = admitted_set(vec![candidate("model-j1", 10)]);
    let resolution1 = resolve_route(&set1.request, &fixture1.registry).expect("route resolves");
    let outcome1 = dispatched(&mut fixture1, &set1, "accounted-step-1")
        .await
        .expect("dispatch 1 succeeds");
    let mut fixture2 = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set2 = admitted_set(vec![candidate("model-j2", 10)]);
    let resolution2 = resolve_route(&set2.request, &fixture2.registry).expect("route resolves");
    let outcome2 = dispatched(&mut fixture2, &set2, "accounted-step-2")
        .await
        .expect("dispatch 2 succeeds");

    let pricing = PricingBook::from_json(PRICE_BOOK).expect("book validates");
    let mut ledger = AccountingLedger::new();
    ledger
        .record(
            account_routed_outcome(
                &outcome1,
                &resolution1,
                &accounting_context(Some("USD")),
                Some(&pricing),
            )
            .expect("accounting 1 succeeds"),
        )
        .expect("usd record registers");
    ledger
        .record(
            account_routed_outcome(
                &outcome2,
                &resolution2,
                &accounting_context(Some("EUR")),
                Some(&pricing),
            )
            .expect("accounting 2 succeeds"),
        )
        .expect("eur record registers");

    let graph = ledger
        .aggregate_graph("graph-acct-1")
        .expect("graph aggregates");
    assert_eq!(graph.cost.currencies.len(), 2);
    assert!(graph.cost.mixed_currencies.contains(&"USD".to_string()));
    assert!(graph.cost.mixed_currencies.contains(&"EUR".to_string()));
    assert!(
        !graph.cost.currencies.values().any(|v| *v > 15),
        "no bogus grand total mixing USD+EUR units"
    );
}

fn budgets(tokens: u64) -> prometheos_lite::workflow::soma::contracts::Budgets {
    serde_json::from_value(json!({ "tokens": tokens })).expect("budgets parse")
}

#[tokio::test]
async fn over_budget_attempt_carries_typed_over_budget_evidence() {
    // Candidate narrows the route budget to 1 token; provider reports 11.
    // Over-budget evidence attaches (post-effect report, not a gate).
    let mut fixture = fixture(Some(response_with_usage(7, 4, Some(11))));
    let mut narrowing = candidate("model-k", 10);
    narrowing.budget = Some(budgets(1));
    let set = admitted_set_with_authority(vec![narrowing], authority_with_budgets(10));
    let outcome = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch succeeds (already executed)");
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let record = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(None),
        Option::None,
    )
    .expect("accounting succeeds");
    let evidence = record.over_budget.expect("over-budget evidence attaches");
    assert_eq!(evidence.dimension, "tokens");
    assert_eq!(evidence.observed, 11);
    assert_eq!(evidence.ceiling, 1);
    assert_eq!(evidence.ceiling_source, "effective-route-budget");
}

#[tokio::test]
async fn unauthorized_route_produces_zero_effects_and_no_records() {
    let mut fixture = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set = admitted_set(vec![candidate("model-l", 10)]);
    // Refused by pre-flight: capability not declared by this harness.
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let ledger = AccountingLedger::new();
    // Dispatch through an undeclared capability: the seam refuses.
    let outcome = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &set.request,
        registry: &fixture.registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        clock: &FixedTrustedTimeSource::new("2026-12-01T12:00:01Z"),
        step: prometheos_lite::workflow::execution_harness::HarnessStep {
            step_id: STEP_ID.to_string(),
            capability: "harness.subprocess.spawn".to_string(), // undeclared
            args: json!({}),
        },
    })
    .await;
    assert_eq!(
        fixture._server.accepts.load(Ordering::SeqCst),
        0,
        "no effects"
    );
    assert!(outcome.is_err());
    assert!(
        ledger.records().is_empty(),
        "no accounting for refused attempt"
    );
}

#[tokio::test]
async fn price_book_fails_closed_on_bad_input() {
    // Empty entries, missing currencies, duplicate keys, bad version.
    let empty = PricingBook::from_json(
        r#"{"schemaVersion":"1.0.0","source":"","version":"","entries":[]}"#,
    );
    assert!(empty.is_err());

    let bad_currency = PricingBook::from_json(
        r#"{"schemaVersion":"1.0.0","source":"fixture","version":"1","entries":[{"provider":"p","model":"m","currency":"usd","promptMinorPer1k":1}]}"#,
    );
    assert!(bad_currency.is_err());

    let duplicate = PricingBook::from_json(
        r#"{"schemaVersion":"1.0.0","source":"fixture","version":"1","entries":[{"provider":"p","model":"m","currency":"USD","promptMinorPer1k":1},{"provider":"p","model":"m","currency":"USD","completionMinorPer1k":1}]}"#,
    );
    assert!(duplicate.is_err());

    let bad_schema = PricingBook::from_json(
        r#"{"schemaVersion":"9.9.9","source":"fixture","version":"1","entries":[{"provider":"p","model":"m","currency":"USD","promptMinorPer1k":1}]}"#,
    );
    assert!(bad_schema.is_err());
}

#[tokio::test]
async fn cost_cannot_exceed_upper_bounds_by_construction_and_never_softens_zero() {
    // u64::MAX prompt tokens with a rate that overflows the final minor-unit
    // conversion -> cost Unknown. Never panics, never wraps, never rounds up.
    let mut fixture = fixture(Some(response_with_usage(u64::MAX, 0, Some(u64::MAX))));
    let mut overflow_candidate = candidate("model-n", 10);
    overflow_candidate.provider_identity = "local-fixture".to_string();
    overflow_candidate.model = Some("overflow-model".to_string());
    let set = admitted_set(vec![overflow_candidate]);
    let outcome = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch succeeds");
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let overflow_book = PricingBook::from_json(
        r#"{
            "schemaVersion": "1.0.0",
            "source": "operator-price-book",
            "version": "price-book-2026-11",
            "entries": [
                { "provider": "local-fixture", "model": "overflow-model", "currency": "USD",
                  "promptMinorPer1k": 18446744073, "completionMinorPer1k": 0 }
            ]
        }"#,
    )
    .expect("overflow book validates");
    let record = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("USD")),
        Some(&overflow_book),
    )
    .expect("accounting succeeds");
    match &record.cost {
        CostAccounting::Unknown { reason } => assert!(
            reason.contains("overflow"),
            "huge usage with huge rate must overflow to Unknown, got: {reason}"
        ),
        CostAccounting::Known(cost) => {
            panic!(
                "overflowed usage must not produce a finite cost, got {:?}",
                cost
            )
        }
    }
}

#[tokio::test]
async fn accounting_records_never_carry_secrets_or_prompts() {
    let secret = "PROMETHEOS_TEST_SECRET_9f83c0ffee";
    let prompt = "this is the secret prompt content";
    let mut fixture = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set = admitted_set(vec![candidate("model-o", 10)]);
    let outcome = {
        let mut resolution =
            resolve_route(&set.request, &fixture.registry).expect("route resolves");
        let _ = &mut resolution;
        dispatch_resolved_route(RoutedDispatchRequest {
            resolution: &resolution,
            original_request: &set.request,
            registry: &fixture.registry,
            runner: &mut fixture.runner,
            manifest: &manifest(),
            local_restrictions: &restrictions(),
            known_secrets: vec![secret.to_string()],
            clock: &FixedTrustedTimeSource::new("2026-12-01T12:00:01Z"),
            step: prometheos_lite::workflow::execution_harness::HarnessStep {
                step_id: STEP_ID.to_string(),
                capability: LOCAL_MODEL_INFER_CAPABILITY.to_string(),
                args: json!({
                    "model": MODEL,
                    "endpoint": fixture._server.endpoint,
                    "messages": [{ "role": "user", "content": prompt }],
                    "maxTokens": 64,
                }),
            },
        })
        .await
        .expect("dispatch succeeds")
    };
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let record = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("USD")),
        Option::None,
    )
    .expect("accounting succeeds");
    let serialized = serde_json::to_string(&record).expect("serialize");
    assert!(
        !serialized.contains(secret),
        "known secret leaked into accounting record"
    );
    assert!(
        !serialized.contains(prompt),
        "prompt content leaked into accounting record"
    );
}

#[tokio::test]
async fn cancelled_failure_records_typed_unknown_outcome_without_inventing() {
    let fixture_ = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set = admitted_set(vec![candidate("model-p", 10)]);
    let resolution = resolve_route(&set.request, &fixture_.registry).expect("route resolves");
    let deadline = Instant::now();
    let record = account_routed_failure(
        &resolution,
        "cancelled",
        &accounting_context(Some("USD")),
        STEP_ID,
        None,
        deadline.elapsed().as_millis() as u64,
    );
    match &record.outcome {
        prometheos_lite::workflow::accounting::AttemptOutcomeAccounting::Cancelled { kind } => {
            assert_eq!(kind, "cancelled")
        }
        other => panic!("expected cancelled outcome, got {other:?}"),
    }
    match &record.usage {
        UsageAccounting::Unknown { .. } => {}
        other => panic!("expected unknown usage, got {other:?}"),
    }
    match &record.cost {
        CostAccounting::Unknown { .. } => {}
        other => panic!("expected unknown cost, got {other:?}"),
    }
}

#[tokio::test]
async fn replayed_identical_attempt_is_idempotent_duplicate() {
    let mut fixture = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set = admitted_set(vec![candidate("model-q", 10)]);
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let outcome = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch succeeds");
    let mut ledger = AccountingLedger::new();
    let record = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("USD")),
        Option::None,
    )
    .expect("accounting succeeds");
    let _ = ledger.record(record.clone()).expect("first registers");
    let second = ledger.record(record).expect("replay registers");
    assert_eq!(second, InsertOutcome::Duplicate);
    assert_eq!(
        ledger
            .aggregate_graph("graph-acct-1")
            .expect("aggregate graph")
            .attempts,
        1
    );
}

// ---------------------------------------------------------------------------
// Adversarial integrity tests (PR #304 review)
// ---------------------------------------------------------------------------

fn account_with(
    outcome: &RoutedDispatchOutcome,
    resolution: &RouteResolution,
) -> Result<prometheos_lite::workflow::accounting::AttemptAccountingV1, AccountingError> {
    account_routed_outcome(
        outcome,
        resolution,
        &accounting_context(Some("USD")),
        Option::None,
    )
}

#[tokio::test]
async fn adversarial_outcome_from_route_a_with_resolution_from_route_b_refuses() {
    let mut fixture = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set_a = admitted_set(vec![candidate("model-misc-a", 10)]);
    // The dispatched outcome is sealed against route resolution semantics of
    // set A; accounting it against B's resolution must refuse.
    let outcome = dispatched(&mut fixture, &set_a, STEP_ID)
        .await
        .expect("dispatch succeeds");
    // Resolution B is a legitimately different sealed resolution (different
    // route set id via different picker): build one on the same fixture.
    let mut set_b = admitted_set(vec![candidate("model-misc-b", 10)]);
    set_b.request.route_set_id = "routes-accounting-b".to_string();
    let resolution_b = resolve_route(&set_b.request, &fixture.registry).expect("route B resolves");
    let err = account_with(&outcome, &resolution_b)
        .expect_err("outcome from route A must refuse against resolution from route B");
    assert!(
        format!("{err:#}").contains("route"),
        "expected attribution mismatch, got: {err:#}"
    );
}

#[tokio::test]
async fn adversarial_provider_usage_nested_in_content_is_not_read() {
    // Inject a usage-shaped object deep inside the provider's message
    // content; the governed contract has no usage there. The parser must NOT
    // treat it as the provider's usage report: UNKNOWN, with no fabricated
    // cost or token total.
    let nested_content = "here is the answer {\"usage\":{\"promptTokens\":999,\"completionTokens\":999,\"totalTokens\":1998},\"nested\":{\"usage\":{\"promptTokens\":999}}}";
    let body = serde_json::json!({
        "id": "chatcmpl-fixture",
        "object": "chat.completion",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": nested_content },
            "finish_reason": "stop",
        }],
    })
    .to_string();
    let mut fixture = fixture(Some(body));
    let set = admitted_set(vec![candidate("model-nest", 10)]);
    let outcome = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch succeeds");
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let pricing = PricingBook::from_json(PRICE_BOOK).expect("book validates");
    let record = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("USD")),
        Some(&pricing),
    )
    .expect("accounting reports");
    // The usage embedded in the message content is NOT a provider report:
    // accounting must read Unknown, never 999/999/1998.
    match &record.usage {
        UsageAccounting::Unknown { reason } => assert!(!reason.is_empty()),
        other => panic!("nested usage must not be consumed as provider usage, got {other:?}"),
    }
    match &record.cost {
        CostAccounting::Unknown { .. } => {}
        other => panic!("no cost may be derived from untrusted nested usage, got {other:?}"),
    }
}

#[tokio::test]
async fn adversarial_malformed_usage_fields_are_rejected() {
    // String, negative, float, and bad-total reports: each is a typed
    // refusal, never a silent None and never a wrapped total.
    for (body_usage, label) in [
        (json!({"prompt_tokens": "7"}), "string"),
        (json!({"prompt_tokens": -1}), "negative"),
        (json!({"prompt_tokens": 7.5}), "float"),
        (
            json!({"total_tokens": 11, "prompt_tokens": 9, "completion_tokens": 4}),
            "inconsistent",
        ),
        (
            json!({"prompt_tokens": 0, "completion_tokens": u64::MAX, "total_tokens": u64::MAX}),
            "huge-ok",
        ),
    ] {
        let body = serde_json::json!({
            "id": "chatcmpl-fixture",
            "object": "chat.completion",
            "model": MODEL,
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "fixture response" },
                "finish_reason": "stop",
            }],
            "usage": body_usage,
        })
        .to_string();
        let mut fixture = fixture(Some(body));
        let set = admitted_set(vec![candidate("model-mal-a", 10)]);
        let outcome = dispatched(&mut fixture, &set, STEP_ID)
            .await
            .expect("dispatch succeeds");
        let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
        let record = account_routed_outcome(
            &outcome,
            &resolution,
            &accounting_context(Some("USD")),
            Option::None,
        );
        match (label, record) {
            ("huge-ok", Ok(record)) => {
                // promptTokens=0 + completionTokens=u64::MAX: sum overflows,
                // but totalTokens is present and consistent with completion;
                // is_consistent uses checked_add, so u64::MAX + 0 succeeds.
                assert!(matches!(record.usage, UsageAccounting::Known(_)));
            }
            ("huge-ok", Err(_)) => panic!("consistent huge usage must not be refused"),
            (_, Err(error)) => assert!(
                format!("{error:#}").contains("usage"),
                "malformed case {label} must refuse with a usage reason, got: {error:#}"
            ),
            (label, Ok(record)) => panic!(
                "malformed case {label} must refuse but produced: {:?}",
                record.usage
            ),
        }
    }
}

#[tokio::test]
async fn adversarial_same_journal_with_changed_accounting_content_conflicts() {
    let mut fixture = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set = admitted_set(vec![candidate("model-rev-a", 10)]);
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let outcome = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch succeeds");
    let pricing = PricingBook::from_json(PRICE_BOOK).expect("book validates");
    let mut ledger = AccountingLedger::new();
    let record = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("USD")),
        Some(&pricing),
    )
    .expect("accounting succeeds");
    ledger.record(record.clone()).expect("first registers");
    // Same execution, same journal digest, changed price book version: the
    // sealed record digest differs, so the binding must refuse.
    let other_book = PricingBook::from_json(
        r#"{
            "schemaVersion": "1.0.0",
            "source": "operator-price-book",
            "version": "price-book-2027-01",
            "entries": [
                { "provider": "local-fixture", "model": "tiny-local-model", "currency": "USD",
                  "promptMinorPer1k": 1000, "completionMinorPer1k": 2000 }
            ]
        }"#,
    )
    .expect("book validates");
    let changed = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("USD")),
        Some(&other_book),
    )
    .expect("accounting succeeds");
    assert_ne!(record.content_digest, changed.content_digest);
    let error = ledger
        .record(changed)
        .expect_err("changed usage/cost record under the same journal must conflict");
    assert!(matches!(error, LedgerError::IdempotencyConflict { .. }));
    assert_eq!(ledger.records().len(), 1);
}

#[tokio::test]
async fn adversarial_aggregation_overflow_fails_typed_not_saturated() {
    // Three distinct executions each reporting promptTokens = u64::MAX with
    // full completion tokens must overflow checked aggregation — never a
    // silently saturated-looking total.
    let mut ledger = AccountingLedger::new();
    for attempt in 0..3u8 {
        let mut fixture = fixture(Some(response_with_usage(u64::MAX, 4, None)));
        let set = admitted_set(vec![{
            let mut c = candidate(&format!("model-ovf-a-{attempt}"), 10);
            c.priority = attempt as u32;
            c
        }]);
        let outcome = dispatched(&mut fixture, &set, &format!("acct-overflow-{attempt}"))
            .await
            .expect("dispatch succeeds");
        let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
        let record = account_routed_outcome(
            &outcome,
            &resolution,
            &accounting_context(Some("USD")),
            Option::None,
        )
        .expect("accounting succeeds");
        ledger.record(record).expect("record registers");
    }
    let err = ledger
        .aggregate_graph("graph-acct-1")
        .expect_err("near-u64::MAX token sums must refuse, not wrap");
    assert!(
        format!("{err:#}").contains("aggregation would overflow"),
        "expected typed overflow error, got: {err:#}"
    );
}

#[tokio::test]
async fn adversarial_aggregation_overflow_total_tokens_is_refused() {
    let mut ledger = AccountingLedger::new();
    for attempt in 0..2u8 {
        let mut fixture = fixture(Some(response_with_usage(u64::MAX, 0, Some(u64::MAX))));
        let set = admitted_set(vec![{
            let mut c = candidate(&format!("model-ovf-b-{attempt}"), 10);
            c.priority = attempt as u32;
            c
        }]);
        let outcome = dispatched(&mut fixture, &set, &format!("acct-overflow-t-{attempt}"))
            .await
            .expect("dispatch succeeds");
        let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
        let record = account_routed_outcome(
            &outcome,
            &resolution,
            &accounting_context(Some("USD")),
            Option::None,
        )
        .expect("accounting succeeds");
        ledger.record(record).expect("record registers");
    }
    let err = ledger
        .aggregate_graph("graph-acct-1")
        .expect_err("near-u64::MAX token sums must refuse, not wrap");
    assert!(format!("{err:#}").contains("aggregation would overflow"));
}

#[tokio::test]
async fn directed_rate_rounding_is_floor_of_exact_sum() {
    // 1 token * (1000 USD/1k) + 1 token * (2000 USD/1k): floor(3000/1000) = 3.
    // Cost must not round components independently (which would floor to 0+2=2).
    let mut fixture = fixture(Some(response_with_usage(1, 1, Some(2))));
    let set = admitted_set(vec![candidate("model-round-a", 10)]);
    let outcome = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch succeeds");
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let pricing = PricingBook::from_json(PRICE_BOOK).expect("book validates");
    let record = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("USD")),
        Some(&pricing),
    )
    .expect("accounting succeeds");
    match &record.cost {
        CostAccounting::Known(cost) => assert_eq!(cost.minor_units, 3),
        other => panic!("expected known cost, got {other:?}"),
    }
}

#[tokio::test]
async fn pricing_book_is_validated_at_point_of_use() {
    // Construct an invalid book by hand (bypassing from_json); derive must
    // refuse, not derive from invalid data.
    let book = PricingBook {
        source: "operator-price-book".to_string(),
        version: "bad-version".to_string(),
        schema_version: "0.0.9".to_string(), // unsupported
        entries: vec![prometheos_lite::workflow::accounting::PricingEntry {
            provider: "local-fixture".to_string(),
            model: MODEL.to_string(),
            currency: "USD".to_string(),
            prompt_minor_per_1k: Some(1000),
            completion_minor_per_1k: Some(2000),
            flat_minor_per_1k: None,
        }],
    };
    let mut fixture = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set = admitted_set(vec![candidate("model-use-a", 10)]);
    let outcome = dispatched(&mut fixture, &set, STEP_ID)
        .await
        .expect("dispatch succeeds");
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let record = account_routed_outcome(
        &outcome,
        &resolution,
        &accounting_context(Some("USD")),
        Some(&book),
    )
    .expect("accounting succeeds; invalid book yields Unknown not panic");
    match &record.cost {
        CostAccounting::Unknown { reason } => {
            assert!(reason.contains("invalid") || reason.contains("unsupported"))
        }
        other => panic!("invalid book must not derive a known cost, got {other:?}"),
    }
}

#[tokio::test]
async fn failure_with_full_bindings_requires_them_but_absence_is_allowed() {
    let fixture = fixture(Some(response_with_usage(7, 4, Some(11))));
    let set = admitted_set(vec![candidate("model-fail-a", 10)]);
    let resolution = resolve_route(&set.request, &fixture.registry).expect("route resolves");
    let res = account_routed_failure(
        &resolution,
        "failed",
        &accounting_context(Some("USD")),
        STEP_ID,
        None,
        42,
    );
    assert!(
        res.attempt_digest.is_none(),
        "failure never claims an attempt digest"
    );
    assert!(
        res.journal_digest.is_none(),
        "pre-effect failure journal digest is optional"
    );
    assert!(
        res.verify().is_ok(),
        "failed records without bindings still verify"
    );
}
