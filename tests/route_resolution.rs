//! Governed route resolution and the immutable effective execution snapshot
//! (issue #133, Slice A: route decision / policy profile / preflight).
//!
//! These tests pin the bounded slice: one operator-owned candidate set is
//! resolved through the published SPEC 007 negotiation before any effect,
//! profiles only narrow, there is never a fallback, and the resulting
//! snapshot and evidence are sealed, deterministic and digest-verified.

use anyhow::{Result, bail};
use serde_json::json;

use prometheos_lite::workflow::execution_harness::{
    ExecutionHarness, HARNESS_SCHEMA_VERSION, HarnessDeclarationV1, HarnessDispatch, HarnessFamily,
    HarnessOutcome, HarnessRegistry, HarnessStep,
};
use prometheos_lite::workflow::route_resolution::{
    EXECUTION_SNAPSHOT_SCHEMA_VERSION, RouteCandidateV1, RouteProfile, RouteResolutionRequest,
    RouteTrust, RouteWorkspaceBinding, resolve_route,
};
use prometheos_lite::workflow::soma::capability::{
    Identity, RuntimeCapabilitySet, SUPPORTED_CAPABILITY_SCHEMA_VERSION, WorkRequirements,
};
use prometheos_lite::workflow::soma::contracts::{AuthorityProfile, Budgets};
use prometheos_lite::workflow::soma::profile::{Boundaries, Boundary};

use prometheos_lite::workflow::workspace::{
    AdapterKind, WORKSPACE_SCHEMA_VERSION, WorkspaceManifestV1, WorkspaceMode,
};

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn authority() -> AuthorityProfile {
    serde_json::from_str(r#"{"executionClass":"deterministic","mutation":"none"}"#)
        .expect("minimal authority profile parses")
}

fn authority_with_budgets(tokens: u64) -> AuthorityProfile {
    serde_json::from_value(json!({
        "executionClass": "deterministic",
        "mutation": "none",
        "budgets": { "tokens": tokens },
    }))
    .expect("authority profile with budgets parses")
}

fn capability_set(
    harness_id: &str,
    name: &str,
    revision: &str,
    classes: &[&str],
    locality: Option<prometheos_lite::workflow::soma::capability::Locality>,
    privacy: &[&str],
) -> RuntimeCapabilitySet {
    RuntimeCapabilitySet {
        id: format!("{harness_id}.caps"),
        schema_version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
        version: SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string(),
        identity: Identity {
            name: name.to_string(),
            revision: revision.to_string(),
        },
        declared_at: "2026-10-22T00:00:00Z".to_string(),
        spec_versions: vec![SUPPORTED_CAPABILITY_SCHEMA_VERSION.to_string()],
        adapter_families: Vec::new(),
        effect_classes: classes.iter().map(|c| c.to_string()).collect(),
        workspace: None,
        checkpoint_import: Some(false),
        checkpoint_export: Some(false),
        resume_support: Some(false),
        event_streaming: Some(true),
        read_projection: Some(true),
        locality,
        privacy_classes: privacy.iter().map(|c| c.to_string()).collect(),
        max_structured_input: None,
        max_structured_output: None,
        budget_ceilings: None,
        evidence_attachment: Some(true),
        validation: Some(true),
        approval_escalation: Some(false),
    }
}

fn declaration(
    harness_id: &str,
    name: &str,
    revision: &str,
    family: HarnessFamily,
    classes: &[&str],
    locality: Option<prometheos_lite::workflow::soma::capability::Locality>,
    privacy: &[&str],
) -> HarnessDeclarationV1 {
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: harness_id.to_string(),
        identity: Identity {
            name: name.to_string(),
            revision: revision.to_string(),
        },
        family,
        declared_capabilities: vec!["test.capability".to_string()],
        claimed_capabilities: vec!["test.capability".to_string()],
        capabilities: capability_set(harness_id, name, revision, classes, locality, privacy),
        content_digest: None,
    }
    .sealed()
}

/// An adapter that refuses to be invoked. Resolution must never reach it.
struct ProbeHarness {
    declaration: HarnessDeclarationV1,
}

impl ProbeHarness {
    fn new(declaration: HarnessDeclarationV1) -> Self {
        Self { declaration }
    }
}

#[async_trait::async_trait]
impl ExecutionHarness for ProbeHarness {
    fn declaration(&self) -> &HarnessDeclarationV1 {
        &self.declaration
    }

    async fn run(
        &self,
        _step: HarnessStep,
        _dispatch: &mut HarnessDispatch<'_>,
    ) -> Result<HarnessOutcome> {
        bail!("route resolution must never invoke the adapter")
    }
}

fn registry(declarations: Vec<HarnessDeclarationV1>) -> HarnessRegistry {
    let mut registry = HarnessRegistry::new();
    for declaration in declarations {
        registry
            .register(Box::new(ProbeHarness::new(declaration)))
            .expect("registration succeeds");
    }
    registry
}

fn requirements(required: &[&str]) -> WorkRequirements {
    let mut value = json!({
        "id": "reqs-route",
        "schemaVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": false,
        "forbidSubstitution": true,
    });
    if !required.is_empty() {
        value["requiredCapabilities"] = json!(required);
    }
    serde_json::from_value(value).expect("synthetic requirements parse")
}

