//! Harness selection through SPEC 007 capability negotiation (issue #154,
//! slice B).
//!
//! Slice A proved the seam is implementable twice and that declarations are
//! metadata, never authority. It left selection unbound: a caller could pick
//! any registered adapter by string and never negotiate its declared
//! `RuntimeCapabilitySet` against the work's requirements.
//!
//! This suite pins the missing binding:
//!
//! 1. selection runs the published negotiation
//!    (`WorkRequirements` + declaration → `CompatibilityDecision`) and refuses
//!    when the work's required capabilities are not covered;
//! 2. selection is EXPLICIT and revision-pinned — two compatible candidates
//!    are ambiguous, not silently ordered, and a pin that names no registered
//!    adapter is refused;
//! 3. the resulting durable evidence binds the harness id, the revision, the
//!    declaration digest and the decision, and it does not verify if any of
//!    those are altered;
//! 4. a substitution the work forbids is refused and emits no evidence;
//! 5. none of the refusals touch the adapter itself.

use anyhow::{Result, bail};

use prometheos_lite::workflow::execution_harness::{
    ExecutionHarness, HARNESS_SCHEMA_VERSION, HarnessDeclarationV1, HarnessDispatch, HarnessFamily,
    HarnessOutcome, HarnessRegistry, HarnessSelectionRequest, HarnessSelectionTrust,
    HarnessSelectionV1, HarnessStep,
};
use prometheos_lite::workflow::soma::capability::{
    DecisionStatus, Identity, RuntimeCapabilitySet, SUPPORTED_CAPABILITY_SCHEMA_VERSION,
    Substitution, WorkRequirements,
};
use prometheos_lite::workflow::soma::contracts::AuthorityProfile;
use prometheos_lite::workflow::soma::profile::{Boundaries, Boundary};
use prometheos_lite::workflow::workspace::{
    AdapterKind, WORKSPACE_SCHEMA_VERSION, WorkspaceManifestV1, WorkspaceMode,
};

const SELECTION_SCHEMA_VERSION: &str = "1.0.0";

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn canonical_digest<T: serde::Serialize>(value: &T) -> String {
    prometheos_lite::workflow::soma::try_canonical_digest(
        &serde_json::to_value(value).expect("value serializes"),
    )
    .expect("canonical digest succeeds")
}

fn authority() -> AuthorityProfile {
    serde_json::from_str(r#"{"executionClass":"deterministic","mutation":"none"}"#)
        .expect("minimal authority profile parses")
}

fn capability_set(
    harness_id: &str,
    name: &str,
    revision: &str,
    classes: &[&str],
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
        locality: None,
        privacy_classes: Vec::new(),
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
    capability: &str,
    classes: &[&str],
) -> HarnessDeclarationV1 {
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: harness_id.to_string(),
        identity: Identity {
            name: name.to_string(),
            revision: revision.to_string(),
        },
        family,
        declared_capabilities: vec![capability.to_string()],
        claimed_capabilities: vec![capability.to_string()],
        capabilities: capability_set(harness_id, name, revision, classes),
        content_digest: None,
    }
    .sealed()
}

/// An adapter that refuses to be invoked. Selection must never reach it.
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
        bail!("selection must never invoke the adapter")
    }
}

