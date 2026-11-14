//! Routed dispatch binding for governed routes (issue #133, Slice B).
//!
//! End-to-end over the real governed boundary: one authorized route executes
//! a real local governed capability exactly once; every refusal executes it
//! zero times. A pinned or uniformly-selected route never falls back to a
//! different provider/model/harness/workspace, and the sealed RoutedAttemptV1
//! binds the attempt to its durable journal evidence.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use tempfile::TempDir;

use prometheos_lite::workflow::execution_harness::{
    HARNESS_SCHEMA_VERSION, HarnessDeclarationV1, HarnessRegistry, HarnessStep,
};
use prometheos_lite::workflow::native_harness::{
    NATIVE_HARNESS_ID, NATIVE_INSPECT_CAPABILITY, NativeHarness, NativeInspectConfig,
    native_capability,
};
use prometheos_lite::workflow::node_runner::{Capability, CapabilityRegistry, NodeRunner};
use prometheos_lite::workflow::policy::LocalRestrictions;
use prometheos_lite::workflow::route_dispatch::{
    RoutedDispatchFailure, RoutedDispatchOutcome, RoutedDispatchRequest, dispatch_resolved_route,
};
use prometheos_lite::workflow::route_resolution::{
    RouteCandidateV1, RouteProfile, RouteResolutionRequest, RouteTrust, RouteWorkspaceBinding,
    resolve_route,
};
use prometheos_lite::workflow::soma::capability::{
    Identity, SUPPORTED_CAPABILITY_SCHEMA_VERSION, WorkRequirements,
};
use prometheos_lite::workflow::soma::contracts::AuthorityProfile;
use prometheos_lite::workflow::soma::profile::Boundary;
use prometheos_lite::workflow::workspace::{
    AdapterKind, WORKSPACE_SCHEMA_VERSION, WorkspaceManifestV1, WorkspaceMode,
};

mod common;
use common::permit_for;

const NODE_ID: &str = "capability-step";
const STEP_ID: &str = "routed-step-1";

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
            "purpose": "routed dispatch step",
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

fn budgets(tokens: u64) -> prometheos_lite::workflow::soma::contracts::Budgets {
    serde_json::from_value(json!({ "tokens": tokens })).expect("budgets parse")
}

