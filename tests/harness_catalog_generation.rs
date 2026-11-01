//! Generated capability registry and CI drift protection (issue #154,
//! slice C).
//!
//! Slice B bound harness selection to SPEC 007 negotiation. Slice C adds
//! deterministic generation of the capability catalog from implementation-owned,
//! versioned harness declarations, plus fail-closed validation and drift
//! detection so that committed generated artifacts cannot silently diverge
//! from their authoritative source declarations.

use anyhow::{Result, bail};
use prometheos_lite::workflow::execution_harness::{
    ExecutionHarness, HarnessDeclarationV1, HarnessFamily, HarnessRegistry, HarnessStep,
};
use prometheos_lite::workflow::soma::capability::{Identity, RuntimeCapabilitySet};
const HARNESS_SCHEMA_VERSION: &str = "1.0.0";

fn capability_set(
    harness_id: &str,
    name: &str,
    revision: &str,
    classes: &[&str],
) -> RuntimeCapabilitySet {
    RuntimeCapabilitySet {
        id: format!("{harness_id}.caps"),
        schema_version: "1.2.0".to_string(),
        version: "1.2.0".to_string(),
        identity: Identity {
            name: name.to_string(),
            revision: revision.to_string(),
        },
        declared_at: "2026-10-22T00:00:00Z".to_string(),
        spec_versions: vec!["1.2.0".to_string()],
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
        declared_capabilities: vec!["harness.probe".to_string()],
        claimed_capabilities: vec!["harness.probe".to_string()],
        capabilities: capability_set(harness_id, name, revision, classes),
        content_digest: None,
    }
    .sealed()
}

struct ProbeHarness {
    declaration: HarnessDeclarationV1,
}

#[async_trait::async_trait]
impl ExecutionHarness for ProbeHarness {
    fn declaration(&self) -> &HarnessDeclarationV1 {
        &self.declaration
    }

    async fn run(
        &self,
        _step: HarnessStep,
        _dispatch: &mut prometheos_lite::workflow::execution_harness::HarnessDispatch<'_>,
    ) -> Result<prometheos_lite::workflow::execution_harness::HarnessOutcome> {
        bail!("probe not executable")
    }
}

fn registry_with_declarations(decls: Vec<HarnessDeclarationV1>) -> HarnessRegistry {
    let mut registry = HarnessRegistry::new();
    for d in decls {
        registry
            .register(Box::new(ProbeHarness { declaration: d }))
            .expect("registration succeeds");
    }
    registry
}

/// T1 — catalog generation is deterministic regardless of registration order.
#[test]
fn catalog_generation_is_deterministic() {
    let decls = vec![
        declaration(
            "lite.harness.beta",
            "beta",
            "v1",
            HarnessFamily::Native,
            &["process-exec"],
        ),
        declaration(
            "lite.harness.alpha",
            "alpha",
            "v1",
            HarnessFamily::Native,
            &["process-exec"],
        ),
    ];

    let mut forward = HarnessRegistry::new();
    for d in decls.clone() {
        forward
            .register(Box::new(ProbeHarness { declaration: d }))
            .expect("registers");
    }

    let mut reverse = HarnessRegistry::new();
    for d in decls.into_iter().rev() {
        reverse
            .register(Box::new(ProbeHarness { declaration: d }))
            .expect("registers");
    }

    let a = forward.catalog();
    let b = reverse.catalog();
    assert_eq!(a.to_json().unwrap(), b.to_json().unwrap());
    assert_eq!(a.content_digest, b.content_digest);
}

/// T2 — catalog validation rejects malformed/inconsistent catalogs.
#[test]
fn catalog_validation_rejects_inconsistent() {
    let decls = vec![
        declaration(
            "lite.harness.alpha",
            "alpha",
            "v1",
            HarnessFamily::Native,
            &["process-exec"],
        ),
        declaration(
            "lite.harness.beta",
            "beta",
            "v1",
            HarnessFamily::Native,
            &["process-exec"],
        ),
    ];
    let registry = registry_with_declarations(decls);
    let catalog = registry.catalog();

    // Valid catalog passes
    let findings = catalog.validate();
    assert!(
        findings.is_empty(),
        "valid catalog should have no findings: {findings:?}"
    );

    // Tampered catalog: duplicate harness_id
    let mut tampered = catalog.clone();
    if let Some(entry) = tampered.entries.first() {
        let mut dup = entry.clone();
        dup.harness_id = entry.harness_id.clone();
        tampered.entries.push(dup);
    }
    let findings = tampered.validate();
    assert!(
        !findings.is_empty(),
        "duplicate harnessId should be detected"
    );
    assert!(findings.iter().any(|f| f.contains("duplicate")));

    // Tampered catalog: unsorted entries
    let mut unsorted = catalog.clone();
    if unsorted.entries.len() >= 2 {
        unsorted.entries.swap(0, 1);
        let findings = unsorted.validate();
        assert!(!findings.is_empty(), "unsorted entries should be detected");
        assert!(
            findings
                .iter()
                .any(|f| f.contains("not sorted") || f.contains("sorted"))
        );
    }
}

