//! Emitted-diagnostics conformance (plan T4, contract req3).
//!
//! Two invariants:
//!
//! 1. **Categories are catalogue-pinned.** Every code the vendored
//!    `diagnostics.json` defines reports exactly its published category —
//!    never a family-generic stand-in derived from substring matching.
//! 2. **Sources are document pointers.** `source.path` is an RFC 6901
//!    JSON pointer into the offending document (`""` = whole document);
//!    `source.subject` carries a stable element identifier (operation id,
//!    workflow id, workflowDigest).

use prometheos_lite::workflow::governance_compiler::{compile_workflow_text, verify_reviewed_plan};
use prometheos_lite::workflow::soma::Diagnostic;

const VENDORED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendored/soma/v1.1");

fn fixture(rel: &str) -> String {
    std::fs::read_to_string(format!("{VENDORED}/{rel}")).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// Every `{code, category}` pair from the vendored v1.1 catalogue — the
/// normative per-code category table (mirrors the oracle's exact-match
/// `category_for` + taxonomy guard).
const CATALOGUE: &[(&str, &str)] = &[
    ("SOMA-AUTH-0001", "authority_expansion"),
    ("SOMA-AUTH-0002", "authority_exceeded_composite"),
    ("SOMA-AUTH-0003", "scope_violation"),
    ("SOMA-AUTH-0004", "provider_boundary_violation"),
    ("SOMA-AUTH-0005", "capability_violation"),
    ("SOMA-AUTH-0006", "secret_violation"),
    ("SOMA-AUTH-0007", "unreviewed_effect"),
    ("SOMA-AUTH-0008", "irreversible_without_recovery"),
    ("SOMA-AUTH-0009", "budget_exceeded"),
    ("SOMA-AUTH-0010", "abstention_unhandled"),
    ("SOMA-GOV-0001", "constraint_violation"),
    ("SOMA-GOV-0002", "constraint_inconsistent"),
    ("SOMA-GOV-0003", "constraint_undecidable"),
    ("SOMA-CMP-0001", "unsupported_version"),
    ("SOMA-CMP-0002", "invalid_reference"),
    ("SOMA-CMP-0003", "schema_violation"),
    ("SOMA-CMP-0004", "canonicalization_mismatch"),
    ("SOMA-CMP-0005", "type_mismatch"),
    ("SOMA-CMP-0006", "unsupported_type"),
    ("SOMA-CMP-0007", "duplicate_key"),
    ("SOMA-EXP-0001", "cyclic_composition"),
    ("SOMA-EXP-0002", "cyclic_edges"),
    ("SOMA-EXP-0003", "nondeterministic_expansion"),
    ("SOMA-EXP-0004", "unreachable_required"),
    ("SOMA-EXP-0005", "unsatisfied_required_input"),
    ("SOMA-EXP-0006", "optional_to_required"),
    ("SOMA-EXP-0007", "leakage"),
    ("SOMA-OUT-0001", "outcome_coercion"),
    ("SOMA-OUT-0002", "outcome_not_accepted"),
    ("SOMA-RES-0001", "resume_stale"),
    ("SOMA-RES-0002", "resume_expired"),
    ("SOMA-RES-0003", "resume_tampered"),
    ("SOMA-RES-0004", "resume_unsupported"),
    ("SOMA-EVT-0001", "event_unsupported_version"),
    ("SOMA-EVT-0002", "event_broken_causality"),
    ("SOMA-EVT-0003", "event_authority_expansion"),
    ("SOMA-EVT-0004", "event_missing_evidence"),
    ("SOMA-EVT-0005", "event_replay_conflict"),
    ("SOMA-PROF-0001", "profile_authority_expansion"),
    ("SOMA-PROF-0002", "profile_budget_exceeded"),
    ("SOMA-ADAPT-0001", "adapter_incompatible"),
];

/// Every `SOMA-*` literal emitted anywhere in `src/`. Update this list
/// (and `CATALOGUE` above, from the vendored catalogue) whenever a new
/// code is introduced — the guard test fails until both are extended.
const EMITTED: &[&str] = &[
    "SOMA-ADAPT-0001",
    "SOMA-AUTH-0001",
    "SOMA-AUTH-0002",
    "SOMA-AUTH-0003",
    "SOMA-AUTH-0004",
    "SOMA-AUTH-0005",
    "SOMA-AUTH-0006",
    "SOMA-AUTH-0007",
    "SOMA-AUTH-0008",
    "SOMA-AUTH-0009",
    "SOMA-AUTH-0010",
    "SOMA-CMP-0001",
    "SOMA-CMP-0002",
    "SOMA-CMP-0003",
    "SOMA-CMP-0004",
    "SOMA-CMP-0005",
    "SOMA-CMP-0006",
    "SOMA-CMP-0007",
    "SOMA-EVT-0001",
    "SOMA-EVT-0002",
    "SOMA-EVT-0003",
    "SOMA-EVT-0004",
    "SOMA-EVT-0005",
    "SOMA-EXP-0001",
    "SOMA-EXP-0002",
    "SOMA-EXP-0003",
    "SOMA-EXP-0004",
    "SOMA-EXP-0005",
    "SOMA-EXP-0006",
    "SOMA-EXP-0007",
    "SOMA-GOV-0001",
    "SOMA-GOV-0002",
    "SOMA-GOV-0003",
    "SOMA-OUT-0001",
    "SOMA-OUT-0002",
    "SOMA-PROF-0001",
    "SOMA-PROF-0002",
    "SOMA-RES-0001",
    "SOMA-RES-0002",
    "SOMA-RES-0003",
    "SOMA-RES-0004",
];

/// Compile a valid fixture and serialize the sealed plan back to text
/// (the shape `verify_reviewed_plan` consumes).
fn compiled_plan_text(rel: &str) -> String {
    let plan = compile_workflow_text(&fixture(rel))
        .unwrap_or_else(|d| panic!("{rel} must compile: {d:?}"));
    serde_json::to_string(&plan).expect("plan serializes")
}

fn workflow_digest_of(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .expect("plan text parses")
        .get("workflowDigest")
        .and_then(serde_json::Value::as_str)
        .expect("workflowDigest member")
        .to_string()
}

/// Flip the first hex character of `canonicalization.sha256` (stays a
/// lowercase-hex string, so only the seal check can fail).
fn tamper_seal(text: &str) -> String {
    let mut value: serde_json::Value =
        serde_json::from_str(text).expect("plan text parses");
    let sha = value["canonicalization"]["sha256"]
        .as_str()
        .expect("canonicalization.sha256");
    let flipped: String = sha
        .chars()
        .enumerate()
        .map(|(i, c)| if i == 0 { if c == '0' { '1' } else { '0' } } else { c })
        .collect();
    value["canonicalization"]["sha256"] = serde_json::Value::String(flipped);
    serde_json::to_string(&value).expect("tampered plan serializes")
}

#[test]
fn every_catalogue_code_reports_its_pinned_category() {
    for &(code, pinned) in CATALOGUE {
        let emitted = Diagnostic::new(code, "category guard").category;
        assert_eq!(emitted, pinned, "category drift for {code}");
    }
}

#[test]
fn every_emitted_code_is_catalogue_registered_and_never_general() {
    let registered: std::collections::HashSet<&str> =
        CATALOGUE.iter().map(|&(code, _)| code).collect();
    assert_eq!(
        registered.len(),
        CATALOGUE.len(),
        "catalogue rows are unique"
    );
    for &code in EMITTED {
        assert!(
            registered.contains(code),
            "{code} is emitted in src/ but absent from the vendored catalogue"
        );
        let category = Diagnostic::new(code, "category guard").category;
        assert_ne!(category, "general", "{code} fell back to the general bucket");
    }
}

#[test]
fn undeclared_grant_diagnostic_points_into_the_body() {
    let text = fixture("fixtures/invalid/wf-auth-0001.json");
    let diagnostics = compile_workflow_text(&text)
        .expect_err("undeclared grant must refuse compilation");
    let diagnostic = diagnostics
        .iter()
        .find(|d| d.code == "SOMA-AUTH-0001")
        .expect("AUTH-0001 emitted");
    let source = diagnostic.source.as_ref().expect("source attached");
    assert_eq!(
        source.path, "/body/0",
        "RFC 6901 pointer to the offending body unit"
    );
    assert_eq!(
        source.subject.as_deref(),
        Some("op1"),
        "subject carries the stable operation id"
    );
}

#[test]
fn binding_refusal_points_at_the_plan_seal_member() {
    let text = compiled_plan_text("fixtures/valid/wf-valid-base.json");
    let workflow_digest = workflow_digest_of(&text);
    let diagnostics = verify_reviewed_plan(&text, &"0".repeat(64))
        .expect_err("identity mismatch must refuse");
    assert_eq!(diagnostics.len(), 1, "one refusal");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.code, "SOMA-CMP-0004");
    let source = diagnostic.source.as_ref().expect("source attached");
    assert_eq!(source.path, "/canonicalization/sha256");
    assert_eq!(
        source.subject.as_deref(),
        Some(workflow_digest.as_str()),
        "subject names the workflow the seal binds"
    );
}