fn requirements_with(extra: serde_json::Value) -> WorkRequirements {
    let mut value = json!({
        "id": "reqs-route",
        "schemaVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": false,
        "forbidSubstitution": true,
        "requiredCapabilities": ["test.capability"],
    });
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::from_value(value).expect("synthetic requirements parse")
}

fn budgets_cost(cost: u64) -> Budgets {
    serde_json::from_value(json!({ "cost": cost })).expect("budgets parse")
}

fn budgets_tokens_cost(tokens: u64, cost: u64) -> Budgets {
    serde_json::from_value(json!({ "tokens": tokens, "cost": cost })).expect("budgets parse")
}

fn authority_with_budgets_mixed(tokens: Option<u64>, cost: Option<u64>) -> AuthorityProfile {
    let mut value = json!({
        "executionClass": "deterministic",
        "mutation": "none",
        "budgets": {},
    });
    if let Some(t) = tokens {
        value["budgets"]["tokens"] = json!(t);
    }
    if let Some(c) = cost {
        value["budgets"]["cost"] = json!(c);
    }
    serde_json::from_value(value).expect("authority profile with mixed budgets parses")
}

fn local_decl_with_mixed_ceilings() -> HarnessDeclarationV1 {
    let mut caps = capability_set(
        "harness-local",
        "local-harness",
        "v1",
        &["test.capability"],
        Some(Locality::Local),
        &[],
    );
    caps.budget_ceilings = Some(budgets_tokens_cost(950, 30));
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: "harness-local".to_string(),
        identity: Identity {
            name: "local-harness".to_string(),
            revision: "v1".to_string(),
        },
        family: HarnessFamily::LocalModel,
        declared_capabilities: vec!["test.capability".to_string()],
        claimed_capabilities: vec!["test.capability".to_string()],
        capabilities: caps,
        content_digest: None,
    }
    .sealed()
}