/// T3 — declaration validation rejects malformed declarations.
#[test]
fn declaration_validation_rejects_malformed() {
    // Empty harness_id
    let mut bad = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        &["process-exec"],
    );
    bad.harness_id = "".to_string();
    let findings = bad.validate();
    assert!(!findings.is_empty());
    assert!(findings.iter().any(|f| f.contains("empty harnessId")));

    // Over-claimed capability
    let mut bad = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        &["process-exec"],
    );
    bad.claimed_capabilities = vec!["undeclared.cap".to_string()];
    let findings = bad.validate();
    assert!(!findings.is_empty());
    assert!(findings.iter().any(|f| f.contains("claimed capability")));

    // Empty identity
    let mut bad = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        &["process-exec"],
    );
    bad.identity.name = "".to_string();
    let findings = bad.validate();
    assert!(!findings.is_empty());
    assert!(findings.iter().any(|f| f.contains("empty identity")));

    // Duplicate declared capabilities
    let mut bad = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        &["process-exec"],
    );
    bad.declared_capabilities = vec!["cap.a".to_string(), "cap.a".to_string()];
    let findings = bad.validate();
    assert!(!findings.is_empty());
    assert!(findings.iter().any(|f| f.contains("duplicate declared")));

    // No declared capabilities
    let mut bad = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        &["process-exec"],
    );
    bad.declared_capabilities = vec![];
    let findings = bad.validate();
    assert!(!findings.is_empty());
    assert!(
        findings
            .iter()
            .any(|f| f.contains("no declared capabilities"))
    );

    // Stale spec versions
    let mut bad = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        &["process-exec"],
    );
    bad.capabilities.spec_versions = vec![];
    let findings = bad.validate();
    assert!(!findings.is_empty());
    assert!(
        findings
            .iter()
            .any(|f| f.contains("no supported spec versions"))
    );

    // Conflicting identity
    let mut bad = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        &["process-exec"],
    );
    bad.capabilities.identity.name = "different".to_string();
    let findings = bad.validate();
    assert!(!findings.is_empty());
    assert!(findings.iter().any(|f| f.contains("diverges")));

    // Valid declaration has no findings
    let good = declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        &["process-exec"],
    );
    let findings = good.validate();
    assert!(
        findings.is_empty(),
        "valid declaration should have no findings: {findings:?}"
    );
}

/// T4 — drift detection: changing a declaration changes the catalog digest.
#[test]
fn catalog_drift_detected_on_declaration_change() {
    let decls_v1 = vec![declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        &["process-exec"],
    )];
    let registry_v1 = registry_with_declarations(decls_v1);
    let catalog_v1 = registry_v1.catalog();
    let json_v1 = catalog_v1.to_json().unwrap();

    // Same declaration produces identical output
    let decls_v1b = vec![declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        &["process-exec"],
    )];
    let registry_v1b = registry_with_declarations(decls_v1b);
    let catalog_v1b = registry_v1b.catalog();
    assert_eq!(json_v1, catalog_v1b.to_json().unwrap());

    // Changed revision produces different output
    let decls_v2 = vec![declaration(
        "lite.harness.native",
        "lite-native",
        "v2",
        HarnessFamily::Native,
        &["process-exec"],
    )];
    let registry_v2 = registry_with_declarations(decls_v2);
    let catalog_v2 = registry_v2.catalog();
    assert_ne!(json_v1, catalog_v2.to_json().unwrap());
    assert_ne!(catalog_v1.content_digest, catalog_v2.content_digest);
}