/// Instrumented harness that counts invocations to verify zero invocation on refusal.
#[derive(Clone)]
struct CountingHarness {
    declaration: HarnessDeclarationV1,
    invocations: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl CountingHarness {
    fn new(declaration: HarnessDeclarationV1) -> Self {
        Self {
            declaration,
            invocations: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    fn invocation_count(&self) -> usize {
        self.invocations.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl ExecutionHarness for CountingHarness {
    fn declaration(&self) -> &HarnessDeclarationV1 {
        &self.declaration
    }

    async fn run(
        &self,
        _step: HarnessStep,
        _dispatch: &mut HarnessDispatch<'_>,
    ) -> Result<HarnessOutcome> {
        self.invocations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        bail!("counting harness must not be invoked during selection")
    }
}

fn requirements(required: &[&str], forbid_substitution: bool) -> WorkRequirements {
    let mut json = serde_json::json!({
        "id": "reqs-selection",
        "schemaVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": false,
        "forbidSubstitution": forbid_substitution,
    });
    if !required.is_empty() {
        json["requiredCapabilities"] = serde_json::json!(required);
    }
    serde_json::from_value(json).expect("synthetic requirements parse")
}

fn request<'a>(
    requirements: &'a WorkRequirements,
    authority: &'a AuthorityProfile,
    substitutions: Vec<Substitution>,
    pinned_harness_id: Option<&'a str>,
) -> HarnessSelectionRequest<'a> {
    HarnessSelectionRequest {
        selection_id: "sel-1".to_string(),
        requirements,
        authority_declared: authority,
        authority_effective: authority,
        evaluated_at: "2026-10-22T12:00:00Z".to_string(),
        substitutions,
        pinned_harness_id,
        workspace: None,
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

/// Registry with a counting harness for zero-invocation verification.
fn counting_registry(declaration: HarnessDeclarationV1) -> (HarnessRegistry, CountingHarness) {
    let harness = CountingHarness::new(declaration);
    let mut registry = HarnessRegistry::new();
    registry
        .register(Box::new(harness.clone()))
        .expect("registration succeeds");
    (registry, harness)
}

/// Helper to create a sealed workspace manifest for testing.
fn sealed_workspace_manifest(
    workspace_id: &str,
    adapter: AdapterKind,
    mode: WorkspaceMode,
    base_revision: &str,
) -> WorkspaceManifestV1 {
    WorkspaceManifestV1 {
        schema_version: WORKSPACE_SCHEMA_VERSION.to_string(),
        workspace_id: workspace_id.to_string(),
        adapter,
        adapter_revision: "lite.workspace.adapter.v1".to_string(),
        repo_identity: "test-repo".to_string(),
        base_revision: base_revision.to_string(),
        branch: None,
        mode,
        writable_scopes: if matches!(mode, WorkspaceMode::Writable) {
            vec!["work://test".to_string()]
        } else {
            vec![]
        },
        resource_lock_id: format!("lock-{}", workspace_id),
        created_at: "2026-10-22T00:00:00Z".to_string(),
        content_digest: None,
    }
    .sealed()
}

/// Helper to create requirements with workspace boundary.
fn requirements_with_workspace(
    required: &[&str],
    forbid_substitution: bool,
    workspace_boundary: Option<Boundary>,
) -> WorkRequirements {
    let mut json = serde_json::json!({
        "id": "reqs-selection",
        "schemaVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "version": "1.0.0",
        "requiredSpecVersion": SUPPORTED_CAPABILITY_SCHEMA_VERSION,
        "degradationPermitted": false,
        "forbidSubstitution": forbid_substitution,
    });
    if !required.is_empty() {
        json["requiredCapabilities"] = serde_json::json!(required);
    }
    if let Some(boundary) = workspace_boundary {
        json["workspace"] = serde_json::json!({ "workspace": boundary });
    }
    serde_json::from_value(json).expect("synthetic requirements with workspace parse")
}

/// Helper to create a harness declaration with workspace adapter family.
fn declaration_with_workspace_adapter(
    harness_id: &str,
    name: &str,
    revision: &str,
    family: HarnessFamily,
    capability: &str,
    classes: &[&str],
    adapter_family: &str,
    adapter_revision: &str,
) -> HarnessDeclarationV1 {
    let mut caps = capability_set(harness_id, name, revision, classes);
    caps.adapter_families = vec![prometheos_lite::workflow::soma::capability::AdapterFamily {
        family: adapter_family.to_string(),
        revision: adapter_revision.to_string(),
    }];
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: harness_id.to_string(),
        identity: Identity {
            name: name.to_string(),
            revision: revision.to_string(),
        },
        family,
        declared_capabilities: vec![capability.to_string()],
        claimed_capabilities: vec![capability.to_string()],
        capabilities: caps,
        content_digest: None,
    }
    .sealed()
}

/// Harness declaration with workspace adapter family and declared workspace
/// boundary (so the SPEC 007 resolver reproduces a Compatible decision under
/// workspace-bearing requirements).
fn declaration_with_workspace_adapter_and_boundary(
    harness_id: &str,
    name: &str,
    revision: &str,
    family: HarnessFamily,
    capability: &str,
    classes: &[&str],
    adapter_family: &str,
    adapter_revision: &str,
    workspace_boundary: Boundaries,
) -> HarnessDeclarationV1 {
    let mut caps = capability_set(harness_id, name, revision, classes);
    caps.adapter_families = vec![prometheos_lite::workflow::soma::capability::AdapterFamily {
        family: adapter_family.to_string(),
        revision: adapter_revision.to_string(),
    }];
    caps.workspace = Some(workspace_boundary);
    HarnessDeclarationV1 {
        schema_version: HARNESS_SCHEMA_VERSION.to_string(),
        harness_id: harness_id.to_string(),
        identity: Identity {
            name: name.to_string(),
            revision: revision.to_string(),
        },
        family,
        declared_capabilities: vec![capability.to_string()],
        claimed_capabilities: vec![capability.to_string()],
        capabilities: caps,
        content_digest: None,
    }
    .sealed()
}

/// Workspace evidence built from a manifest exactly as the verifier expects.
fn workspace_evidence_for(
    manifest: &WorkspaceManifestV1,
) -> prometheos_lite::workflow::execution_harness::WorkspaceEvidenceV1 {
    prometheos_lite::workflow::execution_harness::WorkspaceEvidenceV1 {
        workspace_id: manifest.workspace_id.clone(),
        adapter: match manifest.adapter {
            AdapterKind::GitWorktree => "git-worktree".to_string(),
            AdapterKind::ExistingReadOnly => "existing-read-only".to_string(),
        },
        adapter_revision: manifest.adapter_revision.clone(),
        mode: match manifest.mode {
            WorkspaceMode::ReadOnly => "read".to_string(),
            WorkspaceMode::Writable => "write".to_string(),
        },
        manifest_digest: manifest.compute_digest(),
    }
}

/// T1 — the compatible candidate is selected and the evidence binds the
/// harness identity, its revision, its declaration digest and the decision.
#[test]
fn compatible_harness_selection_records_revision_and_decision() {
    let registered = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
    );
    let expected_declaration_digest = registered.compute_digest();
    let registry = registry(vec![registered]);
    let requirements = requirements(&["process-exec"], true);
    let authority = authority();

    let selection = registry
        .select(&request(&requirements, &authority, Vec::new(), None))
        .expect("a covered requirement selects its harness");

    assert_eq!(selection.schema_version, SELECTION_SCHEMA_VERSION);
    assert_eq!(selection.harness_id, "lite.harness.native");
    assert_eq!(selection.identity.name, "lite-native");
    assert_eq!(selection.identity.revision, "v1");
    assert_eq!(selection.family, HarnessFamily::Native);
    assert_eq!(selection.declaration_digest, expected_declaration_digest);
    assert_eq!(selection.decision_status, DecisionStatus::Compatible);
    assert!(!selection.decision_id.is_empty());
    assert!(is_hex64(&selection.capability_set_digest));
    assert!(is_hex64(&selection.requirements_digest));
    assert!(selection.verify().is_ok(), "fresh evidence verifies");

    // Evidence round-trips: a durable record is re-readable.
    let json = selection.to_json().expect("evidence serializes");
    let reparsed = HarnessSelectionV1::parse_json(&json).expect("evidence re-parses");
    assert_eq!(reparsed, selection);
}

/// T2 — a required capability the adapter does not declare fails closed
/// before execution, and no selection evidence exists to act on.
#[test]
fn uncovered_required_capability_refuses_selection() {
    let registered = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
    );
    let registry = registry(vec![registered]);
    let requirements = requirements(&["fs.write"], true);
    let authority = authority();

    let error = registry
        .select(&request(&requirements, &authority, Vec::new(), None))
        .expect_err("an uncovered requirement cannot select a harness");
    assert!(
        error.to_string().contains("HSEL-0003"),
        "unexpected refusal: {error}"
    );
}

/// T3 — two compatible candidates are ambiguous. Ordering is not authority:
/// the caller must pin the harness it wants, and the pin is what gets
/// recorded.
#[test]
fn two_compatible_candidates_require_an_explicit_pin() {
    let registry = registry(vec![
        declaration(
            "lite.harness.alpha",
            "lite-alpha",
            "v1",
            HarnessFamily::Native,
            "harness.native.probe",
            &["process-exec"],
        ),
        declaration(
            "lite.harness.beta",
            "lite-beta",
            "v2",
            HarnessFamily::Subprocess,
            "harness.subprocess.probe",
            &["process-exec"],
        ),
    ]);
    let requirements = requirements(&["process-exec"], true);
    let authority = authority();

    let error = registry
        .select(&request(&requirements, &authority, Vec::new(), None))
        .expect_err("an unpinned ambiguity is refused, not resolved by order");
    assert!(
        error.to_string().contains("HSEL-0004"),
        "unexpected refusal: {error}"
    );

    let pinned = registry
        .select(&request(
            &requirements,
            &authority,
            Vec::new(),
            Some("lite.harness.beta"),
        ))
        .expect("the pinned candidate is selected");
    assert_eq!(pinned.harness_id, "lite.harness.beta");
    assert_eq!(pinned.identity.revision, "v2");
    assert_eq!(pinned.family, HarnessFamily::Subprocess);
}

/// T4 — a pin that names no registered adapter is refused. A pin is a
/// requirement, never a hint.
#[test]
fn pinned_harness_must_be_registered() {
    let registry = registry(vec![declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
    )]);
    let requirements = requirements(&["process-exec"], true);
    let authority = authority();

    let error = registry
        .select(&request(
            &requirements,
            &authority,
            Vec::new(),
            Some("lite.harness.absent"),
        ))
        .expect_err("an unregistered pin cannot be satisfied");
    assert!(
        error.to_string().contains("HSEL-0002"),
        "unexpected refusal: {error}"
    );
}

/// T5 — a substitution the work forbids is refused by the negotiation, and
/// selection emits no evidence for it.
#[test]
fn forbidden_substitution_emits_no_selection_evidence() {
    let registry = registry(vec![declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
    )]);
    let requirements = requirements(&["process-exec"], true);
    let authority = authority();
    let substitutions = vec![Substitution {
        from: "lite.harness.other".to_string(),
        to: "lite.harness.native".to_string(),
        reason: Some("cheaper".to_string()),
    }];

