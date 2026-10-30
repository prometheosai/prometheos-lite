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
    HarnessOutcome, HarnessRegistry, HarnessSelectionRequest, HarnessSelectionV1, HarnessStep,
};
use prometheos_lite::workflow::soma::capability::{
    DecisionStatus, Identity, RuntimeCapabilitySet, SUPPORTED_CAPABILITY_SCHEMA_VERSION,
    Substitution, WorkRequirements,
};
use prometheos_lite::workflow::soma::contracts::AuthorityProfile;

const SELECTION_SCHEMA_VERSION: &str = "1.0.0";

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
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

/// T1 — the compatible candidate is selected and the evidence binds the
/// harness identity, its revision, its declaration digest and the decision.
#[test]
fn compatible_harness_selection_records_revision_and_decision() {
    let declaration = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
    );
    let expected_declaration_digest = declaration.compute_digest();
    let registry = registry(vec![declaration]);
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
    let declaration = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        "harness.native.probe",
        &["process-exec"],
    );
    let registry = registry(vec![declaration]);
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