/// T5 — catalog entries preserve capability/authority separation.
#[test]
fn catalog_preserves_capability_authority_separation() {
    let decls = vec![declaration(
        "lite.harness.native",
        "lite-native",
        "v1",
        HarnessFamily::Native,
        &["process-exec"],
    )];
    let registry = registry_with_declarations(decls);
    let catalog = registry.catalog();
    let json = catalog.to_json().unwrap();

    // The catalog must contain declared capabilities
    assert!(json.contains("declaredCapabilities"));
    assert!(json.contains("claimedCapabilities"));
    // The catalog must NOT contain any authority/grant fields
    assert!(!json.contains("grantedAuthority"));
    assert!(!json.contains("authorityGrant"));
    assert!(!json.contains("privilegeLevel"));
    // Unclaimed capabilities field present (limitations)
    assert!(json.contains("unclaimedCapabilities"));
    // Over-claimed flag present
    assert!(json.contains("overClaimed"));
}

/// T6 — catalog is sorted by harness_id for deterministic output.
#[test]
fn catalog_entries_sorted_by_harness_id() {
    let decls = vec![
        declaration(
            "lite.harness.zeta",
            "zeta",
            "v1",
            HarnessFamily::Native,
            &["process-exec"],
        ),
        declaration(
            "lite.harness.alpha",
            "alpha",
            "v1",
            HarnessFamily::Native,
            &["process-exec"],
        ),
        declaration(
            "lite.harness.beta",
            "beta",
            "v1",
            HarnessFamily::Native,
            &["process-exec"],
        ),
    ];
    let registry = registry_with_declarations(decls);
    let catalog = registry.catalog();
    let ids: Vec<&str> = catalog
        .entries
        .iter()
        .map(|e| e.harness_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec![
            "lite.harness.alpha",
            "lite.harness.beta",
            "lite.harness.zeta"
        ]
    );
}

/// T8 - drift detection: committed catalog matches generated output.
/// This test reads a committed catalog artifact and compares it byte-for-byte
/// against a freshly generated catalog from the same declarations.
#[test]
fn catalog_drift_detection() {
    let decls = vec![
        declaration(
            "lite.harness.alpha",
            "alpha",
            "v1",
            HarnessFamily::Native,
            &["process-exec"],
        ),
        declaration(
            "lite.harness.beta",
            "beta",
            "v2",
            HarnessFamily::Subprocess,
            &["process-exec"],
        ),
    ];
    let registry = registry_with_declarations(decls.clone());
    let catalog = registry.catalog();
    let generated = catalog.to_json().unwrap();

    // Read committed artifact
    let committed = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/harness_catalog_canonical.json"
    ))
    .expect("committed catalog fixture exists");

    assert_eq!(
        generated,
        committed,
        "generated catalog does not match committed fixture - possible drift"
    );

    // Verify provenance against trusted declarations
    let provenance_findings = catalog.verify_against_declarations(&decls);
    assert!(
        provenance_findings.is_empty(),
        "catalog must match trusted declarations: {provenance_findings:?}"
    );
}
/// T9 — provenance verification: catalog validates against trusted declarations.
#[test]
fn catalog_provenance_verification() {
    let decls = vec![
        declaration(
            "lite.harness.alpha",
            "alpha",
            "v1",
            HarnessFamily::Native,
            &["process-exec"],
        ),
        declaration(
            "lite.harness.beta",
            "beta",
            "v2",
            HarnessFamily::Subprocess,
            &["process-exec"],
        ),
    ];
    let registry = registry_with_declarations(decls.clone());
    let catalog = registry.catalog();

    // Catalog validates structurally
    let findings = catalog.validate();
    assert!(
        findings.is_empty(),
        "valid catalog should have no findings: {findings:?}"
    );

    // Catalog matches its trusted declarations
    let provenance_findings = catalog.verify_against_declarations(&decls);
    assert!(
        provenance_findings.is_empty(),
        "catalog should match trusted declarations: {provenance_findings:?}"
    );

    // Tampered catalog: wrong declaration digest
    let mut tampered = catalog.clone();
    if let Some(entry) = tampered.entries.first_mut() {
        entry.declaration_digest = "0".repeat(64);
    }
    let findings = tampered.verify_against_declarations(&decls);
    assert!(!findings.is_empty(), "tampered digest should be detected");
    assert!(
        findings
            .iter()
            .any(|f| f.contains("declarationDigest mismatch"))
    );
}

/// T10 - empty registry produces empty catalog.
#[test]
fn empty_registry_produces_empty_catalog() {
    let registry = HarnessRegistry::new();
    let catalog = registry.catalog();
    let json = catalog.to_json().unwrap();
    assert!(
        catalog.entries.is_empty(),
        "empty registry must produce empty catalog"
    );
    assert!(
        json.contains("\"entries\": []"),
        "empty catalog JSON must have empty entries"
    );
}