    let error = registry
        .select(&request(&requirements, &authority, substitutions, None))
        .expect_err("a forbidden substitution is refused");
    assert!(
        error.to_string().contains("SOMA-CAP-0004"),
        "unexpected refusal: {error}"
    );
}

/// T6 — the evidence is self-verifying: altering the recorded revision or the
/// recorded declaration digest invalidates it.
#[test]
fn tampered_selection_evidence_does_not_verify() {
    let registry = registry(vec![declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
    )]);
    let requirements = requirements(&["process-exec"], true);
    let authority = authority();
    let selection = registry
        .select(&request(&requirements, &authority, Vec::new(), None))
        .expect("selection succeeds");

    let mut revision_tampered = selection.clone();
    revision_tampered.identity.revision = "v2".to_string();
    assert!(
        revision_tampered.verify().is_err(),
        "a rewritten revision must not verify"
    );

    let mut digest_tampered = selection.clone();
    digest_tampered.declaration_digest = "0".repeat(64);
    assert!(
        digest_tampered.verify().is_err(),
        "a rewritten declaration digest must not verify"
    );
}

/// P2 regression — selection evidence binds authority and evaluated_at.
///
/// Two selections with the same selection id, requirements and harness but
/// different authority snapshots or evaluation instants must produce
/// distinguishable, independently verifiable evidence. The current contract
/// omits the authority and evaluated_at bindings, so a silent substitution of
/// authority or time cannot be detected.
#[test]
fn selection_evidence_binds_authority_and_evaluated_at() {
    let registry = registry(vec![declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
    )]);
    let requirements = requirements(&["process-exec"], true);

    let authority_a = authority();
    let authority_b =
        serde_json::from_str(r#"{"executionClass":"deterministic","mutation":"explicit"}"#)
            .expect("different authority parses");

    let evaluated_at_a = "2026-10-22T12:00:00Z".to_string();
    let evaluated_at_b = "2026-10-22T13:00:00Z".to_string();

    let selection_a = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-bind".to_string(),
            requirements: &requirements,
            authority_declared: &authority_a,
            authority_effective: &authority_a,
            evaluated_at: evaluated_at_a.clone(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: None,
        })
        .expect("first selection succeeds");

    let selection_b = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-bind".to_string(),
            requirements: &requirements,
            authority_declared: &authority_b,
            authority_effective: &authority_b,
            evaluated_at: evaluated_at_b.clone(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: None,
        })
        .expect("second selection succeeds");

    // The evidence must be distinguishable. With the current contract it is
    // not, so this assertion fails until the contract records the negotiation
    // basis.
    assert_ne!(
        selection_a, selection_b,
        "selections with different authority/evaluated_at must produce distinct evidence"
    );
}

/// Re-seal a selection after rewriting a field, so a test can isolate a
/// semantic rule from the content-digest guard.
fn resealed(
    selection: &HarnessSelectionV1,
    mutate: impl FnOnce(&mut HarnessSelectionV1),
) -> HarnessSelectionV1 {
    let mut tampered = selection.clone();
    tampered.content_digest = None;
    mutate(&mut tampered);
    tampered
        .try_sealed()
        .expect("Lite-constructed evidence re-seals")
}