fn authority() -> AuthorityProfile {
    serde_json::from_str(r#"{"executionClass":"deterministic","mutation":"none"}"#)
        .expect("minimal authority parses")
}

fn requirements() -> WorkRequirements {
    serde_json::from_value(json!({
        "id": "reqs-route",
        "schemaVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": false,
        "forbidSubstitution": true,
        "requiredCapabilities": ["filesystem-read"],
    }))
    .expect("requirements parse")
}

fn requirements_with_workspace(boundary: Boundary) -> WorkRequirements {
    serde_json::from_value(json!({
        "id": "reqs-ws",
        "schemaVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": false,
        "forbidSubstitution": true,
        "requiredCapabilities": ["filesystem-read"],
        "workspace": { "workspace": match boundary { Boundary::Write => "write", Boundary::Read => "read", Boundary::None => "none" } },
    }))
    .expect("requirements with workspace parse")
}

fn candidate(id: &str, priority: u32) -> RouteCandidateV1 {
    RouteCandidateV1 {
        schema_version: "1.0.0".to_string(),
        candidate_id: id.to_string(),
        provider_identity: format!("provider-{id}"),
        provider_locality: prometheos_lite::workflow::soma::capability::Locality::Local,
        model: Some(format!("model-{id}")),
        harness_id: NATIVE_HARNESS_ID.to_string(),
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

fn binding(adapter: AdapterKind, revision: &str) -> RouteWorkspaceBinding {
    RouteWorkspaceBinding {
        adapter,
        revision: revision.to_string(),
    }
}

fn sealed_workspace_manifest(workspace_id: &str, mode: WorkspaceMode) -> WorkspaceManifestV1 {
    WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
        workspace_id: workspace_id.to_string(),
        adapter: AdapterKind::GitWorktree,
        adapter_revision: "lite.workspace.adapter.v1".to_string(),
        repo_identity: "test-repo".to_string(),
        base_revision: "base-rev".to_string(),
        branch: None,
        mode,
        writable_scopes: if matches!(mode, WorkspaceMode::Writable) {
            vec!["work://y".to_string()]
        } else {
            vec![]
        },
        resource_lock_id: format!("lock-{workspace_id}"),
        created_at: "2026-12-01T00:00:00Z".to_string(),
        content_digest: None,
    }
    .sealed()
}

/// Native probe harness whose declaration a caller can re-mint with a
/// different revision (registry-mutation simulation).
#[derive(Clone)]
struct ProbeHarness {
    declaration: HarnessDeclarationV1,
}

impl ProbeHarness {
    fn new(declaration: HarnessDeclarationV1) -> Self {
        Self { declaration }
    }
}

#[async_trait::async_trait]
impl prometheos_lite::workflow::execution_harness::ExecutionHarness for ProbeHarness {
    fn declaration(&self) -> &HarnessDeclarationV1 {
        &self.declaration
    }

    async fn run(
        &self,
        _step: prometheos_lite::workflow::execution_harness::HarnessStep,
        _dispatch: &mut prometheos_lite::workflow::execution_harness::HarnessDispatch<'_>,
    ) -> anyhow::Result<prometheos_lite::workflow::execution_harness::HarnessOutcome> {
        anyhow::bail!("probe harness must not be invoked")
    }
}

fn probe_candidate_declaration(revision: &str) -> HarnessDeclarationV1 {
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: NATIVE_HARNESS_ID.to_string(),
        identity: Identity {
            name: "lite-native-inspect".to_string(),
            revision: revision.to_string(),
        },
        family: prometheos_lite::workflow::execution_harness::HarnessFamily::Native,
        declared_capabilities: vec![NATIVE_INSPECT_CAPABILITY.to_string()],
        claimed_capabilities: vec![NATIVE_INSPECT_CAPABILITY.to_string()],
        capabilities: prometheos_lite::workflow::soma::capability::RuntimeCapabilitySet {
            id: format!("{NATIVE_INSPECT_CAPABILITY}.caps"),
            schema_version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
            identity: Identity {
                name: "lite-native-inspect".to_string(),
                revision: revision.to_string(),
            },
            declared_at: "2026-12-01T00:00:00Z".to_string(),
            spec_versions: vec![SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string()],
            adapter_families: Vec::new(),
            effect_classes: vec![NATIVE_INSPECT_CAPABILITY.to_string()],
            workspace: None,
            checkpoint_import: Some(false),
            checkpoint_export: Some(false),
            resume_support: Some(false),
            event_streaming: Some(true),
            read_projection: Some(true),
            locality: Some(prometheos_lite::workflow::soma::capability::Locality::Local),
            privacy_classes: Vec::new(),
            max_structured_input: None,
            max_structured_output: None,
            budget_ceilings: Some(budgets(10_000)),
            evidence_attachment: Some(true),
            validation: Some(true),
            approval_escalation: Some(false),
        },
        content_digest: None,
    }
    .sealed()
}

/// Build a registry whose harness declaration is sealed for `revision`.
fn probe_registry(revision: &str) -> HarnessRegistry {
    let mut registry = HarnessRegistry::new();
    registry
        .register(Box::new(ProbeHarness::new(probe_candidate_declaration(
            revision,
        ))))
        .expect("probe registers");
    registry
}

fn counted_native_capability(counter: Arc<AtomicUsize>, root: std::path::PathBuf) -> Capability {
    let native = native_capability(NativeInspectConfig::default(), root);
    let handler = native.handler.expect("native handler is present");
    let effect_counter = counter.clone();
    Capability::deterministic(&["path"], move |args| {
        effect_counter.fetch_add(1, Ordering::SeqCst);
        handler(args)
    })
}

fn failing_capability(counter: Arc<AtomicUsize>, message: &'static str) -> Capability {
    Capability::deterministic(&["path"], move |_args| {
        counter.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("{message}")
    })
}

struct Fixture {
    _dir: TempDir,
    counter: Arc<AtomicUsize>,
    harness_registry: HarnessRegistry,
    runner: NodeRunner,
}

fn fixture() -> Fixture {
    let dir = tempfile::Builder::new()
        .prefix("route-dispatch-fixture")
        .tempdir()
        .expect("tempdir");
    std::fs::write(dir.path().join("payload.txt"), "slice-b-route").expect("fixture file");
    let counter = Arc::new(AtomicUsize::new(0));
    let mut capability_registry = CapabilityRegistry::new();
    capability_registry.declare(
        NATIVE_INSPECT_CAPABILITY,
        counted_native_capability(counter.clone(), dir.path().to_path_buf()),
    );
    let permit = permit_for(&[NODE_ID], &[NATIVE_INSPECT_CAPABILITY]);
    let runner = NodeRunner::new(capability_registry, permit);
    let mut harness_registry = HarnessRegistry::new();
    harness_registry
        .register(Box::new(NativeHarness::new()))
        .expect("native harness registers");
    Fixture {
        _dir: dir,
        counter,
        harness_registry,
        runner,
    }
}

fn step() -> HarnessStep {
    HarnessStep {
        step_id: STEP_ID.to_string(),
        capability: NATIVE_INSPECT_CAPABILITY.to_string(),
        args: json!({ "path": "payload.txt" }),
    }
}

struct RoutedSet {
    request: RouteResolutionRequest<'static>,
}

fn routed_set(candidates: Vec<RouteCandidateV1>) -> RoutedSet {
    let requirements = Box::leak(Box::new(requirements()));
    let authority = Box::leak(Box::new(authority()));
    let request = RouteResolutionRequest {
        route_set_id: "routes-slice-b".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates,
        pinned_candidate_id: None,
        requirements,
        authority_declared: authority,
        authority_effective: authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-12-01T12:00:00Z".to_string(),
        workspace: None,
    };
    RoutedSet { request }
}

async fn dispatch(
    fixture: &mut Fixture,
    set: &RoutedSet,
    dispatched_at: &str,
) -> Result<RoutedDispatchOutcome, RoutedDispatchFailure> {
    let resolution =
        resolve_route(&set.request, &fixture.harness_registry).expect("route resolves");
    dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &set.request,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at,
        step: step(),
    })
    .await
}

