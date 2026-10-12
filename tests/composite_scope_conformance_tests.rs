//! E4/X03 (#160) Slice A — composite-scope conformance.
//!
//! Dedicated two-level fixture (`tests/fixtures/composite/nested-two-level.json`),
//! not borrowed from projection-only fixtures. Proves recursive composite
//! validation, boundary-port compatibility in both directions, and the
//! undeclared-export / authority-escalation / context-disclosure gates at both
//! nesting levels — all through the audit only (no execution graph).

use prometheos_lite::workflow::soma::canonical::try_canonical_digest;
use prometheos_lite::workflow::soma::validate_artifact_text;
use serde_json::{Value, json};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/composite/nested-two-level.json"
);

fn load() -> Value {
    let text = std::fs::read_to_string(FIXTURE).expect("read fixture");
    serde_json::from_str(&text).expect("fixture parses")
}

fn audit(value: &Value) -> Vec<String> {
    let text = serde_json::to_string(value).expect("serialize");
    match validate_artifact_text("WorkflowDefinition", &text) {
        Ok(diags) => diags.into_iter().map(|d| d.code).collect(),
        Err(reason) => panic!("fixture must parse/validate structurally: {reason}"),
    }
}

/// Index of the nested composite named `id` inside `body`, if present.
fn composite_index(body: &Value, id: &str) -> usize {
    body.as_array()
        .expect("array")
        .iter()
        .position(|c| c.get("id").and_then(Value::as_str) == Some(id))
        .unwrap_or_else(|| panic!("composite {id} present"))
}

#[test]
fn two_level_fixture_audits_clean() {
    let codes = audit(&load());
    assert!(codes.is_empty(), "expected clean audit, got {codes:?}");
}

#[test]
fn canonical_serialization_and_digest_are_stable() {
    let v = load();
    let audit_a = try_canonical_digest(&v).expect("digest a");
    let audit_b = try_canonical_digest(&v).expect("digest b");
    assert_eq!(audit_a, audit_b, "canonical digest must be stable");

    // Re-serialize → re-parse → digest is unchanged (byte/determinism pinned).
    let text = serde_json::to_string(&v).expect("serialize");
    let reparsed: Value = serde_json::from_str(&text).expect("reparse");
    assert_eq!(
        try_canonical_digest(&reparsed).expect("digest c"),
        audit_a,
        "reparse must not change the canonical digest"
    );
}

#[test]
fn validation_runs_without_constructing_an_execution_graph() {
    // The audit entry point is source-only: it never builds an execution graph
    // (which would refuse composites). A clean result here proves composites
    // validate independently of graph construction.
    assert!(audit(&load()).is_empty());
}

#[test]
fn parent_to_child_port_type_mismatch_fails() {
    let mut v = load();
    let body = &mut v["body"];
    let flow = composite_index(body, "flow");
    let inner = composite_index(&body[flow]["body"], "inner");
    // Parent (flow) declares `staged:Order`; child (inner) declares `staged:Shipment`.
    body[flow]["body"][inner]["inputPorts"][0]["type"] = json!("Shipment");
    assert!(
        audit(&v).iter().any(|c| c == "SOMA-CMP-0005"),
        "expected SOMA-CMP-0005 (parent→child port mismatch)"
    );
}

#[test]
fn child_to_parent_port_type_mismatch_fails() {
    let mut v = load();
    let body = &mut v["body"];
    let flow = composite_index(body, "flow");
    let inner = composite_index(&body[flow]["body"], "inner");
    // Child (inner) exports `ready:Shipment`; parent (flow) projects `ready:Shipment`
    // originally — flip the child's export type.
    body[flow]["body"][inner]["outputPorts"][0]["type"] = json!("Order");
    assert!(
        audit(&v).iter().any(|c| c == "SOMA-CMP-0005"),
        "expected SOMA-CMP-0005 (child→parent port mismatch)"
    );
}

#[test]
fn undeclared_effect_export_fails_at_inner_level() {
    let mut v = load();
    let body = &mut v["body"];
    let flow = composite_index(body, "flow");
    let inner = composite_index(&body[flow]["body"], "inner");
    // Key presence of `effectExports` (empty) arms the crossing gate.
    body[flow]["body"][inner]["effectExports"] = json!([]);
    body[flow]["body"][inner]["body"][0]["effects"] = json!([{ "name": "write" }]);
    assert!(
        audit(&v).iter().any(|c| c == "SOMA-EXP-0007"),
        "expected SOMA-EXP-0007 (undeclared effect crossing)"
    );
}

#[test]
fn authority_escalation_fails_at_inner_level() {
    let mut v = load();
    let body = &mut v["body"];
    let flow = composite_index(body, "flow");
    let inner = composite_index(&body[flow]["body"], "inner");
    // `leaf` grants a capability outside its scope's `authorityImports`.
    body[flow]["body"][inner]["body"][0]["authority"] = json!(["exfiltrate"]);
    let codes = audit(&v);
    assert!(
        codes.iter().any(|c| c == "SOMA-AUTH-0002" || c == "SOMA-AUTH-0001"),
        "expected an authority escalation diagnostic, got {codes:?}"
    );
}

#[test]
fn undeclared_context_disclosure_fails_at_inner_level() {
    let mut v = load();
    let body = &mut v["body"];
    let flow = composite_index(body, "flow");
    let inner = composite_index(&body[flow]["body"], "inner");
    // Key presence of `context.discloses` (empty) arms the context gate.
    body[flow]["body"][inner]["context"] = json!({ "discloses": [] });
    body[flow]["body"][inner]["body"][0]["context"] = json!(["private-note"]);
    assert!(
        audit(&v).iter().any(|c| c == "SOMA-EXP-0007"),
        "expected SOMA-EXP-0007 (undeclared context crossing)"
    );
}