/// P1 regression — `evaluatedAt` is parsed as RFC 3339, not merely shaped.
///
/// Positional character checks accepted both a swapped-separator value such as
/// `2026:10:22-12-00-00Z` and an out-of-range calendar value such as
/// `2026-99-99T99:99:99Z`. Both must be refused even when the evidence is
/// re-sealed, and a valid leap day must be accepted.
#[test]
fn evaluated_at_must_be_a_valid_rfc3339_utc_instant() {
    let registry = registry(vec![declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
    )]);
    let requirements = requirements(&["process-exec"], true);
    let authority = authority();
    let selection = registry
        .select(&request(&requirements, &authority, Vec::new(), None))
        .expect("selection succeeds");

    let malformed = [
        // Separator confusion: the shape is right, the positions are not.
        "2026:10:22-12-00-00Z",
        "2026-10-22-12:00:00Z",
        "2026-10-22T12-00-00Z",
        "2026/10/22T12:00:00Z",
        "2026-10-22T12:00:00z",
        // Calendar values the positional check could not see.
        "2026-99-99T99:99:99Z",
        "2026-13-01T12:00:00Z",
        "2026-00-01T12:00:00Z",
        "2026-02-30T12:00:00Z",
        "2026-04-31T12:00:00Z",
        "2025-02-29T12:00:00Z",
        // Clock ranges.
        "2026-10-22T24:00:00Z",
        "2026-10-22T12:60:00Z",
        "2026-10-22T12:00:60Z",
        // Not UTC-Z: an offset is a different instant contract.
        "2026-10-22T12:00:00+00:00",
        "2026-10-22T12:00:00-03:00",
        // Empty and truncated.
        "",
        "2026-10-22",
    ];
    for value in malformed {
        let tampered = resealed(&selection, |evidence| {
            evidence.evaluated_at = value.to_string();
        });
        assert!(
            tampered.verify().is_err(),
            "evaluatedAt {value:?} must not verify"
        );
    }

    for value in [
        "2024-02-29T00:00:00Z",
        "2000-02-29T23:59:59Z",
        "2026-10-22T12:00:00Z",
    ] {
        let accepted = resealed(&selection, |evidence| {
            evidence.evaluated_at = value.to_string();
        });
        assert!(
            accepted.verify().is_ok(),
            "evaluatedAt {value:?} must verify"
        );
    }
}

/// P1 regression — integrity is not semantic validity.
///
/// Re-sealing substituted evidence keeps `verify()` green because the bytes are
/// internally consistent, but `verify_against` re-derives every negotiation
/// input from trusted values and re-runs the resolver, so the substitution
/// fails closed.
#[test]
fn semantic_verification_detects_substituted_inputs() {
    let registered = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
    );
    let registry = registry(vec![registered]);
    let reqs = requirements(&["process-exec"], true);
    let authority_a = authority();
    let authority_b: AuthorityProfile =
        serde_json::from_str(r#"{"executionClass":"deterministic","mutation":"explicit"}"#)
            .expect("different authority parses");

    let selection = registry
        .select(&request(&reqs, &authority_a, Vec::new(), None))
        .expect("selection succeeds");

    let harness = registry
        .get("lite.harness.native")
        .expect("registered harness is retrievable");

    let trust = HarnessSelectionTrust {
        harness,
        requirements: &reqs,
        authority_declared: &authority_a,
        authority_effective: &authority_a,
        selection_id: "sel-1",
        evaluated_at: "2026-10-22T12:00:00Z",
        substitutions: &[],
        workspace: None,
    };
    selection
        .verify_against(&trust)
        .expect("matching trusted inputs verify semantically");

    // A substituted effective authority, re-sealed to stay internally
    // consistent, passes integrity and fails semantics.
    let substituted = resealed(&selection, |evidence| {
        evidence.authority_effective_digest = canonical_digest(&authority_b);
    });
    assert!(
        substituted.verify().is_ok(),
        "integrity alone cannot see the substitution"
    );
    let error = substituted
        .verify_against(&trust)
        .expect_err("a substituted authority must not verify semantically");
    assert!(
        error.to_string().contains("HSEL-0008"),
        "unexpected semantic refusal: {error}"
    );

    // Trusted inputs that no longer match the recorded requirements.
    let stricter = requirements(&["fs.write"], true);
    let error = selection
        .verify_against(&HarnessSelectionTrust {
            harness,
            requirements: &stricter,
            authority_declared: &authority_a,
            authority_effective: &authority_a,
            selection_id: "sel-1",
            evaluated_at: "2026-10-22T12:00:00Z",
            substitutions: &[],
            workspace: None,
        })
        .expect_err("different trusted requirements must not verify");
    assert!(
        error.to_string().contains("HSEL-0008"),
        "unexpected semantic refusal: {error}"
    );

    // A different harness revision claiming the same id is not the harness
    // the evidence was bound to.
    let other = ProbeHarness::new(declaration(
        "lite.harness.native",
        "lite-native",
        "v2",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
    ));
    let error = selection
        .verify_against(&HarnessSelectionTrust {
            harness: &other,
            requirements: &reqs,
            authority_declared: &authority_a,
            authority_effective: &authority_a,
            selection_id: "sel-1",
            evaluated_at: "2026-10-22T12:00:00Z",
            substitutions: &[],
            workspace: None,
        })
        .expect_err("a different harness revision must not verify");
    assert!(
        error.to_string().contains("HSEL-0008"),
        "unexpected semantic refusal: {error}"
    );

    // A re-sealed but forged decision digest is refused by the resolver
    // replay, not just by the content digest.
    let forged = resealed(&selection, |evidence| {
        evidence.decision_digest = "0".repeat(64);
    });
    assert!(
        forged.verify().is_ok(),
        "integrity alone cannot see the forgery"
    );
    let error = forged
        .verify_against(&trust)
        .expect_err("a forged decision digest must not verify semantically");
    assert!(
        error.to_string().contains("HSEL-0008"),
        "unexpected semantic refusal: {error}"
    );
}

/// P2 regression — `verify_against` must reject forged-but-resealed evidence.
///
/// A modified selection_id, evaluated_at, substitutions, or decision_id can
/// keep the content digest internally consistent, so `verify()` will pass.
/// Semantic verification must still fail with HSEL-0008.
#[test]
fn semantic_verification_rejects_forged_fields() {
    let declaration = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
    );
    let registry = registry(vec![declaration]);
    let reqs = requirements(&["process-exec"], true);
    let authority = authority();

    let selection = registry
        .select(&request(&reqs, &authority, Vec::new(), None))
        .expect("selection succeeds");

    let harness = registry
        .get("lite.harness.native")
        .expect("registered harness is retrievable");

    let trust = HarnessSelectionTrust {
        harness,
        requirements: &reqs,
        authority_declared: &authority,
        authority_effective: &authority,
        selection_id: "sel-1",
        evaluated_at: "2026-10-22T12:00:00Z",
        substitutions: &[],
        workspace: None,
    };

    // Baseline: matching trusted inputs verify semantically
    selection
        .verify_against(&trust)
        .expect("matching trusted inputs verify semantically");

    // Helper to reseal and test
    let test_forgery = |name: &str, mutate: fn(&mut HarnessSelectionV1)| {
        let forged = resealed(&selection, |evidence| {
            mutate(evidence);
        });
        // Integrity alone cannot detect the forgery
        assert!(
            forged.verify().is_ok(),
            "integrity verification should pass for resealed evidence"
        );
        // Semantic verification must fail
        let err = forged.verify_against(&trust).expect_err(&format!(
            "forged {name} evidence must fail semantic verification"
        ));
        assert!(
            err.to_string().contains("HSEL-0008"),
            "expected HSEL-0008 for {name}, got: {err}"
        );
    };

    // Forged selection_id
    test_forgery("selection_id", |e| {
        e.selection_id = "forged-sel".to_string();
    });

    // Forged evaluated_at
    test_forgery("evaluated_at", |e| {
        e.evaluated_at = "2026-10-22T13:00:00Z".to_string();
    });

    // Forged substitutions
    test_forgery("substitutions", |e| {
        e.substitutions = vec![Substitution {
            from: "process-exec".to_string(),
            to: "forged".to_string(),
            reason: None,
        }];
    });

    // Forged decision_id
    test_forgery("decision_id", |e| {
        e.decision_id = "forged/dec".to_string();
    });
}