#[tokio::test]
async fn authorized_route_executes_exactly_once() {
    let mut fixture = fixture();
    let set = routed_set(vec![candidate("native-a", 10)]);
    let outcome = dispatch(&mut fixture, &set, "2026-12-01T12:00:01Z")
        .await
        .expect("authorized route executes");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 1);
    assert_eq!(outcome.attempt.governed_calls, 1);
    assert_eq!(outcome.attempt.idempotency_key, STEP_ID);
    assert_eq!(outcome.attempt.harness_id, NATIVE_HARNESS_ID);
    assert_eq!(outcome.attempt.provider_identity, "provider-native-a");
    assert_eq!(outcome.attempt.outcome_kind, "completed");
    assert_eq!(outcome.attempt.journal_entry.identity_key, STEP_ID);
    assert!(outcome.attempt.journal_entry.prev_entry_digest.is_none());
    assert!(outcome.attempt.journal_entry.detail_digest.len() == 64);
    assert!(outcome.attempt.verify().is_ok());
}

#[tokio::test]
async fn attempt_binds_registry_candidate_provider_model_and_workspace() {
    let mut fixture = fixture();
    let set = routed_set(vec![candidate("native-b", 10)]);
    let resolution =
        resolve_route(&set.request, &fixture.harness_registry).expect("route resolves");
    assert_eq!(resolution.snapshot.selected.candidate_id, "native-b");
    let outcome = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &set.request,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: step(),
    })
    .await
    .expect("binds and dispatches");
    let attempt = &outcome.attempt;
    assert_eq!(
        attempt.snapshot_digest,
        resolution.snapshot.content_digest.as_deref().unwrap()
    );
    assert_eq!(
        attempt.route_evidence_digest,
        resolution.evidence.content_digest.as_deref().unwrap()
    );
    assert_eq!(attempt.decision_digest, resolution.snapshot.decision_digest);
    assert_eq!(attempt.model.as_deref(), Some("model-native-b"));
    assert_eq!(attempt.provider_identity, "provider-native-b");
    assert_eq!(attempt.manifest_digest.len(), 64);
    assert!(attempt.verify().is_ok());
}