#[test]
fn seal_tamper_refusal_points_at_the_plan_seal_member() {
    let text = compiled_plan_text("fixtures/valid/wf-valid-base.json");
    let workflow_digest = workflow_digest_of(&text);
    let tampered = tamper_seal(&text);
    assert_ne!(tampered, text, "seal member actually changed");
    let diagnostics = verify_reviewed_plan(&tampered, &"0".repeat(64))
        .expect_err("tampered seal must refuse");
    let diagnostic = diagnostics
        .iter()
        .find(|d| d.code == "SOMA-CMP-0004")
        .expect("CMP-0004 emitted");
    let source = diagnostic.source.as_ref().expect("source attached");
    assert_eq!(source.path, "/canonicalization/sha256");
    assert_eq!(
        source.subject.as_deref(),
        Some(workflow_digest.as_str()),
        "subject names the workflow the seal binds"
    );
}

#[test]
fn plan_version_refusal_points_at_the_plan_version_member() {
    let text = compiled_plan_text("fixtures/valid/wf-valid-base.json");
    let workflow_digest = workflow_digest_of(&text);
    let tampered = text.replace(
        "\"planVersion\":\"1.0.0\"",
        "\"planVersion\":\"2.0.0\"",
    );
    assert_ne!(tampered, text, "planVersion member replaced");
    let diagnostics = verify_reviewed_plan(&tampered, &"0".repeat(64))
        .expect_err("unsupported plan version must refuse");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.code, "SOMA-CMP-0001");
    let source = diagnostic.source.as_ref().expect("source attached");
    assert_eq!(source.path, "/planVersion");
    assert_eq!(
        source.subject.as_deref(),
        Some(workflow_digest.as_str()),
        "subject names the workflow the version binds"
    );
}