// P1-C — NEW D1 Conformance Tests
// These test production HarnessRegistry::select with workspace compatibility enforcement.

/// T1 — missing required workspace causes refusal before execution
#[test]
fn missing_required_workspace_refuses_selection() {
    let registry = registry(vec![declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    )]);

    // Work requirements demand a workspace boundary (read).
    let requirements = requirements_with_workspace(&["process-exec"], true, Some(Boundary::Read));

    let authority = authority();
    let error = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-req-ws".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: None,
        })
        .expect_err("missing required workspace cannot select a harness");

    assert!(
        error.to_string().contains("HSEL-0009"),
        "expected HSEL-0009 for missing required workspace, got: {error}"
    );
}

/// T2 — read-only workspace cannot satisfy writable requirements
#[test]
fn readonly_workspace_rejected_for_writable_requirements() {
    let workspace_manifest = sealed_workspace_manifest(
        "ws-ro",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );
    let registry = registry(vec![declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    )]);

    // Work requirements demand a writable workspace, but manifest is read-only.
    let requirements = requirements_with_workspace(&["process-exec"], true, Some(Boundary::Write));

    let authority = authority();
    let error = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-readonly-writable".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&workspace_manifest),
        })
        .expect_err("read-only workspace cannot satisfy writable requirements");

    assert!(
        error.to_string().contains("HSEL-0009"),
        "expected HSEL-0009 for read-only workspace with writable requirement, got: {error}"
    );
}

/// T3 — adapter family mismatch refuses before execution
#[test]
fn adapter_family_mismatch_refuses_selection() {
    let workspace_manifest = sealed_workspace_manifest(
        "ws-ro",
        AdapterKind::GitWorktree,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );
    let registry = registry(vec![declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only", // mismatch: git-worktree vs existing-read-only
        "lite.workspace.adapter.v1",
    )]);

    let requirements = requirements_with_workspace(&["process-exec"], true, Some(Boundary::Read));

    let authority = authority();
    let error = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-family-mismatch".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&workspace_manifest),
        })
        .expect_err("adapter family mismatch refuses before execution");

    assert!(
        error.to_string().contains("HSEL-0003"),
        "expected HSEL-0003 for adapter family mismatch, got: {error}"
    );
}

/// T4 — adapter revision mismatch refuses before execution
#[test]
fn adapter_revision_mismatch_refuses_selection() {
    let workspace_manifest = sealed_workspace_manifest(
        "ws-ro",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );
    let registry = registry(vec![declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "v2", // mismatch: revision v2 vs expected v1
    )]);

    let requirements = requirements_with_workspace(&["process-exec"], true, Some(Boundary::Read));

    let authority = authority();
    let error = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-rev-mismatch".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&workspace_manifest),
        })
        .expect_err("adapter revision mismatch refuses before execution");

    assert!(
        error.to_string().contains("HSEL-0003"),
        "expected HSEL-0003 for adapter revision mismatch, got: {error}"
    );
}

/// T5 — unsupported workspace schema refused
#[test]
fn unsupported_workspace_schema_refuses_selection() {
    let workspace_manifest = WorkspaceManifestV1 {
        schema_version: "2.0.0".to_string(), // unsupported
        workspace_id: "ws-bad".to_string(),
        adapter: AdapterKind::ExistingReadOnly,
        adapter_revision: "lite.workspace.adapter.v1".to_string(),
        repo_identity: "test-repo".to_string(),
        base_revision: "abc".repeat(40),
        branch: None,
        mode: WorkspaceMode::ReadOnly,
        writable_scopes: vec![],
        resource_lock_id: "lock-ws-bad".to_string(),
        created_at: "2026-10-22T00:00:00Z".to_string(),
        content_digest: None,
    };

    let registry = registry(vec![declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    )]);

    let requirements = requirements_with_workspace(&["process-exec"], true, Some(Boundary::Read));

    let authority = authority();
    let error = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-bad-schema".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&workspace_manifest),
        })
        .expect_err("unsupported workspace schema refused");

    assert!(
        error.to_string().contains("HSEL-0009"),
        "expected HSEL-0009 for unsupported schema, got: {error}"
    );
}

/// T6 — tampered manifest digest refused
#[test]
fn tampered_manifest_digest_refuses_selection() {
    let mut workspace_manifest = sealed_workspace_manifest(
        "ws-tampered",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );
    // Tamper with the content digest
    workspace_manifest.content_digest = Some("0".repeat(64));

    let registry = registry(vec![declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    )]);

    let requirements = requirements_with_workspace(&["process-exec"], true, Some(Boundary::Read));

    let authority = authority();
    let error = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-tampered-digest".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&workspace_manifest),
        })
        .expect_err("tampered manifest digest refused");

    assert!(
        error.to_string().contains("HSEL-0009"),
        "expected HSEL-0009 for tampered manifest, got: {error}"
    );
}