#[tokio::test]
async fn stale_metadata_between_resolve_and_dispatch_refuses() {
    // Fresh at resolve (TTL valid at 12:00:00), stale at dispatch (12:00:01
    // + 1s of TTL budget): dispatch-time freshness wins.
    let mut fixture = fixture();
    let mut short = candidate("native-c", 10);
    short.trust = trust("2026-12-01T12:00:00Z", 1); // expires at exactly 12:00:01
    let set = routed_set(vec![short]);
    let resolution =
        resolve_route(&set.request, &fixture.harness_registry).expect("route resolves (fresh)");
    let err = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &set.request,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: step(),
    })
    .await
    .expect_err("expired trust at dispatch time refuses");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 0);
    assert!(matches!(err, RoutedDispatchFailure::Refused(_)));
    assert!(format!("{err}").contains("stale"));
}

#[tokio::test]
async fn dispatched_at_before_evaluated_at_refuses() {
    let mut fixture = fixture();
    let set = routed_set(vec![candidate("native-d", 10)]);
    let resolution =
        resolve_route(&set.request, &fixture.harness_registry).expect("route resolves");
    let err = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &set.request,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-11-30T11:00:00Z", // before evaluatedAt
        step: step(),
    })
    .await
    .expect_err("dispatchedAt before evaluatedAt refuses");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 0);
    assert!(matches!(err, RoutedDispatchFailure::Refused(_)));
}

#[tokio::test]
async fn tampered_snapshot_refuses_before_effects() {
    let mut fixture = fixture();
    let set = routed_set(vec![candidate("native-e", 10)]);
    let resolution =
        resolve_route(&set.request, &fixture.harness_registry).expect("route resolves");
    let mut tampered = resolution.clone();
    tampered.snapshot.selected.provider_identity = "provider-forged".to_string();
    // Re-sealing is what an attacker would attempt; the digest no longer
    // verifies, so the dispatch refuses before effects.
    let err = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &tampered,
        original_request: &set.request,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: step(),
    })
    .await
    .expect_err("tampered snapshot refuses");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 0);
    assert!(matches!(err, RoutedDispatchFailure::Refused(_)));
}

#[tokio::test]
async fn tampered_evidence_refuses_before_effects() {
    let mut fixture = fixture();
    let set = routed_set(vec![candidate("native-f", 10)]);
    let resolution =
        resolve_route(&set.request, &fixture.harness_registry).expect("route resolves");
    let mut tampered = resolution.clone();
    tampered.evidence.selected.candidate_id = "route-forged".to_string();
    let err = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &tampered,
        original_request: &set.request,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: step(),
    })
    .await
    .expect_err("tampered evidence refuses");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 0);
    assert!(matches!(err, RoutedDispatchFailure::Refused(_)));
}

#[tokio::test]
async fn registry_mutation_after_resolution_refuses() {
    let mut fixture = fixture();
    let set = routed_set(vec![candidate("native-g", 10)]);
    let resolution =
        resolve_route(&set.request, &fixture.harness_registry).expect("route resolves");
    // The registry is rebuilt with a re-sealed declaration at a different
    // revision: the dispatch must refuse.
    let mutated = probe_registry("v2");
    let err = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &set.request,
        registry: &mutated,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: step(),
    })
    .await
    .expect_err("registry mutation refuses");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 0);
    assert!(matches!(err, RoutedDispatchFailure::Refused(_)));
}