#[test]
fn effective_budget_is_the_strict_intersection_of_all_ceilings() {
    // The concrete review case: declared 1000, effective authority 50,
    // work hard limit 900, candidate 500. The effective token ceiling must
    // never exceed 50 - and never silently discard the stricter ceiling.
    let registry = registry(vec![local_decl_with_budget_ceiling()]);
    let declared = authority_with_budgets(1000);
    let effective = authority_with_budgets(50);
    let requirements = requirements_with(json!({ "hardLimits": { "tokens": 900 } }));
    let mut c = candidate("route-intersect", Locality::Local, "harness-local", "v1", 1);
    c.budget = Some(budgets(500));
    let request = RouteResolutionRequest {
        route_set_id: "routes-intersect".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![c],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &declared,
        authority_effective: &effective,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    let resolution = resolve_route(&request, &registry)
        .expect("candidate ceiling narrows to the effective authority grant");
    assert_eq!(
        resolution.snapshot.effective_profile.effective.budgets,
        Some(budgets(50))
    );
    assert_eq!(
        resolution.snapshot.effective_profile.declared.budgets,
        Some(budgets(900)),
        "declared side is the strict intersection of authority/hard-limit/runtime ceilings"
    );
    // Sealed artifacts still pass integrity checks.
    assert!(resolution.snapshot.verify().is_ok());
    assert!(resolution.evidence.verify().is_ok());
    let snapshot = serde_json::to_value(&resolution.snapshot).expect("snapshot serializes");
    assert_eq!(
        snapshot["effectiveProfile"]["effective"]["budgets"]["tokens"],
        json!(50)
    );
    assert_eq!(
        snapshot["effectiveProfile"]["declared"]["budgets"]["tokens"],
        json!(900)
    );
}

#[test]
fn budget_intersection_is_per_dimension_and_independent() {
    // Partially populated sources intersect independently per dimension:
    // tokens -> min(authority 1000, hard 900, runtime 950) = 900,
    // cost    -> min(authority 40, runtime 30, eff authority 20, candidate 10) = 10.
    let registry = registry(vec![local_decl_with_mixed_ceilings()]);
    let declared = authority_with_budgets_mixed(Some(1000), Some(40));
    let effective = authority_with_budgets_mixed(None, Some(20));
    let requirements = requirements_with(json!({ "hardLimits": { "tokens": 900 } }));
    let mut c = candidate("route-dims", Locality::Local, "harness-local", "v1", 1);
    c.budget = Some(budgets_cost(10));
    let request = RouteResolutionRequest {
        route_set_id: "routes-dims".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![c],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &declared,
        authority_effective: &effective,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    let resolution =
        resolve_route(&request, &registry).expect("per-dimension intersection resolves");
    let effective_budgets = resolution
        .snapshot
        .effective_profile
        .effective
        .budgets
        .as_ref()
        .expect("effective budgets present");
    assert_eq!(
        effective_budgets.tokens,
        Some(serde_json::Number::from(900u64))
    );
    assert_eq!(
        effective_budgets.cost,
        Some(serde_json::Number::from(10u64))
    );
    let declared_budgets = resolution
        .snapshot
        .effective_profile
        .declared
        .budgets
        .as_ref()
        .expect("declared budgets present");
    assert_eq!(
        declared_budgets.tokens,
        Some(serde_json::Number::from(900u64))
    );
    assert_eq!(declared_budgets.cost, Some(serde_json::Number::from(30u64)));
    assert!(resolution.snapshot.verify().is_ok());
    assert!(resolution.evidence.verify().is_ok());
}

#[test]
fn future_dated_verified_at_is_refused() {
    let registry = registry(vec![local_decl()]);
    let mut c = candidate("route-future", Locality::Local, "harness-local", "v1", 1);
    // One second after evaluatedAt (2026-10-22T12:00:00Z)
    c.trust = trust("2026-10-22T12:00:01Z", 3600);
    let err = resolve_standard(vec![c], &registry, RouteProfile::Standard, None)
        .expect_err("future-dated verification refuses");
    assert!(format!("{err:#}").contains("later than evaluatedAt"));
}

#[test]
fn verified_at_exactly_evaluated_at_is_permitted() {
    let registry = registry(vec![local_decl()]);
    let mut c = candidate("route-now", Locality::Local, "harness-local", "v1", 1);
    c.trust = trust("2026-10-22T12:00:00Z", 3600);
    let resolution = resolve_standard(vec![c], &registry, RouteProfile::Standard, None)
        .expect("verifiedAt == evaluatedAt is permitted");
    assert_eq!(resolution.snapshot.selected.candidate_id, "route-now");
}

#[test]
fn ttl_seconds_u64_max_refuses_without_wrapping() {
    let registry = registry(vec![local_decl()]);
    let mut c = candidate("route-ttl-max", Locality::Local, "harness-local", "v1", 1);
    c.trust = trust("2026-10-22T00:00:00Z", u64::MAX);
    let err = resolve_standard(vec![c], &registry, RouteProfile::Standard, None)
        .expect_err("u64::MAX ttlSeconds refuses without wrapping");
    assert!(format!("{err:#}").contains("representable duration"));
}

#[test]
fn ttl_seconds_overflowing_the_instant_refuses() {
    let registry = registry(vec![local_decl()]);
    let mut c = candidate("route-ttl-over", Locality::Local, "harness-local", "v1", 1);
    c.trust = trust("2026-10-22T00:00:00Z", i64::MAX as u64);
    let err = resolve_standard(vec![c], &registry, RouteProfile::Standard, None)
        .expect_err("ttlSeconds whose addition overflows the instant refuses");
    assert!(format!("{err:#}").contains("representable chrono duration"));
}

fn budgets(tokens: u64) -> Budgets {
    serde_json::from_value(json!({ "tokens": tokens })).expect("budgets parse")
}

fn trust(verified_at: &str, ttl_seconds: u64) -> RouteTrust {
    RouteTrust {
        source: "operator-fixture".to_string(),
        verified_at: verified_at.to_string(),
        ttl_seconds,
    }
}

fn candidate(
    candidate_id: &str,
    provider_locality: prometheos_lite::workflow::soma::capability::Locality,
    harness_id: &str,
    revision: &str,
    priority: u32,
) -> RouteCandidateV1 {
    RouteCandidateV1 {
        schema_version: "1.0.0".to_string(),
        candidate_id: candidate_id.to_string(),
        provider_identity: format!("provider-{candidate_id}"),
        provider_locality,
        model: Some(format!("model-{candidate_id}")),
        harness_id: harness_id.to_string(),
        harness_revision: revision.to_string(),
        workspace: None,
        trust: trust("2026-10-22T00:00:00Z", 86_400),
        budget: None,
        priority,
        experimental: false,
    }
}

fn resolve_standard(
    candidates: Vec<RouteCandidateV1>,
    registry: &HarnessRegistry,
    request_profile: RouteProfile,
    pinned: Option<&str>,
) -> anyhow::Result<prometheos_lite::workflow::route_resolution::RouteResolution> {
    let requirements = requirements(&["test.capability"]);
    let authority = authority();
    let request = RouteResolutionRequest {
        route_set_id: "routes-main".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates,
        pinned_candidate_id: pinned,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: request_profile,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    resolve_route(&request, registry)
}

fn local_decl() -> HarnessDeclarationV1 {
    declaration(
        "harness-local",
        "local-harness",
        "v1",
        HarnessFamily::LocalModel,
        &["test.capability"],
        Some(prometheos_lite::workflow::soma::capability::Locality::Local),
        &[],
    )
}

fn remote_decl() -> HarnessDeclarationV1 {
    declaration(
        "harness-remote",
        "remote-harness",
        "v1",
        HarnessFamily::RemoteService,
        &["test.capability"],
        Some(Locality::Remote),
        &[],
    )
}

/// Local declaration claiming a runtime budget ceiling so work carrying
/// hard limits can negotiate compatibly (SOMA-AUTH-0009).
fn local_decl_with_budget_ceiling() -> HarnessDeclarationV1 {
    let mut caps = capability_set(
        "harness-local",
        "local-harness",
        "v1",
        &["test.capability"],
        Some(Locality::Local),
        &[],
    );
    caps.budget_ceilings = Some(budgets(10_000));
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: "harness-local".to_string(),
        identity: Identity {
            name: "local-harness".to_string(),
            revision: "v1".to_string(),
        },
        family: HarnessFamily::LocalModel,
        declared_capabilities: vec!["test.capability".to_string()],
        claimed_capabilities: vec!["test.capability".to_string()],
        capabilities: caps,
        content_digest: None,
    }
    .sealed()
}

/// Local declaration claiming a writable workspace boundary so work
/// demanding a write workspace can negotiate compatibly (SOMA-CAP-0005).
fn local_decl_with_workspace_write() -> HarnessDeclarationV1 {
    let mut caps = capability_set(
        "harness-local",
        "local-harness",
        "v1",
        &["test.capability"],
        Some(Locality::Local),
        &[],
    );
    caps.workspace = Some(Boundaries {
        workspace: Some(Boundary::Write),
        filesystem: None,
        git: None,
        process: None,
        network: None,
        secret: None,
    });
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: "harness-local".to_string(),
        identity: Identity {
            name: "local-harness".to_string(),
            revision: "v1".to_string(),
        },
        family: HarnessFamily::LocalModel,
        declared_capabilities: vec!["test.capability".to_string()],
        claimed_capabilities: vec!["test.capability".to_string()],
        capabilities: caps,
        content_digest: None,
    }
    .sealed()
}

use prometheos_lite::workflow::soma::capability::Locality;

#[test]
fn standard_profile_selects_highest_priority_eligible_route() {
    let registry = registry(vec![local_decl(), remote_decl()]);
    // Remote candidate has better priority but must not win under conservative;
    // under standard it may.
    let candidates = vec![
        candidate("route-a-local", Locality::Local, "harness-local", "v1", 10),
        candidate(
            "route-b-remote",
            Locality::Remote,
            "harness-remote",
            "v1",
            5,
        ),
    ];
    let resolution = resolve_standard(candidates, &registry, RouteProfile::Standard, None)
        .expect("standard profile resolves a compatible route");
    assert_eq!(resolution.snapshot.selected.candidate_id, "route-b-remote");
    assert_eq!(resolution.evidence.selected.candidate_id, "route-b-remote");
    assert_eq!(resolution.evidence.rejected.len(), 1);
    assert_eq!(
        resolution.evidence.rejected[0].candidate_id,
        "route-a-local"
    );
    assert_eq!(
        resolution.evidence.rejected[0].reasons,
        vec!["eligible but not selected: priority 10 > best eligible priority 5".to_string(),]
    );
    assert!(is_hex64(&resolution.evidence.snapshot_digest));
    assert!(resolution.evidence.verify().is_ok());
    assert!(resolution.snapshot.verify().is_ok());
}

#[test]
fn conservative_profile_restricts_to_local_providers() {
    let registry = registry(vec![local_decl(), remote_decl()]);
    let candidates = vec![
        candidate("route-a-local", Locality::Local, "harness-local", "v1", 10),
        candidate(
            "route-b-remote",
            Locality::Remote,
            "harness-remote",
            "v1",
            5,
        ),
    ];
    let resolution = resolve_standard(candidates, &registry, RouteProfile::Conservative, None)
        .expect("conservative profile resolves the local route");
    assert_eq!(resolution.snapshot.selected.candidate_id, "route-a-local");
    assert!(resolution.evidence.verify().is_ok());
    let rejection = &resolution.evidence.rejected[0];
    assert_eq!(rejection.candidate_id, "route-b-remote");
    assert!(rejection.reasons.iter().any(|r| r.contains("conservative")));
}

#[test]
fn experimental_profile_admits_experimental_candidate() {
    let registry = registry(vec![local_decl()]);
    let mut experimental = candidate("route-exp", Locality::Local, "harness-local", "v1", 1);
    experimental.experimental = true;
    let baseline = candidate("route-base", Locality::Local, "harness-local", "v1", 9);

    for profile in [RouteProfile::Conservative, RouteProfile::Standard] {
        let resolution = resolve_standard(
            vec![experimental.clone(), baseline.clone()],
            &registry,
            profile,
            None,
        )
        .expect("non-experimental baseline is selectable");
        assert_eq!(resolution.snapshot.selected.candidate_id, "route-base");
    }
    let resolution = resolve_standard(
        vec![experimental, baseline],
        &registry,
        RouteProfile::Experimental,
        None,
    )
    .expect("experimental profile admits the experimental candidate");
    assert_eq!(resolution.snapshot.selected.candidate_id, "route-exp");
}

#[test]
fn pinned_candidate_is_selected_no_fallback() {
    let registry = registry(vec![local_decl(), remote_decl()]);
    // The pinned candidate loses on priority; the resolver must honor the pin.
    let candidates = vec![
        candidate("route-a-local", Locality::Local, "harness-local", "v1", 10),
        candidate(
            "route-b-remote",
            Locality::Remote,
            "harness-remote",
            "v1",
            5,
        ),
    ];
    let resolution = resolve_standard(
        candidates,
        &registry,
        RouteProfile::Standard,
        Some("route-a-local"),
    )
    .expect("pin selects the pinned candidate");
    assert_eq!(resolution.snapshot.selected.candidate_id, "route-a-local");
    assert_eq!(
        resolution.evidence.pinned_candidate_id.as_deref(),
        Some("route-a-local")
    );

    // A pin naming an ineligible candidate must NOT fall back to another.
    let mut remote_exp = candidate(
        "route-b-remote",
        Locality::Remote,
        "harness-remote",
        "v1",
        5,
    );
    remote_exp.experimental = true;
    let err = resolve_standard(
        vec![
            candidate("route-a-local", Locality::Local, "harness-local", "v1", 10),
            remote_exp,
        ],
        &registry,
        RouteProfile::Standard,
        Some("route-b-remote"),
    )
    .expect_err("pinned experimental candidate under standard profile refuses without fallback");
    let message = format!("{err:#}");
    assert!(
        message.contains("ROUTE-0004"),
        "unexpected error: {message}"
    );

    // A pin naming an unknown candidate id refuses.
    let err = resolve_standard(
        vec![candidate(
            "route-a-local",
            Locality::Local,
            "harness-local",
            "v1",
            10,
        )],
        &registry,
        RouteProfile::Standard,
        Some("route-does-not-exist"),
    )
    .expect_err("unknown pin refuses");
    assert!(format!("{err:#}").contains("ROUTE-0004"));
}

#[test]
fn pinned_ineligible_pin_does_not_swap_in_replacement() {
    let registry = registry(vec![local_decl(), remote_decl()]);
    // Both candidates eligible; pin the remote one under standard: it must be
    // selected. Then make the pin unavailable and confirm we never silently
    // substitute even though another candidate is eligible.
    let candidates = vec![
        candidate("route-a-local", Locality::Local, "harness-local", "v1", 10),
        candidate(
            "route-b-remote",
            Locality::Remote,
            "harness-remote",
            "v1",
            5,
        ),
    ];
    let result = resolve_standard(
        candidates,
        &registry,
        RouteProfile::Conservative,
        Some("route-b-remote"),
    );
    assert!(
        result.is_err(),
        "pinned remote under conservative refuses; no substitution to local is allowed"
    );
}

#[test]
fn ambiguity_at_same_best_priority_refuses() {
    let registry = registry(vec![local_decl()]);
    let candidates = vec![
        candidate("route-a", Locality::Local, "harness-local", "v1", 5),
        candidate("route-b", Locality::Local, "harness-local", "v1", 5),
    ];
    let err = resolve_standard(candidates, &registry, RouteProfile::Conservative, None)
        .expect_err("two eligible candidates tied at best priority must refuse as ambiguous");
    let message = format!("{err:#}");
    assert!(
        message.contains("ROUTE-0005"),
        "unexpected error: {message}"
    );
    assert!(message.contains("route-a") && message.contains("route-b"));
}

#[test]
fn no_eligible_candidate_refuses_with_per_candidate_reasons() {
    let registry = registry(vec![local_decl()]);
    // Remote candidate under conservative profile: no eligible candidates.
    let candidates = vec![candidate(
        "route-r",
        Locality::Remote,
        "harness-local",
        "v1",
        1,
    )];
    let err = resolve_standard(candidates, &registry, RouteProfile::Conservative, None)
        .expect_err("no eligible candidate refuses");
    let message = format!("{err:#}");
    assert!(
        message.contains("ROUTE-0003"),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("route-r"),
        "diagnostic carries the candidate id: {message}"
    );
}

#[test]
fn empty_candidate_set_refuses() {
    let registry = registry(vec![local_decl()]);
    let err = resolve_standard(vec![], &registry, RouteProfile::Standard, None)
        .expect_err("empty set refuses");
    assert!(format!("{err:#}").contains("ROUTE-0001"));
}

#[test]
fn duplicate_candidate_ids_refuse() {
    let registry = registry(vec![local_decl()]);
    let candidates = vec![
        candidate("route-dup", Locality::Local, "harness-local", "v1", 1),
        candidate("route-dup", Locality::Local, "harness-local", "v1", 2),
    ];
    let err = resolve_standard(candidates, &registry, RouteProfile::Standard, None)
        .expect_err("duplicate candidate ids refuse");
    assert!(format!("{err:#}").contains("ROUTE-0002"));
}

#[test]
fn mismatched_harness_revision_refuses_candidate() {
    let registry = registry(vec![local_decl()]);
    let mut c = candidate("route-mismatch", Locality::Local, "harness-local", "v9", 1);
    c.provider_identity = "provider-mismatch".to_string();
    let err = resolve_standard(vec![c], &registry, RouteProfile::Standard, None)
        .expect_err("candidate pinned to an unpublished revision refuses");
    assert!(format!("{err:#}").contains("published revision"));
}

#[test]
fn stale_trust_metadata_refuses_candidate() {
    let registry = registry(vec![local_decl()]);
    // Route metadata verified at T expires at T+ttl; evaluation after expiry
    // must refuse (freshness is a ceiling, not a hint).
    let mut c = candidate("route-stale", Locality::Local, "harness-local", "v1", 1);
    c.trust = trust("2026-10-01T00:00:00Z", 3600); // expired at evaluation
    let err = resolve_standard(vec![c], &registry, RouteProfile::Standard, None)
        .expect_err("stale capability metadata refuses");
    assert!(format!("{err:#}").contains("stale"));
}

#[test]
fn fresh_trust_metadata_within_ttl_is_accepted() {
    let registry = registry(vec![local_decl()]);
    // verifiedAt + ttl must land strictly after evaluatedAt.
    let mut c = candidate("route-fresh", Locality::Local, "harness-local", "v1", 1);
    c.trust = trust("2026-10-22T11:59:59Z", 1); // expires 12:00:00Z == evaluatedAt -> stale
    let err = resolve_standard(vec![c.clone()], &registry, RouteProfile::Standard, None)
        .expect_err("expiry at exactly evaluatedAt refuses (boundary is stale)");
    assert!(format!("{err:#}").contains("stale"));

    c.trust = trust("2026-10-22T11:59:59Z", 2); // expires 12:00:01Z > evaluatedAt -> fresh
    let resolution = resolve_standard(vec![c], &registry, RouteProfile::Standard, None)
        .expect("metadata expiring after evaluatedAt is fresh");
    assert_eq!(resolution.snapshot.selected.candidate_id, "route-fresh");
}

#[test]
fn candidate_budget_must_fit_work_hard_limits() {
    let registry = registry(vec![local_decl_with_budget_ceiling()]);
    let requirements = requirements_with(json!({ "hardLimits": { "tokens": 100 } }));
    let authority = authority_with_budgets(1000);
    let mut over = candidate("route-over", Locality::Local, "harness-local", "v1", 1);
    over.budget = Some(budgets(200));
    let request = RouteResolutionRequest {
        route_set_id: "routes-budget".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![over],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    let err =
        resolve_route(&request, &registry).expect_err("candidate budget above hard limit refuses");
    let message = format!("{err:#}");
    assert!(
        message.contains("hard limit") || message.contains("tokens"),
        "unexpected error: {message}"
    );

    // Boundary: exactly at the limit must pass.
    let mut at_limit = candidate("route-at-limit", Locality::Local, "harness-local", "v1", 1);
    at_limit.budget = Some(budgets(100));
    let request = RouteResolutionRequest {
        route_set_id: "routes-budget".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![at_limit],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    resolve_route(&request, &registry).expect("budget exactly at hard limit is permitted");
}

#[test]
fn candidate_budget_without_work_hard_limits_refuses() {
    // No declared ceiling anywhere bounds the candidate's tokens dimension
    // (minimal authority, no runtime ceilings, no work hardLimits) -> refuse.
    let registry = registry(vec![local_decl()]);
    let mut undeclared = candidate("route-unbounded", Locality::Local, "harness-local", "v1", 1);
    undeclared.budget = Some(budgets(50));
    let requirements = requirements(&["test.capability"]); // no hardLimits
    let authority = authority();
    let request = RouteResolutionRequest {
        route_set_id: "routes-budget".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![undeclared],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    let err = resolve_route(&request, &registry)
        .expect_err("a candidate budget with no work hard limit is unverifiable and refuses");
    assert!(format!("{err:#}").contains("hardLimits"));
}

#[test]
fn required_capability_not_covered_refuses() {
    let registry = registry(vec![local_decl()]);
    // Requirement the declaration cannot cover.
    let requirements = requirements(&["capability.absent"]);
    let authority = authority();
    let c = candidate("route-uncovered", Locality::Local, "harness-local", "v1", 1);
    let request = RouteResolutionRequest {
        route_set_id: "routes-cap".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![c],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    let err =
        resolve_route(&request, &registry).expect_err("uncovered required capability refuses");
    assert!(format!("{err:#}").contains("ROUTE-0003"));
}

#[test]
fn work_locality_constraint_excludes_remote_candidate() {
    let registry = registry(vec![local_decl(), remote_decl()]);
    let requirements = requirements_with(json!({
        "locality": { "allowedExecution": ["local"] }
    }));
    let authority = authority();
    let remote = candidate("route-remote", Locality::Remote, "harness-remote", "v1", 1);
    let request = RouteResolutionRequest {
        route_set_id: "routes-loc".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![remote],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Experimental, // even experimental cannot override work locality
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    let err = resolve_route(&request, &registry)
        .expect_err("work locality constraints exclude the remote candidate");
    assert!(format!("{err:#}").contains("not permitted by work locality"));
}

#[test]
fn privacy_class_must_be_claimed_by_the_harness() {
    let registry_with_privacy = registry(vec![declaration(
        "harness-private",
        "private-harness",
        "v1",
        HarnessFamily::LocalModel,
        &["test.capability"],
        Some(Locality::Local),
        &["no-secrets"],
    )]);
    let requirements = requirements_with(json!({
        "locality": { "privacy": ["no-secrets"] }
    }));
    let authority = authority();
    let c = candidate("route-priv", Locality::Local, "harness-private", "v1", 1);
    let request = RouteResolutionRequest {
        route_set_id: "routes-priv".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![c],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    resolve_route(&request, &registry_with_privacy)
        .expect("privacy class claimed by the harness is compatible");

    let registry_missing = registry(vec![declaration(
        "harness-open",
        "open-harness",
        "v1",
        HarnessFamily::LocalModel,
        &["test.capability"],
        Some(Locality::Local),
        &[],
    )]);
    let c = candidate("route-open", Locality::Local, "harness-open", "v1", 1);
    let request = RouteResolutionRequest {
        route_set_id: "routes-priv".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![c],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    let err = resolve_route(&request, &registry_missing)
        .expect_err("a harness not claiming the required privacy class refuses");
    assert!(format!("{err:#}").contains("privacy class"));
}

#[test]
fn experimental_candidate_still_cannot_widen_authority() {
    // An experimental route whose effective authority widens the declared one
    // must fail closed — profile never grants authority.
    let registry = registry(vec![local_decl()]);
    let declared = serde_json::from_value(json!({
        "executionClass": "deterministic",
        "mutation": "none",
        "budgets": { "tokens": 10 },
    }))
    .unwrap();
    let effective = serde_json::from_value(json!({
        "executionClass": "deterministic",
        "mutation": "none",
        "budgets": { "tokens": 1000 },
    }))
    .unwrap();
    let mut exp = candidate("route-widen", Locality::Local, "harness-local", "v1", 1);
    exp.experimental = true;
    let requirements = requirements(&["test.capability"]);
    let request = RouteResolutionRequest {
        route_set_id: "routes-widen".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![exp],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &declared,
        authority_effective: &effective,
        profile: RouteProfile::Experimental,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    let err = resolve_route(&request, &registry)
        .expect_err("effective authority wider than declared must refuse even experimentally");
    let message = format!("{err:#}");
    assert!(
        message.contains("ROUTE-0003"),
        "unexpected error: {message}"
    );
}

#[test]
fn snapshot_maps_to_spec006_contract() {
    let registry = registry(vec![local_decl()]);
    let requirements = requirements(&["test.capability"]);
    let authority = authority_with_budgets(1000);
    let c = candidate("route-spec6", Locality::Local, "harness-local", "v1", 3);
    let mut c = c;
    c.model = Some("fixture-model".to_string());
    let request = RouteResolutionRequest {
        route_set_id: "routes-spec6".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![c],
        pinned_candidate_id: Some("route-spec6"),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    let resolution = resolve_route(&request, &registry).expect("route resolves");
    let snapshot = &resolution.snapshot;
    assert_eq!(snapshot.schema_version, EXECUTION_SNAPSHOT_SCHEMA_VERSION);
    assert_eq!(snapshot.snapshot_id, "routes-spec6/route-spec6");
    assert_eq!(snapshot.effective_profile.id, snapshot.snapshot_id);
    assert_eq!(
        snapshot.effective_profile.harness, "harness-local",
        "SPEC 006 harness identity is the selected harness"
    );
    assert_eq!(
        snapshot.effective_profile.provider.as_deref(),
        Some("provider-route-spec6")
    );
    assert_eq!(
        snapshot.effective_profile.model.as_deref(),
        Some("fixture-model")
    );
    // SPEC 006 invariants: effective authority and budgets do not exceed
    // declared ones.
    assert_eq!(
        snapshot.effective_profile.effective.capabilities,
        vec!["test.capability".to_string()]
    );
    assert_eq!(
        snapshot.effective_profile.declared.capabilities,
        vec!["test.capability".to_string()]
    );
    // Sealing and verification are self-contained.
    assert!(snapshot.verify().is_ok());
    resolution.evidence.verify().expect("evidence verifies");
}

#[test]
fn resolution_is_deterministic_across_invocations() {
    let build = || {
        let registry = registry(vec![local_decl(), remote_decl()]);
        let candidates = vec![
            candidate("route-a-local", Locality::Local, "harness-local", "v1", 10),
            candidate(
                "route-b-remote",
                Locality::Remote,
                "harness-remote",
                "v1",
                5,
            ),
        ];
        resolve_standard(candidates, &registry, RouteProfile::Standard, None)
            .expect("resolution succeeds")
    };
    let first = build();
    let second = build();
    assert_eq!(
        first.snapshot.content_digest,
        second.snapshot.content_digest
    );
    assert_eq!(
        first.evidence.content_digest,
        second.evidence.content_digest
    );
    assert_eq!(
        serde_json::to_value(&first.evidence).expect("evidence serializes"),
        serde_json::to_value(&second.evidence).expect("evidence serializes")
    );
}

#[test]
fn tampering_with_evidence_or_snapshot_fails_verification() {
    let registry = registry(vec![local_decl()]);
    let resolution = resolve_standard(
        vec![candidate(
            "route-t",
            Locality::Local,
            "harness-local",
            "v1",
            1,
        )],
        &registry,
        RouteProfile::Standard,
        None,
    )
    .expect("resolution succeeds");

    let mut tampered = resolution.evidence.clone();
    tampered.declaration_digest = "0".repeat(64);
    assert!(
        tampered.verify().is_err(),
        "rewritten declaration digest fails evidence verification"
    );

    let mut tampered = resolution.snapshot.clone();
    tampered.selected.candidate_id = "route-forged".to_string();
    assert!(
        tampered.verify().is_err(),
        "rewritten selection fails snapshot verification"
    );
}

#[test]
fn evidence_carries_no_raw_authority_or_secret_values() {
    let registry = registry(vec![local_decl()]);
    let authority = serde_json::from_value(json!({
        "executionClass": "deterministic",
        "mutation": "none",
        "readableScopes": ["repo://secret-scope"],
        "secrets": [{ "name": "secret-handle-0" }],
    }))
    .expect("authority with secrets parses");
    let requirements = requirements(&["test.capability"]);
    let c = candidate("route-redact", Locality::Local, "harness-local", "v1", 1);
    let request = RouteResolutionRequest {
        route_set_id: "routes-redact".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![c],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    let resolution = resolve_route(&request, &registry).expect("route resolves");
    let evidence_json = serde_json::to_string(&resolution.evidence).expect("evidence serializes");
    let snapshot_json = serde_json::to_string(&resolution.snapshot).expect("snapshot serializes");
    // The sealed route evidence binds identity and digests only — raw
    // authority envelope material must never appear in it.
    for forbidden in [
        "repo://secret-scope",
        "secret-handle-0",
        "\"executionClass\"",
        "\"readableScopes\"",
        "\"secrets\"",
    ] {
        assert!(
            !evidence_json.contains(forbidden),
            "raw authority material leaked into route evidence: {forbidden}"
        );
    }
    // The SPEC 006 snapshot intentionally embeds the authority pair (the
    // contract requires ProfileSide.authority verbatim); it seals and
    // verifies as a unit.
    assert!(snapshot_json.contains("executionClass"));
    assert!(resolution.snapshot.verify().is_ok());
    assert!(resolution.evidence.verify().is_ok());
    // Digest fields are lower hex sha256.
    assert!(is_hex64(&resolution.evidence.authority_declared_digest));
    assert!(is_hex64(&resolution.evidence.requirements_digest));
    assert!(is_hex64(&resolution.evidence.snapshot_digest));
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
            vec!["work://test".to_string()]
        } else {
            vec![]
        },
        resource_lock_id: format!("lock-{workspace_id}"),
        created_at: "2026-10-22T00:00:00Z".to_string(),
        content_digest: None,
    }
    .sealed()
}

#[test]
fn workspace_binding_must_match_the_actual_manifest() {
    let registry = registry(vec![local_decl_with_workspace_write()]);
    let requirements = requirements_with(json!({
        "workspace": { "workspace": "write" }
    }));
    let authority = authority();
    // Binding matches the manifest -> eligible.
    let mut matching = candidate("route-ws-ok", Locality::Local, "harness-local", "v1", 1);
    matching.workspace = Some(RouteWorkspaceBinding {
        adapter: AdapterKind::GitWorktree,
        revision: "lite.workspace.adapter.v1".to_string(),
    });
    let manifest = sealed_workspace_manifest("ws-match", WorkspaceMode::Writable);
    let request = RouteResolutionRequest {
        route_set_id: "routes-ws".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![matching],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: Some(&manifest),
    };
    let resolution =
        resolve_route(&request, &registry).expect("matching workspace binding resolves");
    assert!(resolution.snapshot.workspace.is_some());
    assert_eq!(
        resolution.snapshot.workspace.as_ref().unwrap().workspace_id,
        "ws-match"
    );

    // Binding mismatch -> refused (no substitution to another workspace).
    let mut mismatched = candidate("route-ws-bad", Locality::Local, "harness-local", "v1", 1);
    mismatched.workspace = Some(RouteWorkspaceBinding {
        adapter: AdapterKind::ExistingReadOnly,
        revision: "lite.workspace.adapter.v1".to_string(),
    });
    let request = RouteResolutionRequest {
        route_set_id: "routes-ws".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![mismatched],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: Some(&manifest),
    };
    let err = resolve_route(&request, &registry)
        .expect_err("workspace binding mismatch refuses the route");
    assert!(format!("{err:#}").contains("does not match candidate binding"));
}

#[test]
fn work_demanding_workspace_without_manifest_refuses() {
    let registry = registry(vec![local_decl()]);
    let requirements = requirements_with(json!({
        "workspace": { "workspace": "write" }
    }));
    let authority = authority();
    let c = candidate("route-no-ws", Locality::Local, "harness-local", "v1", 1);
    let request = RouteResolutionRequest {
        route_set_id: "routes-ws".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![c],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: None,
    };
    let err = resolve_route(&request, &registry)
        .expect_err("work demanding a workspace without one refuses");
    assert!(format!("{err:#}").contains("ROUTE-0006"));
}

#[test]
fn read_only_workspace_cannot_satisfy_write_boundary() {
    let registry = registry(vec![local_decl()]);
    let requirements = requirements_with(json!({
        "workspace": { "workspace": "write" }
    }));
    let authority = authority();
    let manifest = sealed_workspace_manifest("ws-readonly", WorkspaceMode::ReadOnly);
    let c = candidate("route-ro-ws", Locality::Local, "harness-local", "v1", 1);
    let request = RouteResolutionRequest {
        route_set_id: "routes-ws".to_string(),
        route_set_version: "1.0.0".to_string(),
        candidates: vec![c],
        pinned_candidate_id: None,
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        profile: RouteProfile::Standard,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        workspace: Some(&manifest),
    };
    let err = resolve_route(&request, &registry)
        .expect_err("read-only manifest cannot satisfy a write boundary");
    assert!(format!("{err:#}").contains("ROUTE-0006"));
}