/// T7 — forged workspace evidence refused by verify_against
#[test]
fn forged_workspace_evidence_refused() {
    let registry = registry(vec![declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    )]);

    let requirements = requirements_with_workspace(&["process-exec"], true, None);
    let authority = authority();

    let selection = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-forged-ws".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: None,
        })
        .expect("selection without workspace succeeds");

    // Sealed evidence with forged workspace evidence.
    let mut forged = selection.clone();
    forged.content_digest = None;
    forged.workspace = Some(
        prometheos_lite::workflow::execution_harness::WorkspaceEvidenceV1 {
            workspace_id: "forged-ws".to_string(),
            adapter: "existing-read-only".to_string(),
            adapter_revision: "v1".to_string(),
            mode: "read".to_string(),
            manifest_digest: "0".repeat(64),
        },
    );
    forged = forged.try_sealed().expect("forge seals");

    let authority_a = authority.clone();
    let trust = prometheos_lite::workflow::execution_harness::HarnessSelectionTrust {
        harness: registry.get("lite.harness.native").expect("harness"),
        requirements: &requirements,
        authority_declared: &authority_a,
        authority_effective: &authority_a,
        selection_id: "sel-forged-ws",
        evaluated_at: "2026-10-22T12:00:00Z",
        substitutions: &[],
        workspace: None, // trusted has no workspace
    };

    let error = forged
        .verify_against(&trust)
        .expect_err("forged workspace evidence refused");
    assert!(
        error.to_string().contains("HSEL-0008"),
        "expected HSEL-0008 for forged workspace, got: {error}"
    );
}

/// T8 — trusted workspace mismatch refused
#[test]
fn trusted_workspace_mismatch_refused() {
    let registry = registry(vec![declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    )]);

    let requirements = requirements_with_workspace(&["process-exec"], true, None);
    let authority = authority();

    let selection = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-trust-mismatch".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: None,
        })
        .expect("selection without workspace succeeds");

    // Matching sealed workspace.
    let workspace_manifest = sealed_workspace_manifest(
        "ws-trusted",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );

    // Build trust with a DIFFERENT workspace manifest (different workspace_id).
    let trust = prometheos_lite::workflow::execution_harness::HarnessSelectionTrust {
        harness: registry.get("lite.harness.native").expect("harness"),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        selection_id: "sel-trust-mismatch",
        evaluated_at: "2026-10-22T12:00:00Z",
        substitutions: &[],
        workspace: Some(&workspace_manifest), // different ws_id
    };

    let error = selection
        .verify_against(&trust)
        .expect_err("trusted workspace mismatch refused");
    assert!(
        error.to_string().contains("HSEL-0008"),
        "expected HSEL-0008 for trusted workspace mismatch, got: {error}"
    );
}

/// T9 — attempted authority widening is refused (with real sealed workspace)
#[test]
fn attempted_authority_widening_refused() {
    let registry = registry(vec![declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    )]);

    let ws_manifest = sealed_workspace_manifest(
        "ws-authority",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );

    let requirements = requirements_with_workspace(&["process-exec"], true, Some(Boundary::Read));
    let declared = authority();
    let hostile: AuthorityProfile = serde_json::from_str(
        r#"{"executionClass":"open-ended","mutation":"explicit","readableScopes":["*"],"writableScopes":["*"]}"#,
    )
    .expect("hostile profile parses");

    let error = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-authority-widening".to_string(),
            requirements: &requirements,
            authority_declared: &declared,
            authority_effective: &hostile,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&ws_manifest),
        })
        .expect_err("authority widening refused");

    assert!(
        error.to_string().contains("HSEL-0003"),
        "expected HSEL-0003 for authority widening, got: {error}"
    );
}

/// T10 — zero harness invocation on all refusal paths (instrumented)
#[test]
fn zero_harness_invocation_on_refusal() {
    let decl = declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    );
    let (registry, harness) = counting_registry(decl);

    let authority = authority();

    // Test 1: missing required workspace
    let requirements = requirements_with_workspace(&["process-exec"], true, Some(Boundary::Read));
    let error = registry
        .select(&HarnessSelectionRequest {
            selection_id: "zero-refusal-1".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: None,
        })
        .expect_err("missing required workspace refused");

    assert!(
        error.to_string().contains("HSEL-0009"),
        "expected workspace refusal, got: {error}"
    );
    assert_eq!(
        harness.invocation_count(),
        0,
        "harness must not be invoked on missing workspace refusal"
    );

    // Test 2: read-only workspace with writable requirement
    let ws_manifest = sealed_workspace_manifest(
        "ws-test",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );
    let requirements2 = requirements_with_workspace(&["process-exec"], true, Some(Boundary::Write));
    let error2 = registry
        .select(&HarnessSelectionRequest {
            selection_id: "zero-refusal-2".to_string(),
            requirements: &requirements2,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&ws_manifest),
        })
        .expect_err("readonly workspace refusal");

    assert!(
        error2.to_string().contains("HSEL-0009"),
        "expected workspace refusal, got: {error2}"
    );
    assert_eq!(
        harness.invocation_count(),
        0,
        "harness must not be invoked on mode mismatch refusal"
    );

    // Test 3: adapter family mismatch
    let ws_manifest2 = sealed_workspace_manifest(
        "ws-test2",
        AdapterKind::GitWorktree,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );
    let requirements3 = requirements_with_workspace(&["process-exec"], true, Some(Boundary::Read));
    let error3 = registry
        .select(&HarnessSelectionRequest {
            selection_id: "zero-refusal-3".to_string(),
            requirements: &requirements3,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&ws_manifest2),
        })
        .expect_err("adapter family mismatch refusal");

    assert!(
        error3.to_string().contains("HSEL-0003"),
        "expected adapter family refusal, got: {error3}"
    );
    assert_eq!(
        harness.invocation_count(),
        0,
        "harness must not be invoked on adapter family mismatch"
    );

    // Test 4: adapter revision mismatch
    let ws_manifest3 = sealed_workspace_manifest(
        "ws-test3",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );
    let decl_rev = declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "v2", // mismatch
    );
    let (registry_rev, harness_rev) = counting_registry(decl_rev);
    let error4 = registry_rev
        .select(&HarnessSelectionRequest {
            selection_id: "zero-refusal-4".to_string(),
            requirements: &requirements3,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&ws_manifest3),
        })
        .expect_err("adapter revision mismatch refusal");

    assert!(
        error4.to_string().contains("HSEL-0003"),
        "expected adapter revision refusal, got: {error4}"
    );
    assert_eq!(
        harness_rev.invocation_count(),
        0,
        "harness must not be invoked on adapter revision mismatch"
    );
}