#[tokio::test]
async fn swapped_candidate_model_refuses_before_effects() {
    let mut fixture = fixture();
    let candidates = vec![candidate("native-h", 10)];
    let requirements = requirements();
    let authority = authority();
    let request = RouteResolutionRequest {
        route_set_id: "routes-slice-b".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: candidates.clone(),
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-12-01T12:00:00Z".to_string(),
        workspace: None,
    };
    let resolution = resolve_route(&request, &fixture.harness_registry).expect("route resolves");
    // Operator "edits" the candidate after resolution (silent substitution attempt).
    let mut swapped = candidates[0].clone();
    swapped.model = Some("substituted-model".to_string());
    let request_swapped = RouteResolutionRequest {
        route_set_id: "routes-slice-b".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![swapped],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-12-01T12:00:00Z".to_string(),
        workspace: None,
    };
    let err = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &request_swapped,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: step(),
    })
    .await
    .expect_err("model substitution refuses");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 0);
    assert!(matches!(err, RoutedDispatchFailure::Refused(_)));
}

#[tokio::test]
async fn swapped_candidate_provider_refuses_before_effects() {
    let mut fixture = fixture();
    let mut c = candidate("native-i", 10);
    let requirements = requirements();
    let authority = authority();
    let original = RouteResolutionRequest {
        route_set_id: "routes-slice-b".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![c.clone()],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-12-01T12:00:00Z".to_string(),
        workspace: None,
    };
    let resolution = resolve_route(&original, &fixture.harness_registry).expect("route resolves");
    c.provider_identity = "cloud-other".to_string();
    let swapped = RouteResolutionRequest {
        route_set_id: "routes-slice-b".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![c],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-12-01T12:00:00Z".to_string(),
        workspace: None,
    };
    let err = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &swapped,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: step(),
    })
    .await
    .expect_err("provider substitution refuses");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 0);
    assert!(matches!(err, RoutedDispatchFailure::Refused(_)));
}

#[tokio::test]
async fn swapped_candidate_removed_from_set_refuses() {
    let mut fixture = fixture();
    let candidates = vec![candidate("native-j", 10)];
    let requirements = requirements();
    let authority = authority();
    let original = RouteResolutionRequest {
        route_set_id: "routes-slice-b".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates,
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-12-01T12:00:00Z".to_string(),
        workspace: None,
    };
    let resolution = resolve_route(&original, &fixture.harness_registry).expect("route resolves");
    let mutated = RouteResolutionRequest {
        candidates: vec![],
        ..original
    };
    let err = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &mutated,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: step(),
    })
    .await
    .expect_err("missing selected candidate refuses");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 0);
    assert!(matches!(err, RoutedDispatchFailure::Refused(_)));
}

#[tokio::test]
async fn workspace_manifest_changed_between_resolve_and_dispatch_refuses() {
    let mut fixture = fixture();
    let requirements = requirements_with_workspace(Boundary::Read);
    let authority = authority();
    let manifest_sealed = sealed_workspace_manifest("ws-a", WorkspaceMode::ReadOnly);
    let mut candidate_ws = candidate("native-k", 10);
    candidate_ws.workspace = Some(binding(
        AdapterKind::GitWorktree,
        "lite.workspace.adapter.v1",
    ));
    let original = RouteResolutionRequest {
        route_set_id: "routes-ws".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![candidate_ws],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-12-01T12:00:00Z".to_string(),
        workspace: Some(&manifest_sealed),
    };
    let resolution = resolve_route(&original, &fixture.harness_registry).expect("route resolves");
    // Different contents -> different digest.
    let other = sealed_workspace_manifest("ws-b", WorkspaceMode::ReadOnly);
    let mutated = RouteResolutionRequest {
        workspace: Some(&other),
        ..original
    };
    let err = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &mutated,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: step(),
    })
    .await
    .expect_err("changed workspace manifest refuses");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 0);
    assert!(matches!(err, RoutedDispatchFailure::Refused(_)));
}

#[tokio::test]
async fn workspace_binding_to_different_adapter_refuses() {
    let fixture = fixture();
    let requirements = requirements_with_workspace(Boundary::Write);
    let authority = authority();
    let manifest_sealed = sealed_workspace_manifest("ws-c", WorkspaceMode::Writable);
    let mut candidate_ws = candidate("native-l", 10);
    candidate_ws.workspace = Some(binding(
        AdapterKind::ExistingReadOnly,
        "lite.workspace.adapter.v1",
    ));
    let original = RouteResolutionRequest {
        route_set_id: "routes-ws".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![candidate_ws],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-12-01T12:00:00Z".to_string(),
        workspace: Some(&manifest_sealed),
    };
    let err = resolve_route(&original, &fixture.harness_registry)
        .expect_err("mismatched adapter binding refuses at resolution");
    assert!(format!("{err:#}").contains("does not match candidate binding"));
}

#[tokio::test]
async fn step_capability_not_declared_by_selected_harness_refuses() {
    let mut fixture = fixture();
    let set = routed_set(vec![candidate("native-m", 10)]);
    let resolution =
        resolve_route(&set.request, &fixture.harness_registry).expect("route resolves");
    let mut undeclared_step = step();
    undeclared_step.capability = "harness.subprocess.spawn".to_string();
    let err = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &set.request,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: undeclared_step,
    })
    .await
    .expect_err("undeclared capability refuses");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 0);
    assert!(matches!(err, RoutedDispatchFailure::Refused(_)));
}

#[tokio::test]
async fn permit_denies_unlisted_node_before_effects() {
    let mut fixture = fixture();
    let mut other_capability_registry = CapabilityRegistry::new();
    other_capability_registry.declare(
        NATIVE_INSPECT_CAPABILITY,
        counted_native_capability(fixture.counter.clone(), fixture._dir.path().to_path_buf()),
    );
    // Deliberately do NOT grant the node id to the permit.
    let permit = permit_for(&["someone-else"], &[NATIVE_INSPECT_CAPABILITY]);
    fixture.runner = NodeRunner::new(other_capability_registry, permit);
    let set = routed_set(vec![candidate("native-n", 10)]);
    let err = dispatch(&mut fixture, &set, "2026-12-01T12:00:01Z")
        .await
        .expect_err("permit-denied node cannot reach the effect");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 0);
    assert!(matches!(err, RoutedDispatchFailure::ExecutionFailed { .. }));
}

#[tokio::test]
async fn cancelled_failure_preserves_typed_evidence_without_replay() {
    let dir = tempfile::Builder::new()
        .prefix("route-dispatch-cancel")
        .tempdir()
        .expect("tempdir");
    std::fs::write(dir.path().join("payload.txt"), "cancel-route").expect("fixture");
    let counter = Arc::new(AtomicUsize::new(0));
    let mut capability_registry = CapabilityRegistry::new();
    capability_registry.declare(
        NATIVE_INSPECT_CAPABILITY,
        failing_capability(counter.clone(), "request cancelled by operator"),
    );
    let permit = permit_for(&[NODE_ID], &[NATIVE_INSPECT_CAPABILITY]);
    let runner = NodeRunner::new(capability_registry, permit);
    let harness_registry = {
        let mut r = HarnessRegistry::new();
        r.register(Box::new(NativeHarness::new()))
            .expect("native harness registers");
        r
    };
    let requirements = requirements();
    let authority = authority();
    let candidates = vec![candidate("native-o", 10)];
    let request = RouteResolutionRequest {
        route_set_id: "routes-slice-b".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates,
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-12-01T12:00:00Z".to_string(),
        workspace: None,
    };
    let resolution = resolve_route(&request, &harness_registry).expect("route resolves");
    let mut local_fixture = Fixture {
        _dir: dir,
        counter: counter.clone(),
        harness_registry,
        runner,
    };
    let err = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &request,
        registry: &local_fixture.harness_registry,
        runner: &mut local_fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: step(),
    })
    .await
    .expect_err("handler failure is a typed, preserved failure");
    assert_eq!(counter.load(Ordering::SeqCst), 1, "effect ran exactly once");
    match err {
        RoutedDispatchFailure::ExecutionFailed(failure) => {
            assert_eq!(failure.calls, 1);
            assert!(format!("{:#}", failure.source).contains("cancelled"));
            assert_eq!(
                failure
                    .journal_entry
                    .as_ref()
                    .map(|e| e.identity_key.as_str()),
                None,
                "pre-effect failure produces no terminal journal entry"
            );
        }
        RoutedDispatchFailure::Refused(_) => panic!("handler failure must be ExecutionFailed"),
    }
}