/// T11 — sealed workspace selection succeeds (requirements without boundary, workspace provided)
#[test]
fn sealed_workspace_selection_succeeds() {
    let decl = declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    );
    let registry = registry(vec![decl]);

    let ws_manifest = sealed_workspace_manifest(
        "ws-valid",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );

    // No workspace boundary in requirements — workspace provided as evidence only.
    let requirements = requirements_with_workspace(&["process-exec"], true, None);
    let authority = authority();

    let selection = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-sealed-ws".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&ws_manifest),
        })
        .expect("sealed workspace selection succeeds");

    assert_eq!(selection.decision_status, DecisionStatus::Compatible);
    assert!(selection.workspace.is_some());
    assert_eq!(
        selection.workspace.as_ref().unwrap().workspace_id,
        "ws-valid"
    );
}

/// T12 — matching trusted workspace passes verify_against (no boundary in requirements)
#[test]
fn matching_trusted_workspace_passes_verify_against() {
    let decl = declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    );
    let registry = registry(vec![decl]);

    let ws_manifest = sealed_workspace_manifest(
        "ws-match",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );

    // No workspace boundary in requirements — workspace provided as evidence only.
    let requirements = requirements_with_workspace(&["process-exec"], true, None);
    let authority = authority();

    let selection = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-verify-match".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&ws_manifest),
        })
        .expect("selection with workspace succeeds");

    let trust = prometheos_lite::workflow::execution_harness::HarnessSelectionTrust {
        harness: registry.get("lite.harness.native").expect("harness"),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        selection_id: "sel-verify-match",
        evaluated_at: "2026-10-22T12:00:00Z",
        substitutions: &[],
        workspace: Some(&ws_manifest),
    };

    selection
        .verify_against(&trust)
        .expect("matching trusted workspace passes verify_against");
}

/// T13 — different sealed trusted workspace fails verify_against (no boundary in requirements)
#[test]
fn different_trusted_workspace_fails_verify_against() {
    let decl = declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    );
    let registry = registry(vec![decl]);

    let ws_manifest1 = sealed_workspace_manifest(
        "ws-one",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );
    let ws_manifest2 = sealed_workspace_manifest(
        "ws-two",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"def".repeat(40),
    );

    // No workspace boundary in requirements — workspace provided as evidence only.
    let requirements = requirements_with_workspace(&["process-exec"], true, None);
    let authority = authority();

    let selection = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-verify-diff".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&ws_manifest1),
        })
        .expect("selection with workspace succeeds");

    let trust = prometheos_lite::workflow::execution_harness::HarnessSelectionTrust {
        harness: registry.get("lite.harness.native").expect("harness"),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        selection_id: "sel-verify-diff",
        evaluated_at: "2026-10-22T12:00:00Z",
        substitutions: &[],
        workspace: Some(&ws_manifest2), // different workspace
    };

    let error = selection
        .verify_against(&trust)
        .expect_err("different trusted workspace fails verify_against");
    assert!(
        error.to_string().contains("HSEL-0008"),
        "expected HSEL-0008 for workspace mismatch, got: {error}"
    );
}

/// T14 — evidence Some / trusted None refuses verify_against (no boundary in requirements)
#[test]
fn evidence_some_trusted_none_refuses_verify_against() {
    let decl = declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    );
    let registry = registry(vec![decl]);

    let ws_manifest = sealed_workspace_manifest(
        "ws-evidence",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );

    // No workspace boundary in requirements — workspace provided as evidence only.
    let requirements = requirements_with_workspace(&["process-exec"], true, None);
    let authority = authority();

    let selection = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-evid-some".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&ws_manifest),
        })
        .expect("selection with workspace succeeds");

    let trust = prometheos_lite::workflow::execution_harness::HarnessSelectionTrust {
        harness: registry.get("lite.harness.native").expect("harness"),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        selection_id: "sel-evid-some",
        evaluated_at: "2026-10-22T12:00:00Z",
        substitutions: &[],
        workspace: None, // trusted has no workspace
    };

    let error = selection
        .verify_against(&trust)
        .expect_err("evidence Some / trusted None refuses");
    assert!(
        error.to_string().contains("HSEL-0008"),
        "expected HSEL-0008 for evidence Some / trusted None, got: {error}"
    );
}

/// T15 — evidence None / trusted Some refuses verify_against
#[test]
fn evidence_none_trusted_some_refuses_verify_against() {
    let decl = declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    );
    let registry = registry(vec![decl]);

    let ws_manifest = sealed_workspace_manifest(
        "ws-trusted",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );

    let requirements = requirements_with_workspace(&["process-exec"], true, None);
    let authority = authority();

    let selection = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-evid-none".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: None, // selection has no workspace
        })
        .expect("selection without workspace succeeds");

    let trust = prometheos_lite::workflow::execution_harness::HarnessSelectionTrust {
        harness: registry.get("lite.harness.native").expect("harness"),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        selection_id: "sel-evid-none",
        evaluated_at: "2026-10-22T12:00:00Z",
        substitutions: &[],
        workspace: Some(&ws_manifest), // trusted has workspace
    };

    let error = selection
        .verify_against(&trust)
        .expect_err("evidence None / trusted Some refuses");
    assert!(
        error.to_string().contains("HSEL-0008"),
        "expected HSEL-0008 for evidence None / trusted Some, got: {error}"
    );
}

/// T16 — invalid trusted manifest refuses verify_against (no boundary in requirements)
#[test]
fn invalid_trusted_manifest_refuses_verify_against() {
    let decl = declaration_with_workspace_adapter(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
    );
    let registry = registry(vec![decl]);

    // Create an invalid manifest (tampered digest)
    let mut ws_manifest = sealed_workspace_manifest(
        "ws-invalid",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );
    ws_manifest.content_digest = Some("0".repeat(64)); // tampered

    // No workspace boundary in requirements — workspace provided as evidence only.
    // Selection will refuse with HSEL-0009 because the #171 workspace policy engine
    // validates the manifest contentDigest integrity. This test verifies that the
    // manifest integrity enforcement happens at selection time.
    let requirements = requirements_with_workspace(&["process-exec"], true, None);
    let authority = authority();

    let selection_err = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-invalid-manifest".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&ws_manifest),
        })
        .expect_err("selection should refuse invalid manifest (integrity check at selection time)");

    assert!(
        selection_err.to_string().contains("HSEL-0009"),
        "expected selection to refuse invalid manifest with HSEL-0009, got: {selection_err}"
    );
}