#[tokio::test]
async fn failing_attempt_preserves_prior_evidence_and_does_not_replay() {
    let dir = tempfile::Builder::new()
        .prefix("route-dispatch-continuity")
        .tempdir()
        .expect("tempdir");
    std::fs::write(dir.path().join("payload.txt"), "continuity").expect("fixture");
    let counter = Arc::new(AtomicUsize::new(0));
    // Single governed registry whose handler succeeds on the first call and
    // fails on the second — one continuous journal, deterministic outcome.
    let mut capability_registry = CapabilityRegistry::new();
    let native = native_capability(NativeInspectConfig::default(), dir.path().to_path_buf());
    let native_handler = native.handler.expect("handler present");
    let counter2 = counter.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls2 = calls.clone();
    capability_registry.declare(
        NATIVE_INSPECT_CAPABILITY,
        Capability::deterministic(&["path"], move |args| {
            counter2.fetch_add(1, Ordering::SeqCst);
            let call = calls2.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                native_handler(args)
            } else {
                anyhow::bail!("second attempt failed")
            }
        }),
    );
    let permit = permit_for(&[NODE_ID], &[NATIVE_INSPECT_CAPABILITY]);
    let runner = NodeRunner::new(capability_registry, permit);
    let harness_registry = {
        let mut r = HarnessRegistry::new();
        r.register(Box::new(NativeHarness::new()))
            .expect("native harness registers");
        r
    };
    let requirements = requirements();
    let authority = authority();
    let candidates = vec![candidate("native-p", 10), candidate("native-q", 20)];
    let request = RouteResolutionRequest {
        route_set_id: "routes-slice-b".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates,
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-12-01T12:00:00Z".to_string(),
        workspace: None,
    };
    // Step 1: success, journal entry #1.
    let resolution1 = resolve_route(&request, &harness_registry).expect("route resolves");
    let mut fixture = Fixture {
        _dir: dir,
        counter: counter.clone(),
        harness_registry,
        runner,
    };
    let step1 = HarnessStep {
        step_id: "step-first".to_string(),
        capability: NATIVE_INSPECT_CAPABILITY.to_string(),
        args: json!({ "path": "payload.txt" }),
    };
    let out1 = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution1,
        original_request: &request,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: step1.clone(),
    })
    .await
    .expect("first dispatch succeeds");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 1);
    let prior_entry = out1.attempt.journal_entry.clone();

    // Step 2: distinct step id on the SAME runner; the handler fails now.
    let err = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution1,
        original_request: &request,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:02Z",
        step: HarnessStep {
            step_id: "step-second".to_string(),
            ..step1
        },
    })
    .await
    .expect_err("second dispatch fails through its handler");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 2);
    match err {
        RoutedDispatchFailure::ExecutionFailed(failure) => {
            assert_eq!(failure.calls, 1);
            assert_eq!(
                failure
                    .journal_entry
                    .as_ref()
                    .map(|e| e.identity_key.as_str()),
                Some("step-first"),
                "failure preserves the prior durable journal entry"
            );
            assert_eq!(
                failure.journal_entry.unwrap().entry_digest,
                prior_entry.entry_digest,
                "prior evidence byte-identical"
            );
        }
        RoutedDispatchFailure::Refused(_) => panic!("expected ExecutionFailed"),
    }
}