/// R1 — adapter incompatibility: sealed evidence whose trusted harness does
/// not declare the trusted workspace adapter is refused by `verify_against`.
/// `select()` refuses the same combination, so re-verification must too.
#[test]
fn adapter_incompatible_trusted_harness_refuses_verify_against() {
    // Registry harness declares NO workspace adapter.
    let registry = registry(vec![declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
    )]);

    let requirements = requirements_with_workspace(&["process-exec"], true, None);
    let authority = authority();

    let selection = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-r1-adapter".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: None,
        })
        .expect("adapter-free selection succeeds without a workspace");

    // Sealed evidence that also binds a read-only workspace manifest.
    let ws_manifest = sealed_workspace_manifest(
        "ws-r1",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );
    let mut resealed = selection.clone();
    resealed.content_digest = None;
    resealed.workspace = Some(workspace_evidence_for(&ws_manifest));
    let resealed = resealed.try_sealed().expect("reseal succeeds");

    let trust = prometheos_lite::workflow::execution_harness::HarnessSelectionTrust {
        harness: registry.get("lite.harness.native").expect("harness"),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        selection_id: "sel-r1-adapter",
        evaluated_at: "2026-10-22T12:00:00Z",
        substitutions: &[],
        workspace: Some(&ws_manifest),
    };

    let error = resealed
        .verify_against(&trust)
        .expect_err("adapter-incompatible trusted harness refused");
    assert!(
        error.to_string().contains("HSEL-0008"),
        "expected HSEL-0008 for adapter incompatibility, got: {error}"
    );
    assert!(
        error.to_string().contains("adapter"),
        "expected the adapter gate to fire, got: {error}"
    );
}

/// R2 — workspace mode incompatibility: sealed evidence involving a read-only
/// workspace with requirements demanding a writable workspace is refused by
/// `verify_against`.
#[test]
fn workspace_mode_incompatible_trusted_refuses_verify_against() {
    let decl = declaration_with_workspace_adapter_and_boundary(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
        Boundaries {
            workspace: Some(Boundary::Write),
            filesystem: None,
            git: None,
            process: None,
            network: None,
            secret: None,
        },
    );
    let registry = registry(vec![decl]);

    let writable_manifest = sealed_workspace_manifest(
        "ws-r2-writable",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::Writable,
        &"abc".repeat(40),
    );
    let read_only_manifest = sealed_workspace_manifest(
        "ws-r2-readonly",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );

    let requirements = requirements_with_workspace(&["process-exec"], true, Some(Boundary::Write));
    let authority = authority();

    let selection = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-r2-mode".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&writable_manifest),
        })
        .expect("writable workspace selection succeeds");

    // Reseal evidence to claim the read-only workspace instance.
    let mut resealed = selection.clone();
    resealed.content_digest = None;
    resealed.workspace = Some(workspace_evidence_for(&read_only_manifest));
    let resealed = resealed.try_sealed().expect("reseal succeeds");

    let trust = prometheos_lite::workflow::execution_harness::HarnessSelectionTrust {
        harness: registry.get("lite.harness.native").expect("harness"),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        selection_id: "sel-r2-mode",
        evaluated_at: "2026-10-22T12:00:00Z",
        substitutions: &[],
        workspace: Some(&read_only_manifest),
    };

    let error = resealed
        .verify_against(&trust)
        .expect_err("mode-incompatible trusted workspace refused");
    assert!(
        error.to_string().contains("HSEL-0008"),
        "expected HSEL-0008 for mode incompatibility, got: {error}"
    );
    assert!(
        error.to_string().contains("mode") || error.to_string().contains("boundary"),
        "expected the mode gate to fire, got: {error}"
    );
}

/// R3 — missing required workspace: evidence and trusted inputs that both omit
/// a workspace are refused when the requirements demand one.
#[test]
fn missing_required_workspace_refuses_verify_against() {
    let decl = declaration_with_workspace_adapter_and_boundary(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
        "existing-read-only",
        "lite.workspace.adapter.v1",
        Boundaries {
            workspace: Some(Boundary::Read),
            filesystem: None,
            git: None,
            process: None,
            network: None,
            secret: None,
        },
    );
    let registry = registry(vec![decl]);

    let ws_manifest = sealed_workspace_manifest(
        "ws-r3",
        AdapterKind::ExistingReadOnly,
        WorkspaceMode::ReadOnly,
        &"abc".repeat(40),
    );

    let requirements = requirements_with_workspace(&["process-exec"], true, Some(Boundary::Read));
    let authority = authority();

    let selection = registry
        .select(&HarnessSelectionRequest {
            selection_id: "sel-r3-missing".to_string(),
            requirements: &requirements,
            authority_declared: &authority,
            authority_effective: &authority,
            evaluated_at: "2026-10-22T12:00:00Z".to_string(),
            substitutions: Vec::new(),
            pinned_harness_id: None,
            workspace: Some(&ws_manifest),
        })
        .expect("read workspace selection succeeds");

    // Reseal evidence to drop the workspace.
    let mut resealed = selection.clone();
    resealed.content_digest = None;
    resealed.workspace = None;
    let resealed = resealed.try_sealed().expect("reseal succeeds");

    let trust = prometheos_lite::workflow::execution_harness::HarnessSelectionTrust {
        harness: registry.get("lite.harness.native").expect("harness"),
        requirements: &requirements,
        authority_declared: &authority,
        authority_effective: &authority,
        selection_id: "sel-r3-missing",
        evaluated_at: "2026-10-22T12:00:00Z",
        substitutions: &[],
        workspace: None,
    };

    let error = resealed
        .verify_against(&trust)
        .expect_err("missing required workspace refused by verify_against");
    assert!(
        error.to_string().contains("HSEL-0008"),
        "expected HSEL-0008 for missing required workspace, got: {error}"
    );
    assert!(
        error.to_string().contains("workspace"),
        "expected the missing-workspace gate to fire, got: {error}"
    );
}