#[tokio::test]
async fn successful_second_attempt_chains_onto_first_attempt_evidence() {
    let dir = tempfile::Builder::new()
        .prefix("route-dispatch-chain")
        .tempdir()
        .expect("tempdir");
    std::fs::write(dir.path().join("payload.txt"), "chain").expect("fixture");
    let counter = Arc::new(AtomicUsize::new(0));
    let mut capability_registry = CapabilityRegistry::new();
    let native = native_capability(NativeInspectConfig::default(), dir.path().to_path_buf());
    let native_handler = native.handler.expect("handler present");
    let counter2 = counter.clone();
    capability_registry.declare(
        NATIVE_INSPECT_CAPABILITY,
        Capability::deterministic(&["path"], move |args| {
            counter2.fetch_add(1, Ordering::SeqCst);
            native_handler(args)
        }),
    );
    let permit = permit_for(&[NODE_ID], &[NATIVE_INSPECT_CAPABILITY]);
    let runner = NodeRunner::new(capability_registry, permit);
    let harness_registry = {
        let mut r = HarnessRegistry::new();
        r.register(Box::new(NativeHarness::new()))
            .expect("native harness registers");
        r
    };
    let requirements = requirements();
    let authority = authority();
    let candidates = vec![candidate("native-r", 10)];
    let request = RouteResolutionRequest {
        route_set_id: "routes-slice-b".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates,
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-12-01T12:00:00Z".to_string(),
        workspace: None,
    };
    let mut fixture = Fixture {
        _dir: dir,
        counter: counter.clone(),
        harness_registry,
        runner,
    };
    let resolution = resolve_route(&request, &fixture.harness_registry).expect("route resolves");
    let step_one = HarnessStep {
        step_id: "chain-one".to_string(),
        capability: NATIVE_INSPECT_CAPABILITY.to_string(),
        args: json!({ "path": "payload.txt" }),
    };
    let out1 = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &request,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: step_one,
    })
    .await
    .expect("first succeeds");
    let step_two = HarnessStep {
        step_id: "chain-two".to_string(),
        capability: NATIVE_INSPECT_CAPABILITY.to_string(),
        args: json!({ "path": "payload.txt" }),
    };
    let out2 = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &request,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:02Z",
        step: step_two,
    })
    .await
    .expect("second succeeds");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 2);
    assert_eq!(
        out2.attempt.journal_entry.prev_entry_digest.as_deref(),
        Some(out1.attempt.journal_entry.entry_digest.as_str()),
        "attempt N+1 chains onto attempt N's durable evidence"
    );
    assert!(out1.attempt.verify().is_ok());
    assert!(out2.attempt.verify().is_ok());
}

#[tokio::test]
async fn pinned_route_dispatches_the_pin_not_the_priority_winner() {
    let mut fixture = fixture();
    let candidates = vec![candidate("pinned-a", 50), candidate("pinned-b", 1)];
    let requirements = requirements();
    let authority = authority();
    let request = RouteResolutionRequest {
        route_set_id: "routes-slice-b".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates,
        pinned_candidate_id: Some("pinned-a"),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-12-01T12:00:00Z".to_string(),
        workspace: None,
    };
    let resolution = resolve_route(&request, &fixture.harness_registry).expect("pin resolves");
    assert_eq!(resolution.snapshot.selected.candidate_id, "pinned-a");
    let out = dispatch_resolved_route(RoutedDispatchRequest {
        resolution: &resolution,
        original_request: &request,
        registry: &fixture.harness_registry,
        runner: &mut fixture.runner,
        manifest: &manifest(),
        local_restrictions: &restrictions(),
        known_secrets: vec![],
        dispatched_at: "2026-12-01T12:00:01Z",
        step: step(),
    })
    .await
    .expect("pinned route dispatches");
    assert_eq!(out.attempt.provider_identity, "provider-pinned-a");
    assert_eq!(fixture.counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn attempt_tampering_fails_verification() {
    let mut fixture = fixture();
    let set = routed_set(vec![candidate("native-s", 10)]);
    let outcome = dispatch(&mut fixture, &set, "2026-12-01T12:00:01Z")
        .await
        .expect("dispatch succeeds");
    let mut tampered = outcome.attempt.clone();
    tampered.idempotency_key = "forged".to_string();
    assert!(tampered.verify().is_err());
    let mut tampered_budget = outcome.attempt.clone();
    tampered_budget.journal_entry.entry_digest = "0".repeat(64);
    assert!(tampered_budget.verify().is_err());
}
